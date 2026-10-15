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
        if (*p == '#' || *p == '@') {
            const char *q = p;
            /* find line end */
            const char *line_end = strchr(q, '\n');
            if (!line_end) line_end = q + strlen(q);
            /* trim left */
            while (q < line_end && isspace((unsigned char)*q)) q++;
            const char *kw = q;
            if (*kw == '#') kw++;
            else if (*kw == '@') {
                kw++;
                if (kw < line_end && *kw == '(') { kw++; while (kw < line_end && isspace((unsigned char)*kw)) kw++; }
            }
            if (strncmp(kw, "include", 7) == 0 && !(isalnum((unsigned char)kw[7]) || kw[7] == '_')) {
                q = kw + 7;
                while (q < line_end && isspace((unsigned char)*q)) q++;
                if (*q == '"' || *q == '<') q++;
                char path[512]; size_t pi = 0;
                while (*q && *q != '"' && *q != '>' && *q != ')' && *q != '\n' && pi < 511) path[pi++] = *q++;
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
                    fprintf(stderr, "error: cannot open include file \"%s\"\n", path);
                    exit(1);
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

/* ---------- conditionals: @if(...) { } @elif(...) { } @else { } @endif ---------- */

typedef struct { char *buf; size_t len, cap; } SBuf;

static void sb_init(SBuf *b, size_t cap) {
    if (cap < 64) cap = 64;
    b->buf = malloc(cap);
    b->buf[0] = 0;
    b->len = 0;
    b->cap = cap;
}
static void sb_put(SBuf *b, const char *s, size_t n) {
    if (b->len + n + 1 > b->cap) {
        while (b->len + n + 1 > b->cap) b->cap = b->cap * 2 + 1;
        b->buf = realloc(b->buf, b->cap);
    }
    memcpy(b->buf + b->len, s, n);
    b->len += n;
    b->buf[b->len] = 0;
}
static void sb_putc(SBuf *b, char c) { sb_put(b, &c, 1); }

static int lit_is(const char *s, size_t n, const char *lit) {
    size_t l = strlen(lit);
    if (n < l) return 0;
    return strncmp(s, lit, l) == 0;
}

/* condition expression evaluator: arithmetic, comparisons, !, &&, ||,
   defined(NAME), integer literals and macro names (recursively expanded). */
static long eval_or(const char **e, int depth);
static void eval_skip(const char **e) { while (**e && isspace((unsigned char)**e)) (*e)++; }

static long eval_primary(const char **e, int depth) {
    eval_skip(e);
    if (**e == '(') {
        (*e)++;
        long v = eval_or(e, depth + 1);
        eval_skip(e);
        if (**e == ')') (*e)++;
        return v;
    }
    if (**e == '!' && (*e)[1] != '=') { (*e)++; return !eval_primary(e, depth + 1); }
    if (**e == '-' && (*e)[1] != '>') { (*e)++; return -eval_primary(e, depth + 1); }
    if (**e == '+' ) { (*e)++; return eval_primary(e, depth + 1); }
    if (strncmp(*e, "defined", 7) == 0 &&
        !(isalnum((unsigned char)(*e)[7]) || (*e)[7] == '_')) {
        *e += 7;
        eval_skip(e);
        int paren = (**e == '(');
        if (paren) (*e)++;
        eval_skip(e);
        char nm[64]; size_t ni = 0;
        while (isalnum((unsigned char)**e) || **e == '_') { if (ni < 63) nm[ni] = **e; ni++; (*e)++; }
        nm[ni] = 0;
        if (paren) { eval_skip(e); if (**e == ')') (*e)++; }
        return macro_defined(nm);
    }
    if (isdigit((unsigned char)**e)) {
        char *end = NULL;
        long v = strtol(*e, &end, 10);
        *e = end;
        return v;
    }
    if (isalpha((unsigned char)**e) || **e == '_') {
        char nm[64]; size_t ni = 0;
        while (isalnum((unsigned char)**e) || **e == '_') { if (ni < 63) nm[ni] = **e; ni++; (*e)++; }
        nm[ni] = 0;
        MacroDef *m = find_macro(nm);
        if (!m || m->function_like) return 0;
        char buf[1024];
        strncpy(buf, m->body, sizeof(buf) - 1);
        buf[sizeof(buf) - 1] = 0;
        const char *bp = buf;
        return eval_or(&bp, depth + 1);
    }
    return 0;
}
static long eval_mul(const char **e, int depth) {
    long v = eval_primary(e, depth);
    for (;;) {
        eval_skip(e);
        char c = **e;
        if (c == '*' && (*e)[1] != '*') { (*e)++; v = v * eval_primary(e, depth); }
        else if (c == '/' && (*e)[1] != '/') { (*e)++; long r = eval_primary(e, depth); v = r ? v / r : 0; }
        else if (c == '%') { (*e)++; long r = eval_primary(e, depth); v = r ? v % r : 0; }
        else break;
    }
    return v;
}
static long eval_add(const char **e, int depth) {
    long v = eval_mul(e, depth);
    for (;;) {
        eval_skip(e);
        if (**e == '+' ) { (*e)++; v += eval_mul(e, depth); }
        else if (**e == '-' && (*e)[1] != '>') { (*e)++; v -= eval_mul(e, depth); }
        else break;
    }
    return v;
}
static long eval_rel(const char **e, int depth) {
    long v = eval_add(e, depth);
    for (;;) {
        eval_skip(e);
        if ((*e)[0] == '<' && (*e)[1] == '=') { *e += 2; v = (v <= eval_add(e, depth)); }
        else if ((*e)[0] == '>' && (*e)[1] == '=') { *e += 2; v = (v >= eval_add(e, depth)); }
        else if ((*e)[0] == '<' && (*e)[1] != '<') { (*e)++; v = (v < eval_add(e, depth)); }
        else if ((*e)[0] == '>' && (*e)[1] != '>') { (*e)++; v = (v > eval_add(e, depth)); }
        else break;
    }
    return v;
}
static long eval_eq(const char **e, int depth) {
    long v = eval_rel(e, depth);
    for (;;) {
        eval_skip(e);
        if ((*e)[0] == '=' && (*e)[1] == '=') { *e += 2; v = (v == eval_rel(e, depth)); }
        else if ((*e)[0] == '!' && (*e)[1] == '=') { *e += 2; v = (v != eval_rel(e, depth)); }
        else break;
    }
    return v;
}
static long eval_and(const char **e, int depth) {
    long v = eval_eq(e, depth);
    for (;;) {
        eval_skip(e);
        if ((*e)[0] == '&' && (*e)[1] == '&') { *e += 2; long r = eval_eq(e, depth); v = (v && r); }
        else break;
    }
    return v;
}
static long eval_or(const char **e, int depth) {
    long v = eval_and(e, depth);
    for (;;) {
        eval_skip(e);
        if ((*e)[0] == '|' && (*e)[1] == '|') { *e += 2; long r = eval_and(e, depth); v = (v || r); }
        else break;
    }
    return v;
}
static int eval_cond(const char *expr, int depth) {
    if (!expr || depth > 8) return 0;
    const char *e = expr;
    return eval_or(&e, 0) != 0;
}

/* evaluate to the integer value (not just truthiness) for codegen bounds */
static int eval_int(const char *expr) {
    if (!expr) return 0;
    const char *e = expr;
    return (int)eval_or(&e, 0);
}

/* given s[i]=='(' return index of matching ')' (or end) */
static size_t find_paren_end(const char *s, size_t i, size_t end) {
    int depth = 0;
    int instr = 0;
    for (; i < end; i++) {
        char c = s[i];
        if (instr) { if (c == '\\') { i++; continue; } if (c == '"') instr = 0; continue; }
        if (c == '"') { instr = 1; continue; }
        if (c == '(') depth++;
        else if (c == ')') { depth--; if (depth == 0) return i; }
    }
    return end;
}
/* given s[i]=='{' return index of matching '}' (or end) */
static size_t find_block_end(const char *s, size_t i, size_t end) {
    int depth = 0;
    int instr = 0;
    for (; i < end; i++) {
        char c = s[i];
        if (instr) { if (c == '\\') { i++; continue; } if (c == '"') instr = 0; continue; }
        if (c == '"') { instr = 1; continue; }
        if (c == '{') depth++;
        else if (c == '}') { depth--; if (depth == 0) return i; }
    }
    return end;
}

static void register_define(const char *rest);

static char *pp_range(const char *s, size_t start, size_t end, int depth);
static size_t pp_chain(const char *s, size_t i, size_t end, SBuf *out, int depth) {
    int taken = 0;
    while (i < end) {
        int is_if = lit_is(s + i, end - i, "@if(");
        int is_elif = lit_is(s + i, end - i, "@elif(");
        if (is_if || is_elif) {
            size_t open = is_if ? i + 3 : i + 5;
            size_t cp = find_paren_end(s, open, end);
            char cond[1024];
            size_t cl = (cp > open + 1) ? (cp - open - 1) : 0;
            if (cl > 1023) cl = 1023;
            memcpy(cond, s + open + 1, cl);
            cond[cl] = 0;
            size_t j = cp + 1;
            while (j < end && isspace((unsigned char)s[j])) j++;
            if (j >= end || s[j] != '{') break;
            size_t be = find_block_end(s, j, end);
            int t = 0;
            if (!taken) t = eval_cond(cond, 0);
            if (t) {
                char *sub = pp_range(s, j + 1, be, depth + 1);
                sb_put(out, sub, strlen(sub));
                free(sub);
                taken = 1;
            }
            i = (be < end) ? be + 1 : end;
            size_t k = i;
            while (k < end && isspace((unsigned char)s[k])) k++;
            if (k < end && (lit_is(s + k, end - k, "@elif(") || lit_is(s + k, end - k, "@else"))) { i = k; continue; }
            if (k < end && lit_is(s + k, end - k, "@endif")) i = k + 6;
            return i;
        } else if (lit_is(s + i, end - i, "@else")) {
            size_t j = i + 5;
            while (j < end && isspace((unsigned char)s[j])) j++;
            if (j >= end || s[j] != '{') break;
            size_t be = find_block_end(s, j, end);
            if (!taken) {
                char *sub = pp_range(s, j + 1, be, depth + 1);
                sb_put(out, sub, strlen(sub));
                free(sub);
                taken = 1;
            }
            i = (be < end) ? be + 1 : end;
            size_t k = i;
            while (k < end && isspace((unsigned char)s[k])) k++;
            if (k < end && lit_is(s + k, end - k, "@endif")) i = k + 6;
            return i;
        } else break;
    }
    return i;
}

static size_t count_nl(const char *s, size_t from, size_t to) {
    size_t n = 0;
    for (size_t i = from; i < to; i++) if (s[i] == '\n') n++;
    return n;
}

/* replace whole-identifier occurrences of `word` with `repl` in `text` */
static char *subst_word(const char *text, const char *word, const char *repl) {
    size_t wl = strlen(word), rl = strlen(repl);
    size_t cap = strlen(text) * 2 + 64, n = 0;
    char *out = malloc(cap);
    const char *p = text;
    while (*p) {
        int boundary = (p == text) || !(isalnum((unsigned char)p[-1]) || p[-1] == '_');
        if (wl && boundary && (isalpha((unsigned char)*p) || *p == '_') &&
            strncmp(p, word, wl) == 0 && !(isalnum((unsigned char)p[wl]) || p[wl] == '_')) {
            while (n + rl + 2 >= cap) { cap = cap * 2 + 1; out = realloc(out, cap); }
            memcpy(out + n, repl, rl); n += rl; p += wl;
        } else {
            if (n + 2 >= cap) { cap = cap * 2 + 1; out = realloc(out, cap); }
            out[n++] = *p++;
        }
    }
    out[n] = 0;
    return out;
}

/* split a comma-separated list at top level (respecting parentheses) */
static int split_top(const char *s, char out[][256], int max) {
    int depth = 0, n = 0; size_t oi = 0;
    for (const char *p = s; ; p++) {
        char c = *p;
        if (c == '(') depth++;
        else if (c == ')') depth--;
        if ((c == ',' && depth == 0) || c == 0) {
            out[n][oi] = 0;
            /* trim */
            char *a = out[n];
            while (*a && isspace((unsigned char)*a)) memmove(a, a + 1, strlen(a));
            size_t l = strlen(a);
            while (l > 0 && isspace((unsigned char)a[l - 1])) a[--l] = 0;
            n++; oi = 0;
            if (c == 0 || n >= max) break;
        } else {
            if (oi < 255) out[n][oi++] = c;
        }
    }
    return n;
}

static char *pp_range(const char *s, size_t start, size_t end, int depth) {
    SBuf out;
    sb_init(&out, end > start ? (end - start) + 64 : 64);
    if (depth > 64) { sb_put(&out, s + start, end - start); return out.buf; }
    size_t i = start;
    int instr = 0, inchar = 0;
    while (i < end) {
        char c = s[i];
        if (instr) {
            sb_putc(&out, c);
            if (c == '\\' && i + 1 < end) { sb_putc(&out, s[i + 1]); i += 2; continue; }
            if (c == '"') instr = 0;
            i++;
            continue;
        }
        if (inchar) {
            sb_putc(&out, c);
            if (c == '\\' && i + 1 < end) { sb_putc(&out, s[i + 1]); i += 2; continue; }
            if (c == '\'') inchar = 0;
            i++;
            continue;
        }
        if (c == '"') { instr = 1; sb_putc(&out, c); i++; continue; }
        if (c == '\'') { inchar = 1; sb_putc(&out, c); i++; continue; }
        if (c == '#') {
            /* only a directive if it is the first non-space token on its line */
            size_t ls = i;
            while (ls > 0 && s[ls - 1] != '\n') ls--;
            int onlyws = (ls == 0) || (s[ls - 1] == '\n');
            for (size_t k = ls; k < i && onlyws; k++) {
                if (!isspace((unsigned char)s[k])) onlyws = 0;
            }
            if (onlyws) {
                size_t le = i;
                while (le < end && s[le] != '\n') le++;
                char dir[4096];
                size_t dl = le - i;
                if (dl >= sizeof(dir)) dl = sizeof(dir) - 1;
                memcpy(dir, s + i, dl);
                dir[dl] = 0;
                char *dt = skip_ws(dir);
                if (strncmp(dt, "#define", 7) == 0 && !(isalnum((unsigned char)dt[7]) || dt[7] == '_')) {
                    register_define(dt + 7);
                } else if (strncmp(dt, "#undef", 6) == 0 && !(isalnum((unsigned char)dt[6]) || dt[6] == '_')) {
                    char *rest = skip_ws(dt + 6);
                    char name[64]; size_t ni = 0;
                    while (isalnum((unsigned char)rest[ni]) || rest[ni] == '_') { if (ni < 63) name[ni] = rest[ni]; ni++; }
                    name[ni] = 0;
                    for (int mi = 0; mi < g_nmacro; mi++) {
                        if (strcmp(g_macros[mi].name, name) == 0) {
                            memmove(&g_macros[mi], &g_macros[mi + 1], (g_nmacro - mi - 1) * sizeof(MacroDef));
                            g_nmacro--;
                            break;
                        }
                    }
                } else if (strncmp(dt, "#error", 6) == 0 && !(isalnum((unsigned char)dt[6]) || dt[6] == '_')) {
                    fprintf(stderr, "error: %s\n", dt + 6);
                    exit(1);
                }
                /* strip the directive line but keep the newline for line numbers */
                if (le < end && s[le] == '\n') { sb_putc(&out, '\n'); i = le + 1; }
                else i = le;
                continue;
            }
        }
        if (c == '@' && !lit_is(s + i, end - i, "@align") && !lit_is(s + i, end - i, "@pack")) {
            char prev = (i > 0) ? s[i - 1] : '\n';
            int at_start = (i == 0) || isspace((unsigned char)prev) || prev == '}' || prev == '{' || prev == ';';
            if (at_start && lit_is(s + i, end - i, "@if(")) {
                size_t old_i = i;
                size_t olen = out.len;
                i = pp_chain(s, i, end, &out, depth);
                size_t expected_nl = count_nl(s, old_i, i);
                size_t got_nl = count_nl(out.buf + olen, 0, out.len - olen);
                for (size_t k = got_nl; k < expected_nl; k++) sb_putc(&out, '\n');
                continue;
            }
            if (at_start && lit_is(s + i, end - i, "@endif")) { i += 6; continue; }
            if (at_start && (lit_is(s + i, end - i, "@elif(") || lit_is(s + i, end - i, "@else"))) {
                i += lit_is(s + i, end - i, "@elif(") ? 6 : 5;
                continue;
            }
            /* @define, @undef, @include, @error line-based or parens form */
            if (at_start) {
                if ((lit_is(s + i, end - i, "@repeat(") || lit_is(s + i, end - i, "@for("))) {
                    int is_for = lit_is(s + i, end - i, "@for(");
                    size_t open = i + (is_for ? 4 : 7);
                    size_t cp = find_paren_end(s, open, end);
                    if (cp < end) {
                        char argbuf[1024];
                        size_t al = cp - (open + 1);
                        if (al >= sizeof(argbuf)) al = sizeof(argbuf) - 1;
                        memcpy(argbuf, s + open + 1, al); argbuf[al] = 0;
                        size_t j = cp + 1;
                        while (j < end && isspace((unsigned char)s[j])) j++;
                        if (j < end && s[j] == '{') {
                            size_t be = find_block_end(s, j, end);
                            char body[8192];
                            size_t bl = be - (j + 1);
                            if (bl >= sizeof(body)) bl = sizeof(body) - 1;
                            memcpy(body, s + j + 1, bl); body[bl] = 0;
                            char parts[4][256];
                            int np = split_top(argbuf, parts, 4);
                            if (is_for) {
                                const char *mv = (np >= 1) ? parts[0] : "i";
                                int a = (np >= 2) ? eval_int(parts[1]) : 0;
                                int b = (np >= 3) ? eval_int(parts[2]) : 0;
                                int st = (np >= 4) ? eval_int(parts[3]) : 1;
                                if (st == 0) st = 1;
                                for (int k = a; (st > 0) ? (k < b) : (k > b); k += st) {
                                    char ks[32]; snprintf(ks, sizeof(ks), "%d", k);
                                    char *sub = subst_word(body, mv, ks);
                                    char *proc = pp_range(sub, 0, strlen(sub), depth + 1);
                                    sb_put(&out, proc, strlen(proc));
                                    free(sub); free(proc);
                                }
                            } else {
                                int nrep = (np >= 1) ? eval_int(parts[0]) : 0;
                                for (int k = 0; k < nrep; k++) {
                                    char *proc = pp_range(body, 0, strlen(body), depth + 1);
                                    sb_put(&out, proc, strlen(proc));
                                    free(proc);
                                }
                            }
                            i = (be < end) ? be + 1 : end;
                            continue;
                        }
                    }
                }
                if (i+1 < end && s[i+1] == '(') {
                    size_t cp = find_paren_end(s, i+1, end);
                    if (cp < end) {
                        size_t clen = cp - (i+2);
                        char inner[4096];
                        if (clen >= sizeof(inner)) clen = sizeof(inner)-1;
                        memcpy(inner, s+i+2, clen); inner[clen] = 0;
                        char *t = skip_ws(inner);
                        if (strncmp(t, "define", 6)==0 && !(isalnum((unsigned char)t[6])||t[6]=='_')) {
                            register_define(t+6);
                        } else if (strncmp(t, "undef", 5)==0 && !(isalnum((unsigned char)t[5])||t[5]=='_')) {
                            char *rest = skip_ws(t+5);
                            char name[64]; size_t ni=0;
                            while (isalnum((unsigned char)rest[ni])||rest[ni]=='_') { if (ni<63) name[ni]=rest[ni]; ni++; }
                            name[ni]=0;
                            for (int mi=0; mi<g_nmacro; mi++) if (strcmp(g_macros[mi].name,name)==0) { memmove(&g_macros[mi],&g_macros[mi+1],(g_nmacro-mi-1)*sizeof(MacroDef)); g_nmacro--; break; }
                        } else if (strncmp(t, "error", 5)==0 && !(isalnum((unsigned char)t[5])||t[5]=='_')) {
                            fprintf(stderr, "error: %s\n", skip_ws(t+5));
                            exit(1);
                        }
                        /* @(...) directives are meta-only: strip the line but keep
                           its newline so line numbers stay accurate. Includes are
                           already resolved (or reported) by expand_includes. */
                        /* keep newline for line count? find end of this directive's line */
                        size_t le = cp+1;
                        while (le < end && s[le]!='\n') le++;
                        if (le < end && s[le]=='\n') { sb_putc(&out, '\n'); i = le+1; }
                        else i = le;
                        continue;
                    }
                }
                /* line-based @define etc */
                if (strncmp(s+i, "@define", 7)==0 && !(isalnum((unsigned char)s[i+7])||s[i+7]=='_')) {
                    size_t le = i+7;
                    while (le < end && s[le]!='\n') le++;
                    char dl[4096]; size_t dn = le-(i+7);
                    if (dn >= sizeof(dl)) dn = sizeof(dl)-1;
                    memcpy(dl, s+i+7, dn); dl[dn]=0;
                    register_define(dl);
                    if (le < end && s[le]=='\n') { sb_putc(&out, '\n'); i = le+1; }
                    else i = le; continue;
                }
                if (strncmp(s+i, "@undef", 6)==0 && !(isalnum((unsigned char)s[i+6])||s[i+6]=='_')) {
                    size_t le = i+6;
                    while (le < end && s[le]!='\n') le++;
                    char *rest = skip_ws(s+6);
                    char name[64]; size_t ni=0;
                    while (isalnum((unsigned char)rest[ni])||rest[ni]=='_') { if (ni<63) name[ni]=rest[ni]; ni++; }
                    name[ni]=0;
                    for (int mi=0; mi<g_nmacro; mi++) if (strcmp(g_macros[mi].name,name)==0) { memmove(&g_macros[mi],&g_macros[mi+1],(g_nmacro-mi-1)*sizeof(MacroDef)); g_nmacro--; break; }
                    if (le < end && s[le]=='\n') { sb_putc(&out, '\n'); i = le+1; }
                    else i = le; continue;
                }
                if (strncmp(s+i, "@error", 6)==0 && !(isalnum((unsigned char)s[i+6])||s[i+6]=='_')) {
                    size_t le = i+6;
                    while (le < end && s[le]!='\n') le++;
                    fprintf(stderr, "error: %s\n", skip_ws(s+6));
                    exit(1);
                }
            }
        }
        sb_putc(&out, c);
        i++;
    }
    return out.buf;
}

/* register a #define from the text following the keyword */
static void register_define(const char *rest) {
    rest = skip_ws(rest);
    char name[64]; size_t ni = 0;
    while (isalnum((unsigned char)*rest) || *rest == '_') {
        if (ni < 63) name[ni] = *rest;
        ni++; rest++;
    }
    name[ni] = 0;
    int func = 0;
    char params[8][64]; int pc = 0;
    rest = skip_ws(rest);
    if (*rest == '(') {
        func = 1;
        rest++;
        while (*rest && *rest != ')') {
            while (*rest == ' ' || *rest == ',') rest++;
            if (*rest == ')') break;
            char pn[64]; size_t pin = 0;
            while (isalnum((unsigned char)*rest) || *rest == '_') {
                if (pin < 63) pn[pin] = *rest;
                pin++; rest++;
            }
            pn[pin] = 0;
            if (pin && pc < 8) strcpy(params[pc++], pn);
            while (*rest && *rest != ',' && *rest != ')') rest++;
            if (*rest == ',') rest++;
        }
        if (*rest == ')') rest++;
    }
    char *body = skip_ws(rest);
    char bodybuf[1024];
    strncpy(bodybuf, body, sizeof(bodybuf) - 1);
    bodybuf[sizeof(bodybuf) - 1] = 0;
    size_t bl = strlen(bodybuf);
    while (bl > 0 && isspace((unsigned char)bodybuf[bl - 1])) { bodybuf[bl - 1] = 0; bl--; }
    if (name[0] && g_nmacro < 128) {
        MacroDef *md = &g_macros[g_nmacro++];
        strcpy(md->name, name);
        for (int i = 0; i < pc; i++) strcpy(md->params[i], params[i]);
        md->param_count = pc;
        strcpy(md->body, bodybuf);
        md->function_like = func;
    }
}

/* collect all #define lines so @if(...) can test them */
static char *preprocess_macros(const char *src) {
    if (!src) return NULL;
    /* pp_range evaluates @if/@elif/@else while registering #define/#undef in
       order, so defines inside skipped branches are never registered. */
    char *cond_src = pp_range(src, 0, strlen(src), 0);
    size_t srclen = strlen(cond_src);
    char *out = malloc(srclen * 8 + 1024);
    size_t oi = 0, ocap = srclen * 8 + 1024;
    const char *p = cond_src;
    while (*p) {
        const char *nl = strchr(p, '\n');
        size_t linelen = nl ? (nl - p) : strlen(p);
        char line[4096];
        if (linelen >= sizeof(line)) linelen = sizeof(line) - 1;
        memcpy(line, p, linelen);
        line[linelen] = 0;

        char *exp = expand_text(line, 0);
        size_t el = strlen(exp);
        while (oi + el + 2 >= ocap) { ocap = ocap * 2 + 1; out = realloc(out, ocap); }
        memcpy(out + oi, exp, el);
        oi += el;
        free(exp);
        if (nl) {
            if (oi + 2 >= ocap) { ocap *= 2; out = realloc(out, ocap); }
            out[oi++] = '\n';
        }

        if (nl) p = nl + 1;
        else break;
    }
    out[oi] = 0;
    free(cond_src);
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
