#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

#include "lexer.h"
#include "token.h"
#include "parser.h"
#include "ast.h"
#include "codegen.h"
#include "typecheck.h"

typedef struct {
    char name[64];
    int param_count;
    char params[8][64];
    char body[1024];
    int function_like;
} MacroDef;
static MacroDef g_macros[128];
static int g_nmacro = 0;

static MacroDef *find_macro(const char *name) {
    for (int i = 0; i < g_nmacro; i++) {
        if (strcmp(g_macros[i].name, name) == 0) return &g_macros[i];
    }
    return NULL;
}
static int macro_defined(const char *name) {
    return find_macro(name) != NULL;
}

/* simple #define preprocessor */
static char *expand_includes(const char *src, const char *base_dir, int depth);

/* reads a file into string */
static char *read_text_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc(sz + 1);
    if (!buf) { fclose(f); return NULL; }
    size_t n = fread(buf, 1, sz, f);
    buf[n] = '\0';
    fclose(f);
    return buf;
}

static char *expand_includes(const char *src, const char *base_dir, int depth) {
    if (!base_dir) base_dir = ".";
    if (!src || depth > 16) return src ? strdup(src) : NULL;
    size_t outcap = strlen(src) * 2 + 1024;
    char *out = malloc(outcap);
    size_t oi = 0;
    const char *p = src;
    while (*p) {
        if (*p == '#') {
            const char *q = p;
            /* find line end */
            const char *line_end = strchr(q, '\n');
            if (!line_end) line_end = q + strlen(q);
            /* trim left */
            while (q < line_end && isspace((unsigned char)*q)) q++;
            if (strncmp(q, "#include", 8) == 0) {
                q += 8;
                while (q < line_end && isspace((unsigned char)*q)) q++;
                char delim = 0;
                if (*q == '"' || *q == '<') { delim = (*q == '"') ? '"' : '>'; q++; }
                char path[512]; size_t pi = 0;
                while (*q && *q != '"' && *q != '>' && *q != '\n' && pi < 511) path[pi++] = *q++;
                path[pi] = 0;
                char c0[1024], c1[1024], c2[1024], c3[1024];
                const char *candidates[5];
                snprintf(c0, sizeof(c0), "%s/%s", base_dir ? base_dir : ".", path);
                snprintf(c1, sizeof(c1), "%s", path);
                snprintf(c2, sizeof(c2), "/usr/include/%s", path);
                snprintf(c3, sizeof(c3), "/usr/include/x86_64-linux-gnu/%s", path);
                candidates[0]=c0; candidates[1]=c1; candidates[2]=c2; candidates[3]=c3; candidates[4]=NULL;
                char found_path[1024]; found_path[0]=0;
                char *content = NULL;
                for (int i = 0; i < 4 && !content; i++) {
                    content = read_text_file(candidates[i]);
                    if (content) strncpy(found_path, candidates[i], sizeof(found_path)-1);
                }
                if (content) {
                    char inc_base[1024];
                    strncpy(inc_base, found_path, sizeof(inc_base)-1);
                    inc_base[sizeof(inc_base)-1]=0;
                    char *slash = strrchr(inc_base, '/');
                    if (slash) *slash=0; else strcpy(inc_base, ".");
                    char *expanded = expand_includes(content, inc_base, depth + 1);
                    size_t elen = strlen(expanded);
                    while (oi + elen + 2 >= outcap) { outcap = (outcap+elen) * 2 + 1; out = realloc(out, outcap); }
                    memcpy(out + oi, expanded, elen);
                    oi += elen;
                    out[oi++] = '\n';
                    free(content); free(expanded);
                } else {
                    /* copy line unchanged */
                    size_t ll = (size_t)(line_end - p);
                    while (oi + ll + 2 >= outcap) { outcap = (outcap+ll)*2+1; out = realloc(out,outcap); }
                    memcpy(out+oi, p, ll);
                    if (*line_end == '\n') { out[oi+ll] = '\n'; oi += ll + 1; }
                    else oi += ll;
                }
                p = line_end;
                if (*p == '\n') p++;
                continue;
            }
        }
        /* copy char */
        if (oi + 2 >= outcap) { outcap *= 2; out = realloc(out, outcap); }
        out[oi++] = *p++;
    }
    out[oi] = 0;
    return out;
}

static char *skip_ws(const char *s) {
    while (*s && isspace((unsigned char)*s)) s++;
    return (char *)s;
}

static int find_top_level_comma_or_paren(const char *s) {
    /* returns 0 for comma at top, 1 for closing paren at top, -1 otherwise */
    int depth = 0;
    const char *p = s;
    while (*p) {
        if (*p == '(') depth++;
        else if (*p == ')') { if (depth == 0) return 1; depth--; }
        else if (*p == ',' && depth == 0) return 0;
        p++;
    }
    return -1;
}

/* recursive macro expansion */
static char *expand_text(const char *text, int depth) {
    if (!text) return strdup("");
    if (depth > 16) return strdup(text);
    size_t cap = strlen(text) * 4 + 256;
    if (cap < 256) cap = 256;
    char *out = malloc(cap);
    size_t oi = 0;
    const char *p = text;
    while (*p) {
        if (isalpha((unsigned char)*p) || *p == '_') {
            char ident[64]; size_t ii = 0;
            while (isalnum((unsigned char)*p) || *p == '_') {
                if (ii < 63) ident[ii++] = *p;
                p++;
            }
            ident[ii] = 0;
            MacroDef *m = find_macro(ident);
            const char *q = skip_ws(p);
            if (m && m->function_like && *q == '(') {
                /* parse args */
                const char *cur = q + 1;
                int depthp = 1;
                const char *start = cur;
                char *args[8] = {0};
                int argc = 0;
                int done = 0;
                while (*cur && !done) {
                    if (*cur == '(') depthp++;
                    else if (*cur == ')') {
                        depthp--;
                        if (depthp == 0) {
                            size_t al = cur - start;
                            char *raw = malloc(al + 1); memcpy(raw, start, al); raw[al] = 0;
                            char *tr = skip_ws(raw);
                            char *te = tr + strlen(tr);
                            while (te > tr && isspace((unsigned char)te[-1])) te--;
                            *te = 0;
                            if (argc < 8) args[argc] = expand_text(tr, depth + 1);
                            argc++;
                            free(raw);
                            cur++;
                            done = 1;
                            continue;
                        }
                    } else if (*cur == ',' && depthp == 1) {
                        size_t al = cur - start;
                        char *raw = malloc(al + 1); memcpy(raw, start, al); raw[al] = 0;
                        char *tr = skip_ws(raw);
                        char *te = tr + strlen(tr);
                        while (te > tr && isspace((unsigned char)te[-1])) te--;
                        *te = 0;
                        if (argc < 8) args[argc] = expand_text(tr, depth + 1);
                        argc++;
                        free(raw);
                        start = cur + 1;
                    }
                    cur++;
                }
                p = cur - 1; /* advance later */
                p = cur;
                /* temps for names/vals */
                const char *names[8]; const char *vals[8];
                for (int i = 0; i < m->param_count; i++) {
                    names[i] = m->params[i];
                    vals[i] = (i < argc && args[i]) ? args[i] : "";
                }
                char *prepared = NULL;
                size_t cap2 = strlen(m->body)*1024 + 1024;
                prepared = malloc(cap2);
                size_t pi = 0;
                const char *b = m->body;
                while (*b) {
                    if (isalpha((unsigned char)*b) || *b == '_') {
                        char idb[64]; size_t bi = 0;
                        const char *bk = b;
                        while (isalnum((unsigned char)*b) || *b == '_') { if (bi < 63) idb[bi++] = *b; b++; }
                        idb[bi] = 0;
                        int match = -1;
                        for (int k = 0; k < m->param_count; k++) if (strcmp(idb, m->params[k]) == 0) { match = k; break; }
                        if (match >= 0) {
                            const char *av = (match < argc && args[match]) ? args[match] : "";
                            size_t avl = strlen(av);
                            while (pi + avl + 2 >= cap2) { cap2 *= 2; prepared = realloc(prepared, cap2); }
                            memcpy(prepared + pi, av, avl + 1);
                            pi += avl;
                        } else {
                            size_t idl = strlen(idb);
                            while (pi + idl + 2 >= cap2) { cap2 *= 2; prepared = realloc(prepared, cap2); }
                            memcpy(prepared + pi, idb, idl + 1);
                            pi += idl;
                        }
                    } else {
                        if (pi + 2 >= cap2) { cap2 *= 2; prepared = realloc(prepared, cap2); }
                        prepared[pi++] = *b++;
                    }
                }
                prepared[pi] = 0;
                char *rval = expand_text(prepared, depth + 1);
                size_t rv = strlen(rval);
                if (oi + rv + 2 >= cap) { while (oi + rv + 2 >= cap) { cap = cap*2 + 1; out = realloc(out, cap); } }
                memcpy(out + oi, rval, rv + 1);
                oi += rv;
                for (int i = 0; i < argc; i++) if (args[i]) free(args[i]);
                free(prepared);
                free(rval);
            } else if (m) {
                char *rval = expand_text(m->body, depth + 1);
                size_t rv = strlen(rval);
                if (oi + rv + 2 >= cap) { while (oi + rv + 2 >= cap) { cap = cap*2 + 1; out = realloc(out, cap); } }
                memcpy(out + oi, rval, rv + 1);
                oi += rv;
                free(rval);
            } else {
                size_t il = strlen(ident);
                if (oi + il + 2 >= cap) { cap *= 2; out = realloc(out, cap); }
                memcpy(out + oi, ident, il + 1);
                oi += il;
            }
        } else {
            if (oi + 2 >= cap) { cap *= 2; out = realloc(out, cap); }
            out[oi++] = *p++;
        }
    }
    out[oi] = 0;
    return out;
}

static char *preprocess_macros(const char *src) {
    if (!src) return NULL;
    char *out = malloc(strlen(src) * 8 + 1024);
    size_t oi = 0, ocap = strlen(src) * 8 + 1024;
    /* current conditional depth */
    typedef struct { int parent_active; int truth; } CFrame;
    CFrame stack[64];
    int cdepth = 0;
    int active = 1;
    /* iterate line by line */
    const char *p = src;
    while (*p) {
        /* find next newline */
        const char *nl = strchr(p, '\n');
        size_t linelen = nl ? (nl - p) : strlen(p);
        char line[4096];
        if (linelen >= sizeof(line)) linelen = sizeof(line)-1;
        memcpy(line, p, linelen);
        line[linelen] = 0;

        char line_owned[4096];
        strncpy(line_owned, line, sizeof(line_owned)-1);
        line_owned[sizeof(line_owned)-1]=0;
        /* strip leading spaces */
        char *trimmed = skip_ws(line_owned);

        int is_cond = 0;
        if (*trimmed == '#') {
            if (strncmp(trimmed, "#ifdef", 6) == 0) {
                is_cond = 1;
                char *rest = skip_ws(trimmed + 6);
                char name[64]; size_t ni = 0;
                while (isalnum((unsigned char)rest[ni]) || rest[ni]=='_') { if (ni<63) name[ni]=rest[ni]; ni++; }
                name[ni]=0;
                int parent = active;
                int tr = macro_defined(name);
                if (cdepth < 64) {
                    stack[cdepth].parent_active = parent;
                    stack[cdepth].truth = tr;
                    cdepth++;
                }
                active = parent && tr;
            } else if (strncmp(trimmed, "#ifndef", 7) == 0) {
                is_cond = 1;
                char *rest = skip_ws(trimmed + 7);
                char name[64]; size_t ni=0;
                while (isalnum((unsigned char)rest[ni]) || rest[ni]=='_') { if (ni<63) name[ni]=rest[ni]; ni++; }
                name[ni]=0;
                int parent = active;
                int tr = !macro_defined(name);
                if (cdepth < 64) {
                    stack[cdepth].parent_active = parent;
                    stack[cdepth].truth = tr;
                    cdepth++;
                }
                active = parent && tr;
            } else if (strncmp(trimmed, "#if", 3) == 0) {
                is_cond = 1;
                char *rest = skip_ws(trimmed + 3);
                int parent = active;
                int tr;
                if (strncmp((const char *)rest, "defined", 7) == 0) {
                    /* defined(NAME) */
                    char *rps = rest + 7;
                    while (*rps == ' ' || *rps == '(') rps++;
                    char nm[64]; size_t ni=0;
                    while (isalnum((unsigned char)rps[ni]) || rps[ni]=='_') { if (ni<63) nm[ni]=rps[ni]; ni++; }
                    nm[ni]=0;
                    tr = macro_defined(nm);
                } else if (*rest == '0') tr = 0;
                else if (*rest == '1') tr = 1;
                else if (macro_defined(rest)) tr = 1;
                else tr = 1;
                if (cdepth < 64) {
                    stack[cdepth].parent_active = parent;
                    stack[cdepth].truth = tr;
                    cdepth++;
                }
                active = parent && tr;
            } else if (strncmp(trimmed, "#else", 5) == 0) {
                is_cond = 1;
                if (cdepth > 0) {
                    CFrame *f = &stack[cdepth-1];
                    active = f->parent_active && !f->truth;
                    f->truth = !f->truth;
                }
            } else if (strncmp(trimmed, "#endif", 6) == 0) {
                is_cond = 1;
                if (cdepth > 0) {
                    cdepth--;
                    active = stack[cdepth].parent_active;
                }
            }
        }

        int is_directive = (trimmed[0] == '#');

        if (*trimmed == '#') {
            /* handle define/undef/error */
            if (strncmp(trimmed, "#define", 7) == 0) {
                /* parse name */
                char *rest = skip_ws(trimmed + 7);
                char name[64]; size_t ni=0;
                while (isalnum((unsigned char)*rest) || *rest=='_') {
                    if (ni < 63) name[ni] = *rest;
                    ni++; rest++;
                }
                name[ni]=0;
                /* function like? */
                MacroDef *md = NULL;
                int func = 0;
                char params[8][64]; int pc = 0;
                rest = skip_ws(rest);
                if (*rest == '(') {
                    func = 1;
                    rest++;
                    /* parse args */
                    while (*rest && *rest != ')') {
                        char *rt = rest;
                        /* parse param name, skip ws */
                        while (*rt == ' ' || *rt == ',') rt++;
                        if (*rt == ')') break;
                        if (rt != rest) rest = rt;
                        char pn[64]; size_t pin = 0;
                        while (isalnum((unsigned char)*rest) || *rest=='_') {
                            if (pin < 63) pn[pin] = *rest;
                            pin++; rest++;
                        }
                        pn[pin]=0;
                        if (pin) {
                            strcpy(params[pc], pn);
                            pc++;
                        }
                        /* skip to comma or paren */
                        while (*rest && *rest != ',' && *rest != ')') rest++;
                        if (*rest == ',') rest++;
                    }
                    if (*rest == ')') rest++;
                }
                /* body is rest of line (original line) trimmed */
                char *body = skip_ws(rest);
                /* trim trailing */
                char bodybuf[1024];
                strncpy(bodybuf, body, sizeof(bodybuf)-1);
                bodybuf[sizeof(bodybuf)-1]=0;
                /* strip trailing newline? already line */
                size_t bl = strlen(bodybuf);
                while (bl > 0 && isspace((unsigned char)bodybuf[bl-1])) { bodybuf[bl-1]=0; bl--; }
                if (active && name[0] && g_nmacro < 128) {
                    md = &g_macros[g_nmacro++];
                    strcpy(md->name, name);
                    for (int i=0;i<pc;i++) strcpy(md->params[i], params[i]);
                    md->param_count = pc;
                    strcpy(md->body, bodybuf);
                    md->function_like = func;
                }
            } else if (strncmp(trimmed, "#undef", 6) == 0) {
                /* compact macro table remove */
                if (active) {
                    char *rest = skip_ws(trimmed + 6);
                    char name[64]; size_t ni=0;
                    while (isalnum((unsigned char)rest[ni]) || rest[ni]=='_') { if (ni<63) name[ni]=rest[ni]; ni++; }
                    name[ni]=0;
                    for (int i=0;i<g_nmacro;i++) {
                        if (strcmp(g_macros[i].name,name)==0) {
                            memmove(&g_macros[i], &g_macros[i+1], (g_nmacro-i-1)*sizeof(MacroDef));
                            g_nmacro--;
                            break;
                        }
                    }
                }
            } else if (strncmp(trimmed, "#error", 6) == 0) {
                if (active) {
                    fprintf(stderr, "error: %s\n", trimmed + 6);
                    exit(1);
                }
            }
            /* directives themselves do not appear in output */
        } else if (is_cond) {
            /* consumed */
        } else {
            if (active) {
                /* macro-expand entire line and append */
                char *exp = expand_text(line_owned, 0);
                size_t el = strlen(exp);
                while (oi + el + 2 >= ocap) { ocap = (ocap < 64 ? 64 : ocap)*2 + 1; out = realloc(out, ocap); }
                memcpy(out + oi, exp, el);
                oi += el;
                free(exp);
            }
            /* append newline only if original had one */
            if (nl) {
                if (oi + 2 >= ocap) { ocap *= 2; out = realloc(out, ocap); }
                out[oi++] = '\n';
            }
        }

        /* advance p */
        if (nl) { p = nl + 1; }
        else { p += strlen(p); break; }
    }
    out[oi] = 0;
    return out;
}

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc(sz + 1);
    if (!buf) { fclose(f); return NULL; }
    size_t n = fread(buf, 1, sz, f);
    buf[n] = '\0';
    fclose(f);
    return buf;
}

int main(int argc, char **argv) {
    int tokens_only = 0;
    int emit_ir = 0;
    int emit_asm = 0;
    int emit_exe = 0;
    int run_bin = 0;
    const char *path = NULL;
    const char *out_path = "/tmp/opencode/loi_bin";
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--tokens") == 0) tokens_only = 1;
        else if (strcmp(argv[i], "--run") == 0) run_bin = 1;
        else if (strcmp(argv[i], "--emit") == 0 && i+1 < argc) {
            if (strcmp(argv[i+1], "ir") == 0) emit_ir = 1;
            else if (strcmp(argv[i+1], "asm") == 0) emit_asm = 1;
            else if (strcmp(argv[i+1], "exe") == 0) emit_exe = 1;
            i++;
        } else if (strncmp(argv[i], "--emit=", 7) == 0) {
            if (strcmp(argv[i]+7, "ir") == 0) emit_ir = 1;
            else if (strcmp(argv[i]+7, "asm") == 0) emit_asm = 1;
            else if (strcmp(argv[i]+7, "exe") == 0) emit_exe = 1;
        } else if (strcmp(argv[i], "--out") == 0 && i+1 < argc) {
            out_path = argv[i+1]; i++;
        } else if (strncmp(argv[i], "--out=", 6) == 0) {
            out_path = argv[i]+6;
        } else {
            path = argv[i];
        }
    }

    char *src = NULL;
    if (path) {
        src = read_file(path);
        if (!src) {
            fprintf(stderr, "cannot open %s\n", path);
            return 1;
        }
    } else {
        size_t cap = 4096, len = 0;
        src = malloc(cap);
        int c;
        while ((c = fgetc(stdin)) != EOF) {
            if (len + 1 >= cap) { cap *= 2; src = realloc(src, cap); }
            src[len++] = (char)c;
        }
        src[len] = '\0';
    }

    char base_dir_buf[1024];
    strcpy(base_dir_buf, ".");
    if (path) {
        strncpy(base_dir_buf, path, sizeof(base_dir_buf)-1);
        base_dir_buf[sizeof(base_dir_buf)-1] = 0;
        char *slash = strrchr(base_dir_buf, '/');
        if (slash) *slash = 0;
        else strcpy(base_dir_buf, ".");
    }
    char *expanded = expand_includes(src, base_dir_buf, 0);
    free(src);
    char *processed = preprocess_macros(expanded ? expanded : "");
    free(expanded);
    src = processed;

    Lexer lex;
    lexer_init(&lex, src);
    Parser p;
    parser_init(&p, &lex);
    ASTNode *prog = parse_program(&p);

    if (!p.had_error && prog && !tokens_only) {
        if (!typecheck_program(prog)) {
            fprintf(stderr, "type checking failed\n");
        }
    }

    if (tokens_only) {
        /* re-tokenize from src */
        Lexer l2; lexer_init(&l2, src);
        for (;;) {
            Token t = lexer_next(&l2);
            printf("%d:%d  %-12s  %s\n", t.line, t.column, token_type_str(t.type), t.lexeme ? t.lexeme : "");
            int is_eof = (t.type == TOK_EOF);
            token_free(&t);
            if (is_eof) break;
        }
    } else if (emit_ir) {
        const char *ll_path = "/tmp/opencode/loi.ll";
        FILE *f = fopen(ll_path, "w");
        if (f && prog) { codegen_program(prog, f); fclose(f); }
        else if (f) fclose(f);
        char cmd[1024];
        snprintf(cmd, sizeof(cmd), "/usr/lib/llvm-21/bin/opt -O3 -S %s 2>/dev/null", ll_path);
        (void)system(cmd);
    } else if (run_bin) {
        const char *ll_path = "/tmp/opencode/loi.ll";
        const char *bin_path = "/tmp/opencode/loi_bin";
        FILE *f = fopen(ll_path, "w");
        if (f && prog) { codegen_program(prog, f); fclose(f); }
        else if (f) fclose(f);
        char cmd[1024];
        snprintf(cmd, sizeof(cmd), "clang -O3 %s src/runtime.c -o %s -lpthread -lm 2>/dev/null", ll_path, bin_path);
        if (system(cmd) == 0) {
            char run[1024];
            snprintf(run, sizeof(run), "%s", bin_path);
            system(run);
        } else {
            fprintf(stderr, "compile failed\n");
        }
    } else if (emit_exe) {
        const char *ll_path = "/tmp/opencode/loi.ll";
        FILE *f = fopen(ll_path, "w");
        if (f && prog) { codegen_program(prog, f); fclose(f); }
        else if (f) fclose(f);
        char cmd[2048];
        snprintf(cmd, sizeof(cmd), "clang -O3 %s src/runtime.c -o %s -lpthread -lm 2>/dev/null", ll_path, out_path);
        if (system(cmd) == 0) {
            printf("generated executable: %s\n", out_path);
        } else {
            fprintf(stderr, "compile failed\n");
        }
    } else if (emit_asm) {
        const char *ll_path = "/tmp/opencode/loi.ll";
        const char *s_path = "/tmp/opencode/loi.s";
        FILE *f = fopen(ll_path, "w");
        if (!f) { fprintf(stderr, "cannot open temp\n"); return 1; }
        if (prog) codegen_program(prog, f);
        fclose(f);
        char cmd[1024];
        snprintf(cmd, sizeof(cmd), "clang -O3 -S -x ir %s -o %s 2>/dev/null", ll_path, s_path);
        if (system(cmd) != 0) {
            fprintf(stderr, "clang failed\n");
            return 1;
        }
        /* print assembly */
        FILE *sf = fopen(s_path, "r");
        if (sf) {
            int c;
            while ((c = fgetc(sf)) != EOF) fputc(c, stdout);
            fclose(sf);
        }
    } else {
        if (prog) ast_print(prog, 0);
        if (p.had_error) fprintf(stderr, "parse had errors\n");
    }
    if (prog) ast_free(prog);
    free(src);
    return p.had_error ? 1 : 0;
}
