#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <stdarg.h>
#include <limits.h>
#include "preprocessor.h"

/* ---------------------------------------------------------------------------
 * Limits and constants
 *
 * These were previously scattered as magic numbers throughout the file. They
 * are gathered here so the buffering/recursion policy is easy to audit.
 * ------------------------------------------------------------------------ */
#define MAX_MACROS        128    /* macro table capacity                     */
#define MAX_MACRO_PARAMS  8      /* parameters per function-like macro       */
#define MAX_MACRO_BODY    1024   /* stored macro body length                 */
#define MAX_IDENT         64     /* identifier buffer length                 */
#define MAX_EXPAND_DEPTH  16     /* recursive macro expansion depth          */
#define MAX_INCLUDE_DEPTH 16     /* recursive @inc depth                     */
#define MAX_COND_DEPTH    8      /* nested @if evaluation depth              */
#define MAX_PP_DEPTH      64     /* recursive pp_range depth                 */
#define MAX_COND_LEN      1024   /* @if(...) condition text                  */
#define MAX_INLINE_LEN    4096   /* a single directive line / meta argument  */
#define MAX_BLOCK_LEN     8192   /* @repeat/@for body text                   */
#define MAX_LIST_PARTS    4      /* @for(a,b,c,d) parts                      */
#define MAX_INCLUDE_CAND  12     /* candidate paths tried by @inc/@cinc      */
#define MAX_PART_LEN      256    /* one @for/@repeat(...) argument           */
#define MAX_NUM_LEN       32     /* a formatted integer                      */
#ifndef PATH_MAX
#define PATH_MAX          4096
#endif
#define MAX_PATH          PATH_MAX  /* include path buffer                    */

/* ---------------------------------------------------------------------------
 * Allocation helpers
 *
 * The compiler is a short-lived batch process: an allocation failure is not
 * recoverable, so these abort with a clear message instead of forcing every
 * caller to thread error returns through the whole preprocessor.
 * ------------------------------------------------------------------------ */
static void *xmalloc(size_t n) {
    void *p = malloc(n ? n : 1);
    if (!p) { fprintf(stderr, "loi: out of memory\n"); exit(1); }
    return p;
}
static void *xrealloc(void *p, size_t n) {
    void *q = realloc(p, n ? n : 1);
    if (!q) { fprintf(stderr, "loi: out of memory\n"); exit(1); }
    return q;
}
static char *xstrdup(const char *s) {
    if (!s) return NULL;
    size_t n = strlen(s);
    char *r = xmalloc(n + 1);
    memcpy(r, s, n + 1);
    return r;
}

/* ---------------------------------------------------------------------------
 * StrBuf: a small growable string buffer.
 *
 * All string building in the preprocessor goes through this type; there is no
 * manual `realloc` bookkeeping left in the rest of the file.
 * ------------------------------------------------------------------------ */
typedef struct { char *buf; size_t len, cap; } StrBuf;

static void sb_init(StrBuf *b) {
    b->buf = xmalloc(64);
    b->buf[0] = '\0';
    b->len = 0;
    b->cap = 64;
}
static void sb_reserve(StrBuf *b, size_t extra) {
    if (b->len + extra + 1 <= b->cap) return;
    while (b->len + extra + 1 > b->cap) b->cap = b->cap * 2 + 1;
    b->buf = xrealloc(b->buf, b->cap);
}
static void sb_put(StrBuf *b, const char *s, size_t n) {
    sb_reserve(b, n);
    memcpy(b->buf + b->len, s, n);
    b->len += n;
    b->buf[b->len] = 0;
}
static void sb_putc(StrBuf *b, char c) { sb_put(b, &c, 1); }
static void sb_puts(StrBuf *b, const char *s) { sb_put(b, s, strlen(s)); }
static void sb_printf(StrBuf *b, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    va_list ap2;
    va_copy(ap2, ap);
    int need = vsnprintf(NULL, 0, fmt, ap);
    va_end(ap);
    if (need < 0) { va_end(ap2); return; }
    sb_reserve(b, (size_t)need);
    vsnprintf(b->buf + b->len, (size_t)need + 1, fmt, ap2);
    va_end(ap2);
    b->len += (size_t)need;
}
/* Detach the buffer (caller owns it); the StrBuf is left empty. */
static char *sb_take(StrBuf *b) {
    char *r = b->buf ? b->buf : xmalloc(1);
    if (!b->buf) r[0] = 0;
    b->buf = NULL;
    b->len = b->cap = 0;
    return r;
}

/* ---------------------------------------------------------------------------
 * Small text helpers
 * ------------------------------------------------------------------------ */
static char *skip_ws(const char *s) {
    while (*s && isspace((unsigned char)*s)) s++;
    return (char *)s;
}

/* Copy `src` into `dst[cap]`, always NUL-terminating. Silent truncation is
   fine here: these are fixed-size identifiers/paths, never user data we must
   preserve byte-for-byte. */
static void copy_str(char *dst, size_t cap, const char *src) {
    if (!cap) return;
    size_t n = strlen(src);
    if (n >= cap) n = cap - 1;
    memcpy(dst, src, n);
    dst[n] = '\0';
}

/* True when only whitespace precedes `i` on its line: a directive must start
   a line, so `@inc`/`@error` inside a comment or string is plain text. */
static int line_blank_before(const char *s, size_t i) {
    while (i > 0 && s[i - 1] != '\n') {
        if (!isspace((unsigned char)s[i - 1])) return 0;
        i--;
    }
    return 1;
}

/* Same as above but over the output buffer already produced. */
static int at_line_start(const char *out, size_t oi) {
    while (oi > 0 && out[oi - 1] != '\n') {
        if (!isspace((unsigned char)out[oi - 1])) return 0;
        oi--;
    }
    return 1;
}

/* `name` is a whole identifier match for `kw` at `p` (not a longer word). */
static int kw_match(const char *p, const char *kw) {
    size_t n = strlen(kw);
    if (strncmp(p, kw, n) != 0) return 0;
    char c = p[n];
    return !(isalnum((unsigned char)c) || c == '_');
}

typedef struct {
    char name[MAX_IDENT];
    int param_count;
    char params[MAX_MACRO_PARAMS][MAX_IDENT];
    char body[MAX_MACRO_BODY];
    int function_like;
    int variadic;
    char va_name[MAX_IDENT];
} LoiMacroDef;

static LoiMacroDef g_macros[MAX_MACROS];
static int g_nmacro = 0;

static LoiMacroDef *find_macro(const char *name) {
    for (int i = 0; i < g_nmacro; i++) {
        if (strcmp(g_macros[i].name, name) == 0) return &g_macros[i];
    }
    return NULL;
}

static int macro_defined(const char *name) {
    return find_macro(name) != NULL;
}

static void macro_undef(const char *name) {
    for (int i = 0; i < g_nmacro; i++) {
        if (strcmp(g_macros[i].name, name) == 0) {
            memmove(&g_macros[i], &g_macros[i + 1], (size_t)(g_nmacro - i - 1) * sizeof(LoiMacroDef));
            g_nmacro--;
            return;
        }
    }
}

static void read_name(const char **pp, char *out, size_t cap) {
    size_t n = 0;
    const char *p = *pp;
    while (isalnum((unsigned char)*p) || *p == '_') {
        if (n + 1 < cap) out[n] = *p;
        n++;
        p++;
    }
    out[n < cap ? n : cap - 1] = 0;
    *pp = p;
}

static char *read_text_file(const char *path) {
    if (!path) return NULL;
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return NULL; }
    long sz = ftell(f);
    if (sz < 0) { fclose(f); return NULL; }
    fseek(f, 0, SEEK_SET);
    char *buf = xmalloc((size_t)sz + 1);
    size_t n = fread(buf, 1, (size_t)sz, f);
    buf[n] = '\0';
    fclose(f);
    return buf;
}

/* ---------------------------------------------------------------------------
 * `import` is gone
 *
 * The old `import pkg.sub` form used to be rewritten into `@inc pkg/sub` here.
 * The keyword has been removed from the grammar, so every inclusion/package
 * reference must be written with the `@` sigil (`@inc` / `@cinc`). The parser
 * reports a clear error for the leftover keyword; see parser.c.
 * ------------------------------------------------------------------------ */

/* ---------------------------------------------------------------------------
 * Include guards
 *
 * `@inc` behaves like `#pragma once`: the first time a given real path is
 * included it is spliced in; any later `@inc` of the same path expands to
 * nothing. This protects loi libraries against double inclusion (and against
 * include cycles) without manual `#ifndef` guards. `@cinc` is unaffected.
 * ------------------------------------------------------------------------ */
static char **g_included = NULL;
static size_t g_included_count = 0;
static size_t g_included_cap = 0;

static void include_guard_reset(void) {
    for (size_t i = 0; i < g_included_count; i++) free(g_included[i]);
    free(g_included);
    g_included = NULL;
    g_included_count = 0;
    g_included_cap = 0;
}

/* Returns 1 if `path` was already included; otherwise records it and returns
   0. The real path is used when available so `./lib/x.loi` and `lib/x.loi`
   collapse to a single entry. */
static int include_guard_seen(const char *path) {
    char real[PATH_MAX];
    const char *key = path;
    if (realpath(path, real)) key = real;
    for (size_t i = 0; i < g_included_count; i++) {
        if (strcmp(g_included[i], key) == 0) return 1;
    }
    if (g_included_count == g_included_cap) {
        g_included_cap = g_included_cap ? g_included_cap * 2 : 16;
        g_included = xrealloc(g_included, g_included_cap * sizeof(*g_included));
    }
    g_included[g_included_count++] = xstrdup(key);
    return 0;
}

/* 1-based line number of `p` within the buffer starting at `s`. */
static size_t line_of(const char *s, const char *p) {
    size_t line = 1;
    for (const char *q = s; q < p; q++) if (*q == '\n') line++;
    return line;
}

/* Resolve and read an included file. On success returns the contents (caller
   frees) and copies the winning candidate path into `found`.
   `c_header` selects the C-header search path (@cinc); otherwise the loi
   library search path (@inc) is used. */
static char *locate_include(const char *base_dir, const char *path, int c_header,
                            char *found, size_t found_cap) {
    char cands[MAX_INCLUDE_CAND][MAX_PATH];
    int n = 0;
    snprintf(cands[n++], MAX_PATH, "%s/%s", (base_dir && *base_dir) ? base_dir : ".", path);
    snprintf(cands[n++], MAX_PATH, "%s", path);
    if (c_header) {
        snprintf(cands[n++], MAX_PATH, "/usr/include/%s", path);
        snprintf(cands[n++], MAX_PATH, "/usr/local/include/%s", path);
        snprintf(cands[n++], MAX_PATH, "/usr/include/x86_64-linux-gnu/%s", path);
    } else {
        snprintf(cands[n++], MAX_PATH, "./lib/%s", path);
        snprintf(cands[n++], MAX_PATH, "./lib/%s.loi", path);
        snprintf(cands[n++], MAX_PATH, "./packages/%s", path);
        snprintf(cands[n++], MAX_PATH, "./packages/%s.loi", path);
        snprintf(cands[n++], MAX_PATH, "/usr/share/loi/%s.loi", path);
        snprintf(cands[n++], MAX_PATH, "/usr/local/share/loi/%s.loi", path);
        const char *env = getenv("LOI_PATH");
        if (env && *env) snprintf(cands[n++], MAX_PATH, "%s/%s.loi", env, path);
    }
    for (int i = 0; i < n; i++) {
        char *content = read_text_file(cands[i]);
        if (content) {
            if (found && found_cap) copy_str(found, found_cap, cands[i]);
            return content;
        }
    }
    return NULL;
}

/* Splice `@inc` / `@cinc` directives, recursing into included loi files. */
static char *expand_includes(const char *src, const char *base_dir, int depth) {
    if (!src) return NULL;
    if (!base_dir) base_dir = ".";
    if (depth > MAX_INCLUDE_DEPTH) return xstrdup(src);

    StrBuf out;
    sb_init(&out);
    const char *p = src;
    while (*p) {
        if (at_line_start(out.buf, out.len) && (*p == '#' || *p == '@')) {
            const char *line_end = strchr(p, '\n');
            if (!line_end) line_end = p + strlen(p);
            /* keyword after `@`/`#`, allowing the parenthesized `@(inc ...)` */
            const char *kw = p;
            while (kw < line_end && isspace((unsigned char)*kw)) kw++;
            char sigil = '#';   /* the directive's leading sigil, for messages */
            int kind = 0, klen = 0;   /* 1 = @inc, 2 = @cinc */
            if (kw < line_end && *kw == '#') {
                sigil = '#';
                kw++;
            } else if (kw < line_end && *kw == '@') {
                sigil = '@';
                kw++;
                if (kw < line_end && *kw == '(') {
                    kw++;
                    while (kw < line_end && isspace((unsigned char)*kw)) kw++;
                }
            }
            if (kw_match(kw, "include")) {
                fprintf(stderr,
                        "error: line %zu: '%cinclude' is no longer supported; "
                        "use '@inc' for loi files or '@cinc' for C headers\n",
                        line_of(src, p), sigil);
                exit(1);
            } else if (kw_match(kw, "cinc")) { kind = 2; klen = 4; }
            else if (kw_match(kw, "inc"))     { kind = 1; klen = 3; }

            if (kind) {
                const char *q = kw + klen;
                while (q < line_end && isspace((unsigned char)*q)) q++;
                /* A quoted or `<...>` path is taken literally; a bare package
                   name like `std.math` has its dots mapped to path separators. */
                int literal = (*q == '"' || *q == '<');
                if (literal) q++;
                char path[MAX_PATH];
                size_t pi = 0;
                while (q < line_end && *q && pi + 1 < sizeof(path)) {
                    char c = *q;
                    if (literal) {
                        if (c == '"' || c == '>') break;
                    } else {
                        /* a bare path ends at whitespace, `;`, `)` or a comment */
                        if (isspace((unsigned char)c) || c == ';' || c == ')') break;
                        if (c == '/' && (q[1] == '*' || q[1] == '/')) break;
                    }
                    path[pi++] = c;
                    q++;
                }
                path[pi] = 0;
                while (pi > 0 && isspace((unsigned char)path[pi - 1])) path[--pi] = 0;
                /* `@inc pkg.sub` -> `pkg/sub`, but leave real file paths alone
                   (anything with a slash or a `.loi` suffix). */
                if (kind == 1 && !literal && !strchr(path, '/') &&
                    !(pi >= 4 && strcmp(path + pi - 4, ".loi") == 0)) {
                    for (size_t k = 0; k < pi; k++) if (path[k] == '.') path[k] = '/';
                }

                char found[MAX_PATH];
                found[0] = 0;
                char *content = locate_include(base_dir, path, kind == 2, found, sizeof(found));
                if (!content) {
                    fprintf(stderr, "error: line %zu: cannot find included file \"%s\"\n",
                            line_of(src, p), path);
                    exit(1);
                }
                if (kind == 2) {
                    /* @cinc only verifies the header exists; leave a trace marker. */
                    sb_printf(&out, "/* @cinc: %s */\n", found);
                    free(content);
                } else if (include_guard_seen(found)) {
                    free(content);    /* already included (include guard) */
                } else {
                    char inc_base[MAX_PATH];
                    strncpy(inc_base, found, sizeof(inc_base) - 1);
                    inc_base[sizeof(inc_base) - 1] = 0;
                    char *slash = strrchr(inc_base, '/');
                    if (slash) *slash = 0; else strcpy(inc_base, ".");
                    char *nested = expand_includes(content, inc_base, depth + 1);
                    sb_puts(&out, nested);
                    sb_putc(&out, '\n');
                    free(nested);
                    free(content);
                }
                p = line_end;
                if (*p == '\n') p++;
                continue;
            }
        }
        sb_putc(&out, *p++);
    }
    return sb_take(&out);
}

/* recursive macro expansion */
/* ---- meta-op helpers for @stringify / @paste (defined here since they sit above expand_text) ---- */
static void copy_literal(const char **pp, StrBuf *out);
static char *meta_paren_inner(const char *s, size_t start, size_t *after);
static char *reduce_meta(const char *text, int depth);
static char *expand_text(const char *text, int depth);

/* Copy a string/char literal (including surrounding quotes and escapes) into
   `out`, advancing `*pp` past it. Used to keep literals opaque to macro
   substitution. */
static void copy_literal(const char **pp, StrBuf *out) {
    const char *p = *pp;
    char quote = *p;
    sb_putc(out, *p++);
    while (*p && *p != quote) {
        if (*p == '\\' && p[1]) {
            sb_putc(out, *p++);
            sb_putc(out, *p++);
            continue;
        }
        sb_putc(out, *p++);
    }
    if (*p == quote) sb_putc(out, *p++);
    *pp = p;
}

/* Extract the balanced text inside `s[start]=='('`, trimming outer whitespace.
   `*after` receives the index just past the closing ')'. */
static char *meta_paren_inner(const char *s, size_t start, size_t *after) {
    size_t i = start + 1, b = i;
    int depth = 1;
    while (s[i] && depth) {
        char c = s[i];
        if (c == '(') depth++;
        else if (c == ')') { depth--; if (!depth) break; }
        else if (c == '"' || c == '\\') {
            char q = c;
            i++;
            while (s[i] && s[i] != q) { if (s[i] == '\\') i++; i++; }
        }
        i++;
    }
    size_t e = i;
    while (e > b && isspace((unsigned char)s[e - 1])) e--;
    size_t b2 = b;
    while (b2 < e && isspace((unsigned char)s[b2])) b2++;
    char *out = xmalloc(e - b2 + 1);
    memcpy(out, s + b2, e - b2);
    out[e - b2] = 0;
    *after = (s[i] == ')') ? i + 1 : i;
    return out;
}

/* Slice [start,end), trimming surrounding whitespace, into a fresh string. */
static char *slice_trimmed(const char *start, const char *end) {
    while (start < end && isspace((unsigned char)*start)) start++;
    while (end > start && isspace((unsigned char)end[-1])) end--;
    size_t n = (size_t)(end - start);
    char *r = xmalloc(n + 1);
    memcpy(r, start, n);
    r[n] = '\0';
    return r;
}

/* Evaluate @stringify(x) and @paste(a, b, ...) meta-operators. */
static char *reduce_meta(const char *text, int depth) {
    if (!text) return xstrdup("");
    if (depth > MAX_EXPAND_DEPTH) return xstrdup(text);
    StrBuf out;
    sb_init(&out);
    const char *p = text;
    while (*p) {
        if (*p == '"' || *p == '\\') { copy_literal(&p, &out); continue; }
        if (*p == '@' && (strncmp(p, "@stringify", 10) == 0 || strncmp(p, "@paste", 6) == 0)) {
            int is_str = (strncmp(p, "@stringify", 10) == 0);
            const char *q2 = p + (is_str ? 10 : 6);
            while (*q2 && isspace((unsigned char)*q2)) q2++;
            if (*q2 == '(') {
                size_t after = 0;
                char *inner = meta_paren_inner(text, (size_t)(q2 - text), &after);
                if (is_str) {
                    /* @stringify: wrap the text in quotes, escaping " and \ */
                    sb_putc(&out, '"');
                    for (const char *t = inner; *t; t++) {
                        if (*t == '"' || *t == '\\') sb_putc(&out, '\\');
                        sb_putc(&out, *t);
                    }
                    sb_putc(&out, '"');
                } else {
                    /* @paste: concatenate the comma-separated pieces, trimming */
                    char *reduced = reduce_meta(inner, depth + 1);
                    int d = 0, more = 0;
                    for (char *t = reduced; ; ) {
                        char *seg = t;
                        while (*t) {
                            if (*t == '(') d++;
                            else if (*t == ')') d--;
                            else if (*t == ',' && d == 0) { *t = 0; more = 1; break; }
                            t++;
                        }
                        while (*seg && isspace((unsigned char)*seg)) seg++;
                        char *se = seg + strlen(seg);
                        while (se > seg && isspace((unsigned char)se[-1])) *--se = 0;
                        sb_puts(&out, seg);
                        if (!more) break;
                        more = 0;
                        t++;
                    }
                    free(reduced);
                }
                free(inner);
                p = text + after;
                continue;
            }
        }
        /* Copy plain text up to the next literal or meta-operator. */
        while (*p && *p != '"' && *p != '\\' &&
               !(p[0] == '@' && (strncmp(p, "@stringify", 10) == 0 || strncmp(p, "@paste", 6) == 0))) {
            sb_putc(&out, *p++);
        }
        if (*p == '@') sb_putc(&out, *p++);
    }
    return sb_take(&out);
}

/* Expand object-like and function-like macros in `text`. */
static char *expand_text(const char *text, int depth) {
    if (!text) return xstrdup("");
    if (depth > MAX_EXPAND_DEPTH) return xstrdup(text);
    StrBuf out;
    sb_init(&out);
    const char *p = text;
    while (*p) {
        if (!(isalpha((unsigned char)*p) || *p == '_')) {
            if (*p == '"' || *p == '\\') { copy_literal(&p, &out); continue; }
            sb_putc(&out, *p++);
            continue;
        }
        char ident[MAX_IDENT];
        read_name(&p, ident, sizeof(ident));
        LoiMacroDef *m = find_macro(ident);
        const char *q = skip_ws(p);
        if (m && m->function_like && *q == '(') {
            /* Split the call's arguments at top-level commas. */
            const char *cur = q + 1;
            int paren = 1;
            const char *arg_start = cur;
            char *args[MAX_MACRO_PARAMS] = {0};
            int argc = 0;
            while (*cur) {
                if (*cur == '(') paren++;
                else if (*cur == ')') {
                    paren--;
                    if (paren == 0) {
                        if (argc < MAX_MACRO_PARAMS) args[argc] = slice_trimmed(arg_start, cur);
                        argc++;
                        cur++;
                        break;
                    }
                } else if (*cur == ',' && paren == 1) {
                    if (argc < MAX_MACRO_PARAMS) args[argc] = slice_trimmed(arg_start, cur);
                    argc++;
                    arg_start = cur + 1;
                }
                cur++;
            }
            p = cur;   /* continue after the invocation */
            /* Fold the trailing arguments into the variadic parameter. */
            if (m->variadic) {
                int fixed = m->param_count - 1;
                if (fixed < 0) fixed = 0;
                StrBuf joined;
                sb_init(&joined);
                for (int i = fixed; i < argc && i < MAX_MACRO_PARAMS; i++) {
                    if (i > fixed) sb_puts(&joined, ", ");
                    sb_puts(&joined, args[i] ? args[i] : "");
                    free(args[i]);
                    args[i] = NULL;
                }
                if (fixed < MAX_MACRO_PARAMS) {
                    free(args[fixed]);
                    args[fixed] = sb_take(&joined);
                    argc = fixed + 1;
                } else {
                    free(sb_take(&joined));
                }
            }
            /* Substitute parameters in the macro body. */
            StrBuf prepared;
            sb_init(&prepared);
            const char *b = m->body;
            while (*b) {
                if (isalpha((unsigned char)*b) || *b == '_') {
                    char idb[MAX_IDENT];
                    read_name(&b, idb, sizeof(idb));
                    int match = -1;
                    for (int k = 0; k < m->param_count; k++) {
                        if (strcmp(idb, m->params[k]) == 0) { match = k; break; }
                    }
                    if (match >= 0 && match < argc && args[match]) sb_puts(&prepared, args[match]);
                    else sb_puts(&prepared, idb);
                } else {
                    sb_putc(&prepared, *b++);
                }
            }
            char *subst = sb_take(&prepared);
            char *red = reduce_meta(subst, 0);
            char *rval = expand_text(red, depth + 1);
            sb_puts(&out, rval);
            for (int i = 0; i < argc; i++) free(args[i]);
            free(subst);
            free(red);
            free(rval);
        } else if (m) {
            char *red = reduce_meta(m->body, 0);
            char *rval = expand_text(red, depth + 1);
            sb_puts(&out, rval);
            free(red);
            free(rval);
        } else {
            sb_puts(&out, ident);
        }
    }
    return sb_take(&out);
}

/* ---------- conditionals: @if(...) { } @elif(...) { } @else { } @endif ---------- */

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
        char nm[MAX_IDENT];
        read_name(e, nm, sizeof(nm));
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
        char nm[MAX_IDENT];
        read_name(e, nm, sizeof(nm));
        LoiMacroDef *m = find_macro(nm);
        if (!m || m->function_like) return 0;
        char buf[MAX_MACRO_BODY];
        snprintf(buf, sizeof(buf), "%s", m->body);
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
    if (!expr || depth > MAX_COND_DEPTH) return 0;
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
static size_t pp_chain(const char *s, size_t i, size_t end, StrBuf *out, int depth) {
    int taken = 0;
    while (i < end) {
        int is_if = lit_is(s + i, end - i, "@if(");
        int is_elif = lit_is(s + i, end - i, "@elif(");
        if (is_if || is_elif) {
            size_t open = is_if ? i + 3 : i + 5;
            size_t cp = find_paren_end(s, open, end);
            char cond[MAX_COND_LEN];
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
    size_t wl = strlen(word);
    StrBuf out;
    sb_init(&out);
    const char *p = text;
    while (*p) {
        int boundary = (p == text) || !(isalnum((unsigned char)p[-1]) || p[-1] == '_');
        if (wl && boundary && (isalpha((unsigned char)*p) || *p == '_') &&
            strncmp(p, word, wl) == 0 && !(isalnum((unsigned char)p[wl]) || p[wl] == '_')) {
            sb_puts(&out, repl);
            p += wl;
        } else {
            sb_putc(&out, *p++);
        }
    }
    return sb_take(&out);
}

/* Split a comma-separated list at top level (respecting parentheses), trimming
   each piece. Returns the number of pieces (at most `max`). */
static int split_top(const char *s, char out[][MAX_PART_LEN], int max) {
    int depth = 0, n = 0;
    const char *start = s;
    for (const char *p = s; ; p++) {
        char c = *p;
        if (c == '(') depth++;
        else if (c == ')') depth--;
        if ((c == ',' && depth == 0) || c == 0) {
            const char *b = start, *e = p;
            while (b < e && isspace((unsigned char)*b)) b++;
            while (e > b && isspace((unsigned char)e[-1])) e--;
            size_t len = (size_t)(e - b);
            if (len > 255) len = 255;
            memcpy(out[n], b, len);
            out[n][len] = 0;
            n++;
            start = p + 1;
            if (c == 0 || n >= max) break;
        }
    }
    return n;
}

/* Undefine the macro named by the text following an `@undef`/`#undef`. */
static void do_undef(const char *rest) {
    rest = skip_ws(rest);
    char name[MAX_IDENT];
    read_name(&rest, name, sizeof(name));
    macro_undef(name);
}

/* Emit a compile-time error (from `@error`/`#error`) and stop. */
static void do_error(const char *msg) {
    fprintf(stderr, "error: %s\n", skip_ws(msg));
    exit(1);
}

/* Handle the `@`-directives that consume a whole line or a brace block:
   `@repeat`/`@for`, the parenthesized `@(define ...)` meta form, and the
   line-based `@define`/`@undef`/`@error`.
   `i` must point at the `@` and be the first non-space token on its line.
   On success, appends to `out`, sets `*next`, and returns 1. Returns 0 when
   the text is not one of these directives (so the caller copies it verbatim). */
static int handle_at(const char *s, size_t i, size_t end, int depth, StrBuf *out, size_t *next) {
    /* --- @repeat(n) { ... }  and  @for(iter, from, to, step) { ... } --- */
    int is_for = lit_is(s + i, end - i, "@for(");
    if (is_for || lit_is(s + i, end - i, "@repeat(")) {
        size_t open = i + (is_for ? 4 : 7);
        size_t cp = find_paren_end(s, open, end);
        if (cp < end) {
            size_t j = cp + 1;
            while (j < end && isspace((unsigned char)s[j])) j++;
            if (j < end && s[j] == '{') {
                size_t be = find_block_end(s, j, end);
                char args[MAX_COND_LEN];
                size_t al = cp - (open + 1);
                if (al >= sizeof(args)) al = sizeof(args) - 1;
                memcpy(args, s + open + 1, al);
                args[al] = 0;
                char body[MAX_BLOCK_LEN];
                size_t bl = be - (j + 1);
                if (bl >= sizeof(body)) bl = sizeof(body) - 1;
                memcpy(body, s + j + 1, bl);
                body[bl] = 0;
                char parts[MAX_LIST_PARTS][MAX_PART_LEN];
                int np = split_top(args, parts, MAX_LIST_PARTS);
                if (is_for) {
                    const char *var = (np >= 1) ? parts[0] : "i";
                    int a  = (np >= 2) ? eval_int(parts[1]) : 0;
                    int b  = (np >= 3) ? eval_int(parts[2]) : 0;
                    int st = (np >= 4) ? eval_int(parts[3]) : 1;
                    if (st == 0) st = 1;
                    for (int k = a; (st > 0) ? (k < b) : (k > b); k += st) {
                        char ks[MAX_NUM_LEN];
                        snprintf(ks, sizeof(ks), "%d", k);
                        char *sub = subst_word(body, var, ks);
                        char *proc = pp_range(sub, 0, strlen(sub), depth + 1);
                        sb_puts(out, proc);
                        free(sub);
                        free(proc);
                    }
                } else {
                    int nrep = (np >= 1) ? eval_int(parts[0]) : 0;
                    for (int k = 0; k < nrep; k++) {
                        char *proc = pp_range(body, 0, strlen(body), depth + 1);
                        sb_puts(out, proc);
                        free(proc);
                    }
                }
                *next = (be < end) ? be + 1 : end;
                return 1;
            }
        }
        return 0;
    }

    /* --- parenthesized meta form: @(define ...) / @(undef ...) / @(error ...) --- */
    if (i + 1 < end && s[i + 1] == '(') {
        size_t cp = find_paren_end(s, i + 1, end);
        if (cp < end) {
            size_t clen = cp - (i + 2);
            char inner[MAX_INLINE_LEN];
            if (clen >= sizeof(inner)) clen = sizeof(inner) - 1;
            memcpy(inner, s + i + 2, clen);
            inner[clen] = 0;
            char *t = skip_ws(inner);
            if (kw_match(t, "define")) register_define(t + 6);
            else if (kw_match(t, "undef")) do_undef(t + 5);
            else if (kw_match(t, "error")) do_error(t + 5);
            /* The directive is meta-only: drop it but keep its newline. */
            size_t le = cp + 1;
            while (le < end && s[le] != '\n') le++;
            if (le < end && s[le] == '\n') { sb_putc(out, '\n'); *next = le + 1; }
            else *next = le;
            return 1;
        }
    }

    /* --- line-based @define / @undef / @error --- */
    if (kw_match(s + i, "@define") || kw_match(s + i, "@undef") || kw_match(s + i, "@error")) {
        size_t kwlen = kw_match(s + i, "@define") ? 7 : 6;
        size_t le = i + kwlen;
        while (le < end && s[le] != '\n') le++;
        char payload[MAX_INLINE_LEN];
        size_t pl = le - (i + kwlen);
        if (pl >= sizeof(payload)) pl = sizeof(payload) - 1;
        memcpy(payload, s + i + kwlen, pl);
        payload[pl] = 0;
        if (s[i + 1] == 'd') register_define(payload);
        else if (s[i + 1] == 'u') do_undef(payload);
        else do_error(payload);
        if (le < end && s[le] == '\n') { sb_putc(out, '\n'); *next = le + 1; }
        else *next = le;
        return 1;
    }
    return 0;
}

/* Core line/conditional/directive processor. Copies [start,end) from `s`,
   evaluating `@if` chains and consuming preprocessor directives. */
static char *pp_range(const char *s, size_t start, size_t end, int depth) {
    StrBuf out;
    sb_init(&out);
    if (depth > MAX_PP_DEPTH) {
        sb_put(&out, s + start, end - start);
        return sb_take(&out);
    }
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

        /* `#...` directives are recognized only at the start of a line. */
        if (c == '#' && line_blank_before(s, i)) {
            size_t le = i;
            while (le < end && s[le] != '\n') le++;
            char dir[MAX_INLINE_LEN];
            size_t dl = le - i;
            if (dl >= sizeof(dir)) dl = sizeof(dir) - 1;
            memcpy(dir, s + i, dl);
            dir[dl] = 0;
            char *dt = skip_ws(dir);
            if (kw_match(dt, "#define")) register_define(dt + 7);
            else if (kw_match(dt, "#undef")) do_undef(dt + 6);
            else if (kw_match(dt, "#error")) do_error(dt + 6);
            /* Strip the directive line, keeping its newline for line numbers. */
            if (le < end && s[le] == '\n') { sb_putc(&out, '\n'); i = le + 1; }
            else i = le;
            continue;
        }

        if (c == '@' && !lit_is(s + i, end - i, "@align") && !lit_is(s + i, end - i, "@pack")) {
            char prev = (i > 0) ? s[i - 1] : '\n';
            int at_start = (i == 0) || isspace((unsigned char)prev) || prev == '}' || prev == '{' || prev == ';';
            /* `at_start` is deliberately permissive so `} @elif(...)` chains work.
               Line-eating directives additionally require a blank line prefix, so
               an `@` inside a comment or string is never treated as a directive. */
            int line_blank = line_blank_before(s, i);
            if (at_start && lit_is(s + i, end - i, "@if(")) {
                size_t olen = out.len;
                size_t next_i = pp_chain(s, i, end, &out, depth);
                /* Preserve line numbers by emitting the newlines the skipped
                   branches contained but that were not copied. */
                size_t expected = count_nl(s, i, next_i);
                size_t got = count_nl(out.buf + olen, 0, out.len - olen);
                for (size_t k = got; k < expected; k++) sb_putc(&out, '\n');
                i = next_i;
                continue;
            }
            if (at_start && lit_is(s + i, end - i, "@endif")) { i += 6; continue; }
            if (at_start && (lit_is(s + i, end - i, "@elif(") || lit_is(s + i, end - i, "@else"))) {
                i += lit_is(s + i, end - i, "@elif(") ? 6 : 5;
                continue;
            }
            if (line_blank) {
                size_t next_i;
                if (handle_at(s, i, end, depth, &out, &next_i)) { i = next_i; continue; }
            }
        }
        sb_putc(&out, c);
        i++;
    }
    return sb_take(&out);
}

/* register a #define from the text following the keyword */
static void register_define(const char *rest) {
    rest = skip_ws(rest);
    char name[MAX_IDENT];
    read_name(&rest, name, sizeof(name));
    int func = 0;
    char params[MAX_MACRO_PARAMS][MAX_IDENT];
    int pc = 0;
    int variadic = 0;
    char va_name[MAX_IDENT];
    va_name[0] = 0;
    rest = skip_ws(rest);
    if (*rest == '(') {
        func = 1;
        rest++;
        while (*rest && *rest != ')') {
            while (*rest == ' ' || *rest == ',') rest++;
            if (*rest == ')') break;
            if (rest[0] == '.' && rest[1] == '.' && rest[2] == '.') {
                rest += 3;
                while (*rest == ' ') rest++;
                read_name(&rest, va_name, sizeof(va_name));
                if (va_name[0] == 0) copy_str(va_name, sizeof(va_name), "args");
                variadic = 1;
                while (*rest && *rest != ')') rest++;
                break;
            }
            char pn[MAX_IDENT];
            read_name(&rest, pn, sizeof(pn));
            if (pn[0] && pc < MAX_MACRO_PARAMS) copy_str(params[pc++], MAX_IDENT, pn);
            while (*rest && *rest != ',' && *rest != ')') rest++;
            if (*rest == ',') rest++;
        }
        if (*rest == ')') rest++;
    }
    char *body = skip_ws(rest);
    char bodybuf[MAX_MACRO_BODY];
    strncpy(bodybuf, body, sizeof(bodybuf) - 1);
    bodybuf[sizeof(bodybuf) - 1] = 0;
    size_t bl = strlen(bodybuf);
    while (bl > 0 && isspace((unsigned char)bodybuf[bl - 1])) { bodybuf[bl - 1] = 0; bl--; }
    if (name[0] && g_nmacro < MAX_MACROS) {
        LoiMacroDef *md = &g_macros[g_nmacro++];
        copy_str(md->name, sizeof(md->name), name);
        if (variadic && pc < MAX_MACRO_PARAMS) copy_str(params[pc++], MAX_IDENT, va_name);
        for (int i = 0; i < pc; i++) copy_str(md->params[i], MAX_IDENT, params[i]);
        md->param_count = pc;
        copy_str(md->body, sizeof(md->body), bodybuf);
        md->function_like = func;
        md->variadic = variadic;
        copy_str(md->va_name, sizeof(md->va_name), va_name);
    } else if (name[0]) {
        fprintf(stderr, "error: too many macros (limit %d); ignoring '#define %s'\n",
                MAX_MACROS, name);
    }
}

/* collect all #define lines so @if(...) can test them */
static char *preprocess_macros(const char *src) {
    if (!src) return NULL;
    /* pp_range evaluates @if/@elif/@else while registering #define/#undef in
       order, so defines inside skipped branches are never registered. */
    char *cond_src = pp_range(src, 0, strlen(src), 0);
    StrBuf out;
    sb_init(&out);
    /* Expand each line separately so a call split across lines is not joined. */
    const char *p = cond_src;
    while (*p) {
        const char *nl = strchr(p, '\n');
        size_t linelen = nl ? (size_t)(nl - p) : strlen(p);
        char line[MAX_INLINE_LEN];
        size_t n = linelen < sizeof(line) ? linelen : sizeof(line) - 1;
        memcpy(line, p, n);
        line[n] = 0;
        char *exp = expand_text(line, 0);
        sb_puts(&out, exp);
        free(exp);
        if (!nl) break;
        sb_putc(&out, '\n');
        p = nl + 1;
    }
    free(cond_src);
    char *expanded = sb_take(&out);
    /* Second conditional pass, for @if introduced by macro expansion. */
    char *final = pp_range(expanded, 0, strlen(expanded), 0);
    free(expanded);
    return final;
}

/* Public entry point: textually preprocess a loi source unit. */
char *loi_preprocess(const char *src, const char *base_dir) {
    if (!src) return NULL;
    include_guard_reset();
    char *included = expand_includes(src, base_dir, 0);
    char *result = preprocess_macros(included ? included : "");
    free(included);
    return result;
}
