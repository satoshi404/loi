#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <errno.h>

#include <limits.h>

#include "lexer.h"
#include "token.h"
#include "parser.h"
#include "ast.h"
#include "codegen.h"
#include "typecheck.h"
#include "preprocessor.h"
#include "bindgen.h"

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

/* ---------------------------------------------------------------------------
 * --lib=NAME[,NAME...]
 *
 * Each name is turned into linker flags, in this order of preference:
 *   1. `pkg-config --libs NAME`   (then the lower-cased name: SDL2 -> sdl2)
 *   2. `<lower-name>-config --libs` (sdl2-config, freetype-config, ...)
 *   3. `-lNAME`
 * Names are restricted to [A-Za-z0-9_+.-] because they reach a shell.
 * ------------------------------------------------------------------------ */
static int lib_name_ok(const char *n) {
    if (!*n) return 0;
    for (const char *p = n; *p; p++)
        if (!isalnum((unsigned char)*p) && *p != '_' && *p != '+' && *p != '.' && *p != '-')
            return 0;
    return 1;
}

/* Run `cmd`, capture stdout (trailing whitespace trimmed) into `out`.
   Returns 1 when the command exited 0 and printed something. */
static int capture_cmd(const char *cmd, char *out, size_t cap) {
    FILE *f = popen(cmd, "r");
    if (!f) return 0;
    size_t n = fread(out, 1, cap - 1, f);
    out[n] = 0;
    int rc = pclose(f);
    while (n && isspace((unsigned char)out[n - 1])) out[--n] = 0;
    return rc == 0 && n > 0;
}

/* Append the link flags of one library to `flags`. Returns 0 on success. */
static int lib_link_flags(const char *name, char *flags, size_t cap) {
    char lower[128], cmd[512], got[2048];
    size_t i = 0;
    for (; name[i] && i + 1 < sizeof(lower); i++) lower[i] = (char)tolower((unsigned char)name[i]);
    lower[i] = 0;

    int ok = 0;
    snprintf(cmd, sizeof(cmd), "pkg-config --libs %s 2>/dev/null", name);
    ok = capture_cmd(cmd, got, sizeof(got));
    if (!ok && strcmp(lower, name) != 0) {
        snprintf(cmd, sizeof(cmd), "pkg-config --libs %s 2>/dev/null", lower);
        ok = capture_cmd(cmd, got, sizeof(got));
    }
    if (!ok) {
        snprintf(cmd, sizeof(cmd), "%s-config --libs 2>/dev/null", lower);
        ok = capture_cmd(cmd, got, sizeof(got));
    }
    if (!ok) snprintf(got, sizeof(got), "-l%s", name);

    size_t have = strlen(flags), need = strlen(got) + 2;
    if (have + need >= cap) {
        fprintf(stderr, "error: --lib flags too long\n");
        return 1;
    }
    if (have) flags[have++] = ' ';
    memcpy(flags + have, got, need - 1);
    return 0;
}

/* Resolve a comma separated --lib list into `flags` (space separated). */
static int resolve_libs(const char *list, char *flags, size_t cap) {
    char buf[1024];
    if (strlen(list) >= sizeof(buf)) { fprintf(stderr, "error: --lib list too long\n"); return 1; }
    strcpy(buf, list);
    for (char *tok = strtok(buf, ","); tok; tok = strtok(NULL, ",")) {
        while (isspace((unsigned char)*tok)) tok++;
        if (!lib_name_ok(tok)) {
            fprintf(stderr, "error: invalid library name '%s' in --lib\n", tok);
            return 1;
        }
        if (lib_link_flags(tok, flags, cap)) return 1;
    }
    return 0;
}

/* Build the `clang` command that links the emitted IR with the runtime.
   Extra linker flags are appended after the runtime and the default
   libraries; they come from `lib_flags` (--lib) and $LOI_LDFLAGS (for
   example LOI_LDFLAGS="$(sdl2-config --libs)"). Returns 0 on success, 1 if
   the command would not fit in `cmd` (so a long value fails loudly instead
   of being truncated). */
static int build_link_cmd(char *cmd, size_t cmd_size, const char *ll_path,
                          const char *out_path, const char *lib_flags) {
    const char *extra = getenv("LOI_LDFLAGS");
    if (!extra) extra = "";
    int n = snprintf(cmd, cmd_size,
                     "clang -O3 %s src/runtime.c -o %s -lpthread -lm %s %s 2>/dev/null",
                     ll_path, out_path, lib_flags, extra);
    if (n < 0 || (size_t)n >= cmd_size) {
        fprintf(stderr, "link command too long (check --lib / LOI_LDFLAGS)\n");
        return 1;
    }
    return 0;
}

/* The compiler keeps its scratch files in /tmp/opencode; make sure it exists. */
static void ensure_tmp_dir(void) {
    if (mkdir("/tmp/opencode", 0777) != 0 && errno != EEXIST)
        fprintf(stderr, "warning: cannot create /tmp/opencode: %s\n", strerror(errno));
}

/* Match `--name=value` or `--name value`. On a match returns the value and
   advances *i past a separate value argument; otherwise returns NULL. */
static const char *opt_value(int argc, char **argv, int *i, const char *name) {
    size_t n = strlen(name);
    if (strncmp(argv[*i], name, n) != 0) return NULL;
    if (argv[*i][n] == '=') return argv[*i] + n + 1;
    if (argv[*i][n] == 0 && *i + 1 < argc) return argv[++*i];
    return NULL;
}

int main(int argc, char **argv) {
    int tokens_only = 0;
    int emit_ir = 0;
    int emit_asm = 0;
    int emit_exe = 0;
    int run_bin = 0;
    int generator = 0;
    const char *path = NULL;
    
    // Se o usuario nao passar o  parametro --out padrao sera pathoc nome do executavel seraout
    const char *out_path = "out";//"/tmp/opencode/loi_bin";
    int out_given = 0;
    const char *lib_list = NULL;
    const char *gen_package = NULL;
    const char *gen_strip = NULL;
    char gen_includes[2048] = "";
    char lib_flags[4096] = "";
    for (int i = 1; i < argc; i++) {
        const char *v;
        if (strcmp(argv[i], "--tokens") == 0) tokens_only = 1;
        else if (strcmp(argv[i], "--run") == 0) run_bin = 1;
        else if ((v = opt_value(argc, argv, &i, "--emit")) != NULL) {
            if (strcmp(v, "ir") == 0) emit_ir = 1;
            else if (strcmp(v, "asm") == 0) emit_asm = 1;
            else if (strcmp(v, "exe") == 0) emit_exe = 1;
        } else if ((v = opt_value(argc, argv, &i, "--out")) != NULL) {
            out_path = v; out_given = 1;
        } else if ((v = opt_value(argc, argv, &i, "--lib")) != NULL) {
            lib_list = v;
        } else if ((v = opt_value(argc, argv, &i, "--mode")) != NULL) {
            if (strcmp(v, "generator") == 0) generator = 1;
            else { fprintf(stderr, "error: unknown --mode '%s' (expected 'generator')\n", v); return 1; }
        } else if ((v = opt_value(argc, argv, &i, "--package")) != NULL) {
            gen_package = v;
        } else if ((v = opt_value(argc, argv, &i, "--strip-prefix")) != NULL) {
            gen_strip = v;
        } else if (strncmp(argv[i], "-I", 2) == 0) {
            /* header search path for the generator; -Idir or -I dir */
            const char *dir = argv[i] + 2;
            if (!*dir && i + 1 < argc) dir = argv[++i];
            size_t have = strlen(gen_includes);
            if (have + strlen(dir) + 6 < sizeof(gen_includes))
                snprintf(gen_includes + have, sizeof(gen_includes) - have, " -I'%s'", dir);
        } else {
            path = argv[i];
        }
    }

    ensure_tmp_dir();

    if (lib_list && resolve_libs(lib_list, lib_flags, sizeof(lib_flags))) return 1;

    if (generator) {
        if (!path) {
            fprintf(stderr, "usage: loi --mode=generator <header.h> [--out=file.loi] "
                            "[--package=name] [--strip-prefix=PFX] [--lib=NAME] [-Idir]\n");
            return 1;
        }
        /* --lib also tells the generator where the library's headers live */
        if (lib_list) {
            char *copy = strdup(lib_list);
            for (char *tok = strtok(copy, ","); tok; tok = strtok(NULL, ",")) {
                char cmd[512], got[2048];
                snprintf(cmd, sizeof(cmd), "pkg-config --cflags-only-I %s 2>/dev/null", tok);
                if (capture_cmd(cmd, got, sizeof(got))) {
                    size_t have = strlen(gen_includes);
                    if (have + strlen(got) + 2 < sizeof(gen_includes))
                        snprintf(gen_includes + have, sizeof(gen_includes) - have, " %s", got);
                }
            }
            free(copy);
        }
        BindgenOptions bo;
        memset(&bo, 0, sizeof(bo));
        bo.header = path;
        bo.out_path = out_given ? out_path : NULL;
        bo.package = gen_package;
        bo.strip_prefix = gen_strip;
        bo.include_flags = gen_includes;
        return loi_bindgen(&bo);
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

    if (path && !loi_source_declares_package(src)) {
        fprintf(stderr, "error: file \"%s\" must declare a package "
                        "(e.g. '@package(name)')\n", path);
        free(src);
        return 1;
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
char *processed = loi_preprocess(src, base_dir_buf);
    free(src);
    src = processed;

    Lexer lex;
    lexer_init(&lex, src);
    Parser p;
    parser_init(&p, &lex);
    ASTNode *prog = parse_program(&p);

    if (!p.had_error && prog && !tokens_only) {
        if (!typecheck_program(prog)) {
            fprintf(stderr, "type checking failed\n");
            exit(1);
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
    } else if (p.had_error) {
        fprintf(stderr, "parse had errors\n");
        if (prog) ast_free(prog);
        free(src);
        return 1;
    } else if (emit_ir) {
        const char *ll_path = out_given ? out_path : "/tmp/opencode/loi.ll";
        FILE *f = fopen(ll_path, "w");
        if (!f) { fprintf(stderr, "cannot open %s\n", ll_path); return 1; }
        if (prog) codegen_program(prog, f);
        fclose(f);
        /* optimize in place; unoptimized IR is kept if opt is unavailable */
        char cmd[1400];
        snprintf(cmd, sizeof(cmd), "opt -O3 -S %s -o %s 2>/dev/null", ll_path, ll_path);
        int opt_rc = system(cmd);
        (void)opt_rc;
        if (!out_given) {
            FILE *in = fopen(ll_path, "r");
            if (in) { int c; while ((c = fgetc(in)) != EOF) fputc(c, stdout); fclose(in); }
        }
    } else if (run_bin) {
        const char *ll_path = "/tmp/opencode/loi.ll";
	//        const char *bin_path = "out";//"/tmp/opencode/loi_bin";
	const char *bin_path = out_given ? out_path : "/tmp/opencode/loi_bin";

        FILE *f = fopen(ll_path, "w");
        if (f && prog) { codegen_program(prog, f); fclose(f); }
	else if (f) fclose(f);


	char cmd[PATH_MAX + 8];
        snprintf(cmd, sizeof(cmd), "%s%s", strchr(bin_path, '/') ? "" : "./", bin_path); 
        // char cmd[4096];
        if (build_link_cmd(cmd, sizeof(cmd), ll_path, bin_path, lib_flags)) return 1;
        if (system(cmd) != 0) {
            fprintf(stderr, "compile failed\n");
            if (!lib_flags[0] && !getenv("LOI_LDFLAGS"))
                fprintf(stderr, "hint: if the program calls a C library, link it with --lib=NAME (e.g. --lib=SDL2)\n");
            return 1;
        }
	int st = system(cmd);
        //int st = system(bin_path);
        int code = (st >= 0 && WIFEXITED(st)) ? WEXITSTATUS(st) : 1;
        if (prog) ast_free(prog);
        free(src);
        return code;
    } else if (emit_exe) {
        const char *ll_path = "/tmp/opencode/loi.ll";
        FILE *f = fopen(ll_path, "w");
        if (f && prog) { codegen_program(prog, f); fclose(f); }
        else if (f) fclose(f);
        char cmd[4096];
        if (build_link_cmd(cmd, sizeof(cmd), ll_path, out_path, lib_flags)) return 1;
        if (system(cmd) == 0) {
            printf("generated executable: %s\n", out_path);
        } else {
            fprintf(stderr, "compile failed\n");
            if (!lib_flags[0] && !getenv("LOI_LDFLAGS"))
                fprintf(stderr, "hint: if the program calls a C library, link it with --lib=NAME (e.g. --lib=SDL2)\n");
            return 1;
        }
    } else if (emit_asm) {
        const char *ll_path = "/tmp/opencode/loi.ll";
        const char *s_path = out_given ? out_path : "/tmp/opencode/loi.s";
        FILE *f = fopen(ll_path, "w");
        if (!f) { fprintf(stderr, "cannot open temp\n"); return 1; }
        if (prog) codegen_program(prog, f);
        fclose(f);
        char cmd[1400];
        snprintf(cmd, sizeof(cmd), "clang -O3 -S -x ir %s -o %s 2>/dev/null", ll_path, s_path);
        if (system(cmd) != 0) {
            fprintf(stderr, "clang failed\n");
            return 1;
        }
        if (lib_flags[0]) {
            /* assembly is not linked here; record what the final link needs */
            FILE *af = fopen(s_path, "a");
            if (af) { fprintf(af, "# loi: link with %s\n", lib_flags); fclose(af); }
        }
        if (!out_given) {
            /* print assembly */
            FILE *sf = fopen(s_path, "r");
            if (sf) {
                int c;
                while ((c = fgetc(sf)) != EOF) fputc(c, stdout);
                fclose(sf);
            }
        } else {
            printf("generated assembly: %s\n", s_path);
        }
    } else {
        if (prog) ast_print(prog, 0);
    }
    if (prog) ast_free(prog);
    free(src);
    return 0;
}
