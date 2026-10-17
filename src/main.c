#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <sys/wait.h>

#include "lexer.h"
#include "token.h"
#include "parser.h"
#include "ast.h"
#include "codegen.h"
#include "typecheck.h"
#include "preprocessor.h"

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

/* Build the `clang` command that links the emitted IR with the runtime.
   Extra linker flags come from $LOI_LDFLAGS (for example
   LOI_LDFLAGS="$(sdl2-config --libs)") and are appended after the runtime and
   the default libraries. Returns 0 on success, 1 if the command would not fit
   in `cmd` (so a long value fails loudly instead of being truncated). */
static int build_link_cmd(char *cmd, size_t cmd_size,
                          const char *ll_path, const char *out_path) {
    const char *extra = getenv("LOI_LDFLAGS");
    if (!extra) extra = "";
    int n = snprintf(cmd, cmd_size,
                     "clang -O3 %s src/runtime.c -o %s -lpthread -lm %s 2>/dev/null",
                     ll_path, out_path, extra);
    if (n < 0 || (size_t)n >= cmd_size) {
        fprintf(stderr, "link command too long (check LOI_LDFLAGS)\n");
        return 1;
    }
    return 0;
}

int main(int argc, char **argv) {
    int tokens_only = 0;
    int emit_ir = 0;
    int emit_asm = 0;
    int emit_exe = 0;
    int run_bin = 0;
    const char *path = NULL;
    const char *out_path = "/tmp/opencode/loi_bin";
    int out_given = 0;
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
            out_path = argv[i+1]; out_given = 1; i++;
        } else if (strncmp(argv[i], "--out=", 6) == 0) {
            out_path = argv[i]+6; out_given = 1;
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
        const char *bin_path = "/tmp/opencode/loi_bin";
        FILE *f = fopen(ll_path, "w");
        if (f && prog) { codegen_program(prog, f); fclose(f); }
        else if (f) fclose(f);
        char cmd[4096];
        if (build_link_cmd(cmd, sizeof(cmd), ll_path, bin_path)) return 1;
        if (system(cmd) != 0) {
            fprintf(stderr, "compile failed\n");
            return 1;
        }
        int st = system(bin_path);
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
        if (build_link_cmd(cmd, sizeof(cmd), ll_path, out_path)) return 1;
        if (system(cmd) == 0) {
            printf("generated executable: %s\n", out_path);
        } else {
            fprintf(stderr, "compile failed\n");
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
