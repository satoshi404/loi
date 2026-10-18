/*
 * loi binding generator (see bindgen.h).
 *
 * Pipeline
 *   1. `cc -E -dD gen.h`  -> one translation unit with line markers and the
 *      #define lines, with every #include resolved by the system compiler.
 *   2. Tokenise it and parse the C declarations (typedef / struct / union /
 *      enum / function prototypes). Declarations coming from the headers the
 *      user included ("owned" files: the directory of each directly included
 *      header) are exported; everything else only feeds the type table.
 *   3. A second `cc -E` run expands every object-like macro of the owned
 *      headers; the expansions are folded to integer constants.
 *   4. Emit a loi package: constants, aliases, structs/unions, extern
 *      functions with @attr(link_name("...")).
 *
 * The generator has no libclang dependency, so it works anywhere `cc` does.
 */
#define _GNU_SOURCE
#include <ctype.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <stdarg.h>

#include "bindgen.h"

/* ------------------------------------------------------------------ utils */
static void *xmalloc(size_t n) { void *p = malloc(n ? n : 1); if (!p) { fputs("out of memory\n", stderr); exit(1); } return p; }
static void *xrealloc(void *q, size_t n) { void *p = realloc(q, n ? n : 1); if (!p) { fputs("out of memory\n", stderr); exit(1); } return p; }
static char *xstrdup(const char *s) { size_t n = strlen(s); char *p = xmalloc(n + 1); memcpy(p, s, n + 1); return p; }
static char *xstrndup(const char *s, size_t n) { char *p = xmalloc(n + 1); memcpy(p, s, n); p[n] = 0; return p; }

typedef struct { char *s; size_t len, cap; } Buf;
static void buf_add(Buf *b, const char *s) {
    size_t n = strlen(s);
    if (b->len + n + 1 > b->cap) { b->cap = (b->len + n + 1) * 2; b->s = xrealloc(b->s, b->cap); }
    memcpy(b->s + b->len, s, n + 1);
    b->len += n;
}
static void buf_addf(Buf *b, const char *fmt, ...) __attribute__((format(printf, 2, 3)));
static void buf_addf(Buf *b, const char *fmt, ...) {
    char tmp[4096];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(tmp, sizeof tmp, fmt, ap);
    va_end(ap);
    buf_add(b, tmp);
}

static char *read_all(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = xmalloc((size_t)sz + 1);
    size_t n = fread(buf, 1, (size_t)sz, f);
    buf[n] = 0;
    fclose(f);
    return buf;
}

/* ----------------------------------------------------------------- tokens */
typedef enum { TK_EOF, TK_ID, TK_NUM, TK_STR, TK_CHR, TK_PUNCT } TKind;
typedef struct { TKind k; char *s; int owned; } Tok;
typedef struct { Tok *v; size_t n, cap; } TokVec;

static void tv_push(TokVec *tv, TKind k, const char *s, size_t len, int owned) {
    if (tv->n == tv->cap) { tv->cap = tv->cap ? tv->cap * 2 : 4096; tv->v = xrealloc(tv->v, tv->cap * sizeof(Tok)); }
    tv->v[tv->n].k = k;
    tv->v[tv->n].s = xstrndup(s, len);
    tv->v[tv->n].owned = owned;
    tv->n++;
}

/* Lex one line of preprocessed C into `tv`. */
static void lex_line(const char *p, int owned, TokVec *tv) {
    while (*p) {
        if (isspace((unsigned char)*p)) { p++; continue; }
        const char *s = p;
        if (isalpha((unsigned char)*p) || *p == '_') {
            while (isalnum((unsigned char)*p) || *p == '_') p++;
            tv_push(tv, TK_ID, s, (size_t)(p - s), owned);
        } else if (isdigit((unsigned char)*p) || (*p == '.' && isdigit((unsigned char)p[1]))) {
            while (isalnum((unsigned char)*p) || *p == '.' || *p == '_' ||
                   ((*p == '+' || *p == '-') && (p[-1] == 'e' || p[-1] == 'E' || p[-1] == 'p' || p[-1] == 'P') &&
                    !(s[0] == '0' && (s[1] == 'x' || s[1] == 'X') && (p[-1] == 'e' || p[-1] == 'E'))))
                p++;
            tv_push(tv, TK_NUM, s, (size_t)(p - s), owned);
        } else if (*p == '"' || *p == '\'') {
            char q = *p++;
            while (*p && *p != q) { if (*p == '\\' && p[1]) p++; p++; }
            if (*p) p++;
            tv_push(tv, q == '"' ? TK_STR : TK_CHR, s, (size_t)(p - s), owned);
        } else {
            static const char *two[] = { "...", "->", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", NULL };
            size_t len = 1;
            for (int i = 0; two[i]; i++) {
                size_t l = strlen(two[i]);
                if (strncmp(p, two[i], l) == 0) { len = l; break; }
            }
            p += len;
            tv_push(tv, TK_PUNCT, s, len, owned);
        }
    }
}

/* ------------------------------------------------------------- C type model */
typedef enum {
    K_VOID, K_BOOL, K_CHAR, K_SCHAR, K_UCHAR, K_SHORT, K_USHORT, K_INT, K_UINT,
    K_LONG, K_ULONG, K_LLONG, K_ULLONG, K_FLOAT, K_DOUBLE, K_LDOUBLE, K_INT128,
    K_PTR, K_ARRAY, K_FUNC, K_REC, K_ENUM, K_TYPEDEF, K_UNKNOWN
} Kind;

typedef struct Type Type;
typedef struct Rec Rec;
typedef struct Typedef Typedef;

struct Type {
    Kind k;
    Type *base;             /* pointee / element / return type            */
    long len;               /* array length, -1 when unknown              */
    Rec *rec;
    Typedef *td;
    Type **params; char **pnames; int np; int variadic;
};

typedef struct { char *name; Type *ty; } Field;
struct Rec {
    char *tag;              /* NULL for anonymous                         */
    int is_union, complete, packed, unsupported, owned, used;
    Field *f; int nf, cap;
    char *td_name;          /* first owned typedef that names this record */
    char *loi_name;         /* final name in the generated package        */
    int state;              /* 0 = not emitted, 1 = emitting, 2 = done    */
};
struct Typedef { char *name; Type *ty; int owned; char *loi_name; int alias_checked, alias_ok; };

typedef struct { char *name; int64_t val; int owned; } EConst;
typedef struct { char *name; char *body; int owned; } Macro;
typedef struct { char *name; Type *ret; Type **params; char **pnames; int np; int variadic; int owned; } Func;

static Rec **recs; static int nrecs, caprecs;
static Typedef **tds; static int ntds, captds;
static EConst *ecs; static int necs, capecs;
static Macro *macros; static int nmacros, capmacros;
static Func *funcs; static int nfuncs, capfuncs;

static Type *mk(Kind k) { Type *t = xmalloc(sizeof *t); memset(t, 0, sizeof *t); t->k = k; t->len = -1; return t; }
static Type *mk_ptr(Type *b) { Type *t = mk(K_PTR); t->base = b; return t; }

static Rec *rec_get(const char *tag, int is_union, int owned) {
    if (tag)
        for (int i = 0; i < nrecs; i++)
            if (recs[i]->tag && recs[i]->is_union == is_union && strcmp(recs[i]->tag, tag) == 0) return recs[i];
    Rec *r = xmalloc(sizeof *r);
    memset(r, 0, sizeof *r);
    r->tag = tag ? xstrdup(tag) : NULL;
    r->is_union = is_union;
    r->owned = owned;
    if (nrecs == caprecs) { caprecs = caprecs ? caprecs * 2 : 256; recs = xrealloc(recs, (size_t)caprecs * sizeof *recs); }
    recs[nrecs++] = r;
    return r;
}

static Typedef *td_find(const char *name) {
    for (int i = ntds - 1; i >= 0; i--)
        if (strcmp(tds[i]->name, name) == 0) return tds[i];
    return NULL;
}
static void td_add(const char *name, Type *ty, int owned) {
    Typedef *t = xmalloc(sizeof *t);
    memset(t, 0, sizeof *t);
    t->name = xstrdup(name);
    t->ty = ty;
    t->owned = owned;
    if (ntds == captds) { captds = captds ? captds * 2 : 512; tds = xrealloc(tds, (size_t)captds * sizeof *tds); }
    tds[ntds++] = t;
}

static EConst *ec_find(const char *name) {
    for (int i = necs - 1; i >= 0; i--)
        if (strcmp(ecs[i].name, name) == 0) return &ecs[i];
    return NULL;
}
static void ec_add(const char *name, int64_t v, int owned) {
    if (necs == capecs) { capecs = capecs ? capecs * 2 : 1024; ecs = xrealloc(ecs, (size_t)capecs * sizeof *ecs); }
    ecs[necs].name = xstrdup(name);
    ecs[necs].val = v;
    ecs[necs].owned = owned;
    necs++;
}

static Type *resolve(Type *t) {
    for (int i = 0; t && t->k == K_TYPEDEF && i < 64; i++) t = t->td->ty;
    return t;
}

/* ---------------------------------------------------- constant expressions */
typedef struct { const Tok *t; size_t n, i; int err; } Ev;

static int is_type_word(const char *s) {
    static const char *w[] = { "int", "unsigned", "signed", "long", "short", "char", "void", "float", "double", "_Bool", NULL };
    for (int i = 0; w[i]; i++) if (strcmp(s, w[i]) == 0) return 1;
    return 0;
}

static int64_t parse_cint(const char *s, int *err) {
    char *end;
    int64_t v;
    if (s[0] == '0' && (s[1] == 'b' || s[1] == 'B')) v = (int64_t)strtoull(s + 2, &end, 2);
    else v = (int64_t)strtoull(s, &end, 0);
    while (*end == 'u' || *end == 'U' || *end == 'l' || *end == 'L') end++;
    if (*end) *err = 1;
    return v;
}

static int64_t char_value(const char *s) {
    if (s[1] != '\\') return (unsigned char)s[1];
    switch (s[2]) {
        case 'n': return 10; case 't': return 9; case 'r': return 13; case 'a': return 7;
        case 'b': return 8; case 'f': return 12; case 'v': return 11; case '0': case '1': case '2': case '3':
        case '4': case '5': case '6': case '7': return (int64_t)strtol(s + 2, NULL, 8);
        case 'x': return (int64_t)strtol(s + 3, NULL, 16);
        default: return (unsigned char)s[2];
    }
}

static int64_t ev_cond(Ev *e);
static const Tok *ev_peek(Ev *e) { return e->i < e->n ? &e->t[e->i] : NULL; }
static int ev_punct(Ev *e, const char *p) {
    const Tok *t = ev_peek(e);
    if (t && t->k == TK_PUNCT && strcmp(t->s, p) == 0) { e->i++; return 1; }
    return 0;
}

static int64_t ev_unary(Ev *e) {
    const Tok *t = ev_peek(e);
    if (!t) { e->err = 1; return 0; }
    if (ev_punct(e, "-")) return -ev_unary(e);
    if (ev_punct(e, "+")) return ev_unary(e);
    if (ev_punct(e, "~")) return ~ev_unary(e);
    if (ev_punct(e, "!")) return !ev_unary(e);
    if (t->k == TK_NUM) { e->i++; return parse_cint(t->s, &e->err); }
    if (t->k == TK_CHR) { e->i++; return char_value(t->s); }
    if (t->k == TK_ID && !strcmp(t->s, "sizeof")) {   /* sizeof(type): pointers and basic scalars */
        e->i++;
        if (!ev_punct(e, "(")) { e->err = 1; return 0; }
        int64_t size = 0, depth = 1, ptr = 0;
        while (e->i < e->n && depth > 0) {
            const Tok *u = &e->t[e->i++];
            if (u->k == TK_PUNCT && !strcmp(u->s, "(")) depth++;
            else if (u->k == TK_PUNCT && !strcmp(u->s, ")")) depth--;
            else if (u->k == TK_PUNCT && !strcmp(u->s, "*")) ptr = 1;
            else if (u->k == TK_ID) {
                if (!strcmp(u->s, "char")) size = 1;
                else if (!strcmp(u->s, "short")) size = 2;
                else if (!strcmp(u->s, "int") || !strcmp(u->s, "float")) { if (!size) size = 4; }
                else if (!strcmp(u->s, "long") || !strcmp(u->s, "double")) size = 8;
            }
        }
        if (ptr) return 8;
        if (!size) e->err = 1;
        return size;
    }
    if (t->k == TK_ID) {
        EConst *c = ec_find(t->s);
        e->i++;
        if (c) return c->val;
        e->err = 1;
        return 0;
    }
    if (ev_punct(e, "(")) {
        const Tok *n = ev_peek(e);
        int cast = n && n->k == TK_ID && !ec_find(n->s) && (is_type_word(n->s) || td_find(n->s));
        if (cast) {   /* (Type) operand: the cast itself is ignored */
            while (e->i < e->n && !(e->t[e->i].k == TK_PUNCT && strcmp(e->t[e->i].s, ")") == 0)) e->i++;
            e->i++;
            return ev_unary(e);
        }
        int64_t v = ev_cond(e);
        if (!ev_punct(e, ")")) e->err = 1;
        return v;
    }
    e->err = 1;
    return 0;
}

static int prec_of(const char *op) {
    static const struct { const char *s; int p; } T[] = {
        {"*", 10}, {"/", 10}, {"%", 10}, {"+", 9}, {"-", 9}, {"<<", 8}, {">>", 8},
        {"<", 7}, {">", 7}, {"<=", 7}, {">=", 7}, {"==", 6}, {"!=", 6},
        {"&", 5}, {"^", 4}, {"|", 3}, {"&&", 2}, {"||", 1}, {NULL, 0} };
    for (int i = 0; T[i].s; i++) if (strcmp(op, T[i].s) == 0) return T[i].p;
    return 0;
}

static int64_t ev_bin(Ev *e, int minp) {
    int64_t l = ev_unary(e);
    for (;;) {
        const Tok *t = ev_peek(e);
        if (!t || t->k != TK_PUNCT) break;
        int p = prec_of(t->s);
        if (!p || p < minp) break;
        char op[4];
        snprintf(op, sizeof op, "%s", t->s);
        e->i++;
        int64_t r = ev_bin(e, p + 1);
        if (!strcmp(op, "*")) l *= r;
        else if (!strcmp(op, "/")) { if (!r) { e->err = 1; return 0; } l /= r; }
        else if (!strcmp(op, "%")) { if (!r) { e->err = 1; return 0; } l %= r; }
        else if (!strcmp(op, "+")) l += r;
        else if (!strcmp(op, "-")) l -= r;
        else if (!strcmp(op, "<<")) l = (r >= 0 && r < 64) ? (int64_t)((uint64_t)l << r) : 0;
        else if (!strcmp(op, ">>")) l = (r >= 0 && r < 64) ? l >> r : 0;
        else if (!strcmp(op, "<")) l = l < r;
        else if (!strcmp(op, ">")) l = l > r;
        else if (!strcmp(op, "<=")) l = l <= r;
        else if (!strcmp(op, ">=")) l = l >= r;
        else if (!strcmp(op, "==")) l = l == r;
        else if (!strcmp(op, "!=")) l = l != r;
        else if (!strcmp(op, "&")) l &= r;
        else if (!strcmp(op, "^")) l ^= r;
        else if (!strcmp(op, "|")) l |= r;
        else if (!strcmp(op, "&&")) l = l && r;
        else if (!strcmp(op, "||")) l = l || r;
    }
    return l;
}

static int64_t ev_cond(Ev *e) {
    int64_t c = ev_bin(e, 1);
    if (ev_punct(e, "?")) {
        int64_t a = ev_cond(e);
        if (!ev_punct(e, ":")) { e->err = 1; return 0; }
        int64_t b = ev_cond(e);
        return c ? a : b;
    }
    return c;
}

/* Evaluate tokens [t, t+n) as an integer constant expression. */
static int eval_const(const Tok *t, size_t n, int64_t *out) {
    Ev e = { t, n, 0, 0 };
    *out = ev_cond(&e);
    return !e.err && e.i == e.n;
}

/* ------------------------------------------------------------ the parser */
static TokVec T;          /* filtered token stream of the whole translation unit */
static size_t pos;

static const Tok *cur(void) { static Tok eof = { TK_EOF, (char *)"", 0 }; return pos < T.n ? &T.v[pos] : &eof; }
static const Tok *peekn(size_t k) { static Tok eof = { TK_EOF, (char *)"", 0 }; return pos + k < T.n ? &T.v[pos + k] : &eof; }
static int at_punct(const char *p) { const Tok *t = cur(); return t->k == TK_PUNCT && strcmp(t->s, p) == 0; }
static int at_id(const char *p) { const Tok *t = cur(); return t->k == TK_ID && strcmp(t->s, p) == 0; }
static int accept_punct(const char *p) { if (at_punct(p)) { pos++; return 1; } return 0; }

/* Drop attributes/qualifiers the generator does not model. `packed` is kept
   as a marker token so struct layout stays faithful. */
static int drop_word(const char *s) {
    static const char *w[] = { "__extension__", "__restrict", "__restrict__", "restrict", "__inline", "__inline__",
        "inline", "_Noreturn", "__const", "const", "volatile", "__volatile__", "register", "_Thread_local",
        "__thread", "_Complex", "__complex__", NULL };
    for (int i = 0; w[i]; i++) if (strcmp(s, w[i]) == 0) return 1;
    return 0;
}

static void filter_tokens(const TokVec *raw) {
    for (size_t i = 0; i < raw->n; i++) {
        const Tok *t = &raw->v[i];
        if (t->k == TK_ID) {
            if (drop_word(t->s)) continue;
            int attr = !strcmp(t->s, "__attribute__") || !strcmp(t->s, "__attribute");
            int asmw = !strcmp(t->s, "__asm__") || !strcmp(t->s, "__asm") || !strcmp(t->s, "asm") ||
                       !strcmp(t->s, "__declspec") || !strcmp(t->s, "_Alignas");
            if (attr || asmw) {
                int packed = 0, depth = 0;
                size_t j = i + 1;
                if (j < raw->n && raw->v[j].k == TK_PUNCT && !strcmp(raw->v[j].s, "(")) {
                    for (; j < raw->n; j++) {
                        const Tok *u = &raw->v[j];
                        if (u->k == TK_PUNCT && !strcmp(u->s, "(")) depth++;
                        else if (u->k == TK_PUNCT && !strcmp(u->s, ")")) { if (--depth == 0) break; }
                        else if (u->k == TK_ID && (!strcmp(u->s, "packed") || !strcmp(u->s, "__packed__"))) packed = 1;
                    }
                }
                if (attr && packed) tv_push(&T, TK_ID, "__LOI_PACKED__", 14, t->owned);
                i = j;
                continue;
            }
            if (!strcmp(t->s, "__signed__") || !strcmp(t->s, "__signed")) { tv_push(&T, TK_ID, "signed", 6, t->owned); continue; }
        }
        tv_push(&T, t->k, t->s, strlen(t->s), t->owned);
    }
}

static Type *parse_declarator(Type *base, char **name);
static Type *parse_specifiers(int *is_typedef, int *is_static, int *is_extern);

static void skip_group(const char *open, const char *close) {   /* at `open`; leaves pos after the match */
    int depth = 0;
    while (cur()->k != TK_EOF) {
        if (at_punct(open)) depth++;
        else if (at_punct(close)) { if (--depth == 0) { pos++; return; } }
        pos++;
    }
}

/* skip an initializer / expression until `,` or `;` at nesting depth 0 */
static void skip_init(void) {
    int d = 0;
    while (cur()->k != TK_EOF) {
        if (at_punct("(") || at_punct("[") || at_punct("{")) d++;
        else if (at_punct(")") || at_punct("]") || at_punct("}")) { if (d == 0) return; d--; }
        else if (d == 0 && (at_punct(",") || at_punct(";"))) return;
        pos++;
    }
}

static int is_typename_tok(const Tok *t) {
    if (t->k != TK_ID) return 0;
    return is_type_word(t->s) || !strcmp(t->s, "struct") || !strcmp(t->s, "union") || !strcmp(t->s, "enum") ||
           !strcmp(t->s, "__int128") || (td_find(t->s) != NULL);
}

static Type *parse_enum_spec(int owned) {
    char *tag = NULL;
    if (cur()->k == TK_ID && strcmp(cur()->s, "__LOI_PACKED__") != 0) { tag = xstrdup(cur()->s); pos++; }
    Type *t = mk(K_ENUM);
    if (at_punct("{")) {
        pos++;
        int64_t next = 0;
        while (cur()->k != TK_EOF && !at_punct("}")) {
            if (cur()->k != TK_ID) { pos++; continue; }
            char *name = xstrdup(cur()->s);
            pos++;
            if (accept_punct("=")) {
                size_t s = pos;
                int d = 0;
                while (cur()->k != TK_EOF) {
                    if (at_punct("(")) d++;
                    else if (at_punct(")")) d--;
                    else if (d == 0 && (at_punct(",") || at_punct("}"))) break;
                    pos++;
                }
                int64_t v;
                if (eval_const(&T.v[s], pos - s, &v)) next = v;
            }
            ec_add(name, next, owned);
            next++;
            free(name);
            accept_punct(",");
        }
        accept_punct("}");
    }
    free(tag);
    return t;
}

static Type *parse_rec_spec(int is_union, int owned) {
    int packed = 0;
    while (at_id("__LOI_PACKED__")) { packed = 1; pos++; }
    char *tag = NULL;
    if (cur()->k == TK_ID) { tag = xstrdup(cur()->s); pos++; }
    while (at_id("__LOI_PACKED__")) { packed = 1; pos++; }
    Rec *r = rec_get(tag, is_union, owned);
    if (owned) r->owned = 1;
    free(tag);
    if (at_punct("{") && !r->complete) {
        pos++;
        int anon = 0;
        while (cur()->k != TK_EOF && !at_punct("}")) {
            int td = 0, st = 0, ex = 0;
            Type *base = parse_specifiers(&td, &st, &ex);
            if (!base) { skip_init(); accept_punct(";"); r->unsupported = 1; continue; }
            if (at_punct(";")) {   /* anonymous struct/union member */
                char nm[32];
                snprintf(nm, sizeof nm, "_anon%d", anon++);
                if (r->nf == r->cap) { r->cap = r->cap ? r->cap * 2 : 8; r->f = xrealloc(r->f, (size_t)r->cap * sizeof(Field)); }
                r->f[r->nf].name = xstrdup(nm);
                r->f[r->nf].ty = base;
                r->nf++;
                pos++;
                continue;
            }
            for (;;) {
                char *name = NULL;
                Type *ft = parse_declarator(base, &name);
                if (accept_punct(":")) {   /* bit-field: layout not modelled */
                    r->unsupported = 1;
                    size_t d = 0;
                    while (cur()->k != TK_EOF && !at_punct(",") && !at_punct(";") && !at_punct("}")) { pos++; d++; }
                    (void)d;
                }
                if (ft && name) {
                    if (r->nf == r->cap) { r->cap = r->cap ? r->cap * 2 : 8; r->f = xrealloc(r->f, (size_t)r->cap * sizeof(Field)); }
                    r->f[r->nf].name = name;
                    r->f[r->nf].ty = ft;
                    r->nf++;
                } else {
                    free(name);
                    if (!ft) r->unsupported = 1;
                }
                if (!accept_punct(",")) break;
            }
            if (!accept_punct(";")) { skip_init(); accept_punct(";"); }
        }
        accept_punct("}");
        r->complete = 1;
        while (at_id("__LOI_PACKED__")) { packed = 1; pos++; }
        if (packed) r->packed = 1;
    }
    Type *t = mk(K_REC);
    t->rec = r;
    return t;
}

/* decl-specifiers: storage class + qualifiers + one type. NULL on failure. */
static Type *parse_specifiers(int *is_typedef, int *is_static, int *is_extern) {
    int seen = 0, nlong = 0, sgn = 0, have_char = 0, have_short = 0, have_int = 0, have_float = 0,
        have_double = 0, have_void = 0, have_bool = 0, have_128 = 0;
    Type *named = NULL;
    for (;;) {
        const Tok *t = cur();
        if (t->k != TK_ID) break;
        const char *s = t->s;
        if (!strcmp(s, "typedef")) { *is_typedef = 1; pos++; }
        else if (!strcmp(s, "extern")) { *is_extern = 1; pos++; }
        else if (!strcmp(s, "static")) { *is_static = 1; pos++; }
        else if (!strcmp(s, "auto") || !strcmp(s, "__LOI_PACKED__")) pos++;
        else if (!strcmp(s, "void")) { have_void = 1; seen = 1; pos++; }
        else if (!strcmp(s, "char")) { have_char = 1; seen = 1; pos++; }
        else if (!strcmp(s, "short")) { have_short = 1; seen = 1; pos++; }
        else if (!strcmp(s, "int")) { have_int = 1; seen = 1; pos++; }
        else if (!strcmp(s, "long")) { nlong++; seen = 1; pos++; }
        else if (!strcmp(s, "float")) { have_float = 1; seen = 1; pos++; }
        else if (!strcmp(s, "double")) { have_double = 1; seen = 1; pos++; }
        else if (!strcmp(s, "signed")) { sgn = -1; seen = 1; pos++; }
        else if (!strcmp(s, "unsigned")) { sgn = 1; seen = 1; pos++; }
        else if (!strcmp(s, "_Bool")) { have_bool = 1; seen = 1; pos++; }
        else if (!strcmp(s, "__int128")) { have_128 = 1; seen = 1; pos++; }
        else if (!strcmp(s, "struct") || !strcmp(s, "union")) {
            int u = s[0] == 'u';
            int owned = t->owned;
            pos++;
            named = parse_rec_spec(u, owned);
            seen = 1;
        } else if (!strcmp(s, "enum")) {
            int owned = t->owned;
            pos++;
            named = parse_enum_spec(owned);
            seen = 1;
        } else if (!seen) {
            Typedef *td = td_find(s);
            if (td) { named = mk(K_TYPEDEF); named->td = td; }
            else named = mk(K_UNKNOWN);   /* e.g. __builtin_va_list */
            seen = 1;
            pos++;
        } else break;
    }
    if (!seen) return NULL;
    if (named) return named;
    if (have_void) return mk(K_VOID);
    if (have_bool) return mk(K_BOOL);
    if (have_128) return mk(K_INT128);
    if (have_float) return mk(K_FLOAT);
    if (have_double) return mk(nlong ? K_LDOUBLE : K_DOUBLE);
    if (have_char) return mk(sgn > 0 ? K_UCHAR : (sgn < 0 ? K_SCHAR : K_CHAR));
    if (have_short) return mk(sgn > 0 ? K_USHORT : K_SHORT);
    if (nlong >= 2) return mk(sgn > 0 ? K_ULLONG : K_LLONG);
    if (nlong == 1) return mk(sgn > 0 ? K_ULONG : K_LONG);
    (void)have_int;
    return mk(sgn > 0 ? K_UINT : K_INT);
}

static Type *parse_params_into(Type *fn) {
    /* at the token after '(' */
    size_t cap = 8;
    fn->params = xmalloc(cap * sizeof(Type *));
    fn->pnames = xmalloc(cap * sizeof(char *));
    fn->np = 0;
    if (at_punct(")")) { pos++; return fn; }
    if (at_id("void") && peekn(1)->k == TK_PUNCT && !strcmp(peekn(1)->s, ")")) { pos += 2; return fn; }
    for (;;) {
        if (accept_punct("...")) { fn->variadic = 1; break; }
        int td = 0, st = 0, ex = 0;
        Type *base = parse_specifiers(&td, &st, &ex);
        if (!base) return NULL;
        char *name = NULL;
        Type *pt = parse_declarator(base, &name);
        if (!pt) return NULL;
        if (pt->k == K_ARRAY) pt = mk_ptr(pt->base);
        else if (pt->k == K_FUNC) pt = mk_ptr(pt);
        if ((size_t)fn->np == cap) { cap *= 2; fn->params = xrealloc(fn->params, cap * sizeof(Type *)); fn->pnames = xrealloc(fn->pnames, cap * sizeof(char *)); }
        fn->params[fn->np] = pt;
        fn->pnames[fn->np] = name;
        fn->np++;
        if (!accept_punct(",")) break;
    }
    if (!accept_punct(")")) return NULL;
    return fn;
}

static int nested_ahead(void) {   /* at '(' inside a declarator */
    const Tok *n = peekn(1);
    if (n->k == TK_PUNCT && (!strcmp(n->s, "*") || !strcmp(n->s, "(") || !strcmp(n->s, "^"))) return 1;
    if (n->k == TK_ID && !is_typename_tok(n)) return 1;
    return 0;
}

/* array / function suffixes, outermost first */
static Type *parse_suffixes(Type *base) {
    if (at_punct("[")) {
        pos++;
        long len = -1;
        size_t s = pos;
        while (cur()->k != TK_EOF && !at_punct("]")) pos++;
        int64_t v;
        if (pos > s && eval_const(&T.v[s], pos - s, &v)) len = (long)v;
        if (!accept_punct("]")) return NULL;
        Type *inner = parse_suffixes(base);
        if (!inner) return NULL;
        Type *a = mk(K_ARRAY);
        a->base = inner;
        a->len = len;
        return a;
    }
    if (at_punct("(")) {
        pos++;
        Type *fn = mk(K_FUNC);
        if (!parse_params_into(fn)) return NULL;
        Type *ret = parse_suffixes(base);
        if (!ret) return NULL;
        fn->base = ret;
        return fn;
    }
    return base;
}

static Type *parse_declarator(Type *base, char **name) {
    while (at_punct("*")) { pos++; base = mk_ptr(base); }
    if (at_punct("(") && nested_ahead()) {
        pos++;
        size_t inner = pos;
        pos--;
        skip_group("(", ")");
        Type *outer = parse_suffixes(base);
        if (!outer) return NULL;
        size_t end = pos;
        pos = inner;
        Type *t = parse_declarator(outer, name);
        if (!t || !accept_punct(")")) return NULL;
        pos = end;
        return t;
    }
    if (cur()->k == TK_ID && !is_typename_tok(cur())) { *name = xstrdup(cur()->s); pos++; }
    return parse_suffixes(base);
}

static void func_add(const char *name, Type *fn, int owned) {
    for (int i = 0; i < nfuncs; i++) if (!strcmp(funcs[i].name, name)) return;
    if (nfuncs == capfuncs) { capfuncs = capfuncs ? capfuncs * 2 : 512; funcs = xrealloc(funcs, (size_t)capfuncs * sizeof *funcs); }
    funcs[nfuncs].name = xstrdup(name);
    funcs[nfuncs].ret = fn->base;
    funcs[nfuncs].params = fn->params;
    funcs[nfuncs].pnames = fn->pnames;
    funcs[nfuncs].np = fn->np;
    funcs[nfuncs].variadic = fn->variadic;
    funcs[nfuncs].owned = owned;
    nfuncs++;
}

static int parse_external_decl(void) {
    if (accept_punct(";")) return 1;
    if (at_id("_Static_assert")) { pos++; skip_group("(", ")"); accept_punct(";"); return 1; }
    int owned = cur()->owned;
    int td = 0, st = 0, ex = 0;
    Type *base = parse_specifiers(&td, &st, &ex);
    if (!base) return 0;
    if (accept_punct(";")) return 1;
    for (;;) {
        char *name = NULL;
        Type *t = parse_declarator(base, &name);
        if (!t) return 0;
        if (td && name) {
            td_add(name, t, owned);
            Type *r = t;
            if (r->k == K_REC && r->rec && !r->rec->tag && !r->rec->td_name) r->rec->td_name = xstrdup(name);
            else if (r->k == K_REC && r->rec && owned && !r->rec->td_name) r->rec->td_name = xstrdup(name);
        } else if (t->k == K_FUNC && name) {
            if (at_punct("{")) { skip_group("{", "}"); free(name); return 1; }
            if (!st) func_add(name, t, owned);
        } else {
            if (accept_punct("=")) skip_init();
        }
        if (!accept_punct(",")) break;
    }
    return accept_punct(";");
}

static void skip_broken_decl(size_t start) {
    pos = start;
    int d = 0;
    while (cur()->k != TK_EOF) {
        if (at_punct("(") || at_punct("[") || at_punct("{")) d++;
        else if (at_punct(")") || at_punct("]")) { if (d) d--; }
        else if (at_punct("}")) {
            if (d) d--;
            if (d == 0) { pos++; accept_punct(";"); return; }
        } else if (d == 0 && at_punct(";")) { pos++; return; }
        pos++;
    }
}

static void parse_translation_unit(void) {
    pos = 0;
    while (cur()->k != TK_EOF) {
        size_t start = pos;
        if (!parse_external_decl()) skip_broken_decl(start);
        if (pos == start) pos++;   /* guarantee progress */
    }
}

/* ---------------------------------------------------------- owned headers */
static char **seen_files; static int nseen, capseen;
static char **owned_dirs; static int nowned_dirs, capowned_dirs;
static char **owned_files; static int nowned_files, capowned_files;

static void add_str(char ***v, int *n, int *cap, const char *s) {
    for (int i = 0; i < *n; i++) if (!strcmp((*v)[i], s)) return;
    if (*n == *cap) { *cap = *cap ? *cap * 2 : 64; *v = xrealloc(*v, (size_t)*cap * sizeof(char *)); }
    (*v)[(*n)++] = xstrdup(s);
}

static int ends_with(const char *s, const char *suf) {
    size_t a = strlen(s), b = strlen(suf);
    return a >= b && !strcmp(s + a - b, suf);
}

static int generic_dir(const char *dir) {
    const char *slash = strrchr(dir, '/');
    const char *base = slash ? slash + 1 : dir;
    return !strcmp(base, "include") || ends_with(dir, "-linux-gnu") || !strcmp(dir, "") || !strcmp(dir, ".");
}

static int is_owned_file(const char *f) {
    for (int i = 0; i < nowned_files; i++) if (!strcmp(owned_files[i], f)) return 1;
    for (int i = 0; i < nowned_dirs; i++) {
        size_t n = strlen(owned_dirs[i]);
        if (!strncmp(f, owned_dirs[i], n) && f[n] == '/') return 1;
    }
    return 0;
}

/* Headers of the C standard library and POSIX basics are inputs the library
   depends on, never part of the API being bound. */
static int is_std_header(const char *name) {
    static const char *std[] = { "stdint.h", "stddef.h", "stdbool.h", "stdlib.h", "string.h", "stdio.h", "stdarg.h",
        "limits.h", "float.h", "math.h", "ctype.h", "time.h", "errno.h", "inttypes.h", "wchar.h", "wctype.h",
        "assert.h", "signal.h", "setjmp.h", "locale.h", "stdatomic.h", "stdalign.h", "stdnoreturn.h",
        "unistd.h", "sys/types.h", "sys/stat.h", "sys/time.h", "fcntl.h", "pthread.h", NULL };
    for (int i = 0; std[i]; i++) if (!strcmp(name, std[i])) return 1;
    return 0;
}

/* `#include <X>` / `#include "X"` lines of gen.h decide which directories belong to the library. */
static void compute_owned(const char *header_text, const char *header_path) {
    add_str(&owned_files, &nowned_files, &capowned_files, header_path);
    for (const char *p = header_text; p && *p; ) {
        const char *eol = strchr(p, '\n');
        size_t len = eol ? (size_t)(eol - p) : strlen(p);
        char line[1024];
        if (len >= sizeof line) len = sizeof line - 1;
        memcpy(line, p, len);
        line[len] = 0;
        p = eol ? eol + 1 : NULL;
        char *q = line;
        while (isspace((unsigned char)*q)) q++;
        if (*q != '#') continue;
        q++;
        while (isspace((unsigned char)*q)) q++;
        if (strncmp(q, "include", 7) != 0) continue;
        q += 7;
        while (isspace((unsigned char)*q)) q++;
        char close = *q == '<' ? '>' : (*q == '"' ? '"' : 0);
        if (!close) continue;
        q++;
        char *e = strchr(q, close);
        if (!e) continue;
        *e = 0;
        if (is_std_header(q)) continue;
        for (int i = 0; i < nseen; i++) {
            const char *f = seen_files[i];
            int match = !strcmp(f, q);
            if (!match) {
                size_t fl = strlen(f), ql = strlen(q);
                match = fl > ql && f[fl - ql - 1] == '/' && !strcmp(f + fl - ql, q);
            }
            if (!match) continue;
            char dir[PATH_MAX];
            snprintf(dir, sizeof dir, "%s", f);
            char *s = strrchr(dir, '/');
            if (s) *s = 0; else dir[0] = 0;
            if (generic_dir(dir)) add_str(&owned_files, &nowned_files, &capowned_files, f);
            else add_str(&owned_dirs, &nowned_dirs, &capowned_dirs, dir);
        }
    }
}

/* parse `# 12 "file" ...` -> file (NULL if not a line marker) */
static char *marker_file(const char *line) {
    const char *p = line + 1;
    while (*p == ' ') p++;
    if (!isdigit((unsigned char)*p)) return NULL;
    while (isdigit((unsigned char)*p)) p++;
    while (*p == ' ') p++;
    if (*p != '"') return NULL;
    p++;
    const char *e = p;
    while (*e && *e != '"') { if (*e == '\\' && e[1]) e++; e++; }
    return xstrndup(p, (size_t)(e - p));
}

static void add_macro(const char *name, const char *body, int owned) {
    if (nmacros == capmacros) { capmacros = capmacros ? capmacros * 2 : 1024; macros = xrealloc(macros, (size_t)capmacros * sizeof *macros); }
    macros[nmacros].name = xstrdup(name);
    macros[nmacros].body = xstrdup(body);
    macros[nmacros].owned = owned;
    nmacros++;
}

static void scan_unit(char *text, TokVec *raw, int collect_files) {
    int owned = 0;
    char *file = NULL;
    for (char *p = text; p && *p; ) {
        char *eol = strchr(p, '\n');
        if (eol) *eol = 0;
        char *q = p;
        while (*q == ' ' || *q == '\t') q++;
        if (*q == '#') {
            char *f = marker_file(q);
            if (f) {
                if (collect_files) add_str(&seen_files, &nseen, &capseen, f);
                else { free(file); file = f; owned = is_owned_file(f); f = NULL; }
                free(f);
            } else if (!collect_files) {
                char *d = q + 1;
                while (*d == ' ') d++;
                int def = !strncmp(d, "define", 6) && isspace((unsigned char)d[6]);
                int und = !strncmp(d, "undef", 5) && isspace((unsigned char)d[5]);
                if (def || und) {
                    d += def ? 6 : 5;
                    while (isspace((unsigned char)*d)) d++;
                    char *n = d;
                    while (isalnum((unsigned char)*d) || *d == '_') d++;
                    char *name = xstrndup(n, (size_t)(d - n));
                    if (und) {
                        for (int i = 0; i < nmacros; i++) if (macros[i].name && !strcmp(macros[i].name, name)) { free(macros[i].name); macros[i].name = NULL; }
                    } else if (*d != '(') {   /* object-like only */
                        while (isspace((unsigned char)*d)) d++;
                        add_macro(name, d, owned);
                    }
                    free(name);
                }
            }
        } else if (!collect_files) {
            lex_line(q, owned, raw);
        }
        p = eol ? eol + 1 : NULL;
    }
    free(file);
}

/* ------------------------------------------------------------- name mangling */
static char g_prefix[64];
static char **used_names; static int nused, capused;

static int is_loi_keyword(const char *s) {
    static const char *kw[] = { "package", "main", "return", "if", "else", "while", "switch", "case", "default", "for",
        "fn", "let", "const", "true", "false", "null", "break", "continue", "requires", "ensures", "region", "effect",
        "struct", "union", "enum", "type", "extern", "asm", "int", "float", "double", "do", "char", "bool", "void",
        "byte", "long", "short", "unsigned", "signed", "static", "volatile", "inline", "sizeof", "alignof", NULL };
    for (int i = 0; kw[i]; i++) if (!strcmp(s, kw[i])) return 1;
    return 0;
}

static int name_used(const char *n) { for (int i = 0; i < nused; i++) if (!strcmp(used_names[i], n)) return 1; return 0; }

static char *strip_prefix_name(const char *c) {
    size_t n = strlen(g_prefix);
    if (n && !strncmp(c, g_prefix, n) && c[n] && !isdigit((unsigned char)c[n])) return xstrdup(c + n);
    return xstrdup(c);
}

/* package-level name: prefix stripped, keyword-safe, unique */
static char *claim_name(const char *cname) {
    char *base = strip_prefix_name(cname);
    char buf[256];
    snprintf(buf, sizeof buf, "%s", base);
    free(base);
    while (is_loi_keyword(buf) || name_used(buf)) strncat(buf, "_", sizeof buf - strlen(buf) - 1);
    add_str(&used_names, &nused, &capused, buf);
    return xstrdup(buf);
}

static char *safe_ident(const char *s) {   /* field / parameter names */
    char buf[256];
    snprintf(buf, sizeof buf, "%s", s);
    while (is_loi_keyword(buf)) strncat(buf, "_", sizeof buf - strlen(buf) - 1);
    return xstrdup(buf);
}

/* --------------------------------------------------------------- type mapping */
static const char *prim_name(Kind k) {
    switch (k) {
        case K_VOID: return "void";
        case K_BOOL: return "bool";
        case K_CHAR: case K_SCHAR: return "char";
        case K_UCHAR: return "unsigned char";
        case K_SHORT: return "short";
        case K_USHORT: return "unsigned short";
        case K_INT: case K_ENUM: return "int";
        case K_UINT: return "unsigned int";
        case K_LONG: case K_LLONG: return "long";
        case K_ULONG: case K_ULLONG: return "unsigned long";
        case K_FLOAT: return "float";
        case K_DOUBLE: return "double";
        default: return NULL;
    }
}

static char *map_type(Type *t, const char *hint);

static int alias_ok(Typedef *td) {
    if (!td->owned) return 0;
    if (td->alias_checked) return td->alias_ok;
    td->alias_checked = 1;
    Type *r = resolve(td->ty);
    if (!r || r->k == K_REC || r->k == K_ARRAY || r->k == K_UNKNOWN || r->k == K_INT128 || r->k == K_LDOUBLE || r->k == K_VOID) return 0;
    td->alias_ok = 1;
    return 1;
}

static char *rec_name(Rec *r, const char *hint) {
    if (r->loi_name) return r->loi_name;
    const char *c = r->td_name ? r->td_name : r->tag;
    char tmp[256];
    if (!c) { snprintf(tmp, sizeof tmp, "%s", hint ? hint : "Anon"); c = tmp; }
    r->loi_name = claim_name(c);
    return r->loi_name;
}

static char *map_type(Type *t, const char *hint) {
    if (!t) return NULL;
    if (t->k == K_TYPEDEF) {
        if (alias_ok(t->td)) {
            if (!t->td->loi_name) t->td->loi_name = claim_name(t->td->name);
            return xstrdup(t->td->loi_name);
        }
        return map_type(t->td->ty, hint);
    }
    const char *p = prim_name(t->k);
    if (p) return xstrdup(p);
    switch (t->k) {
        case K_PTR: {
            Type *b = resolve(t->base);
            if (!b || b->k == K_VOID || b->k == K_FUNC || b->k == K_UNKNOWN) return xstrdup("void*");
            if (b->k == K_REC) {
                if (!b->rec->owned) return xstrdup("void*");
                b->rec->used = 1;
                const char *rn = rec_name(b->rec, hint);
                char *o = xmalloc(strlen(rn) + 2);
                sprintf(o, "%s*", rn);
                return o;
            }
            char *inner = map_type(t->base, hint);
            if (!inner) return xstrdup("void*");
            char *out = xmalloc(strlen(inner) + 2);
            sprintf(out, "%s*", inner);
            free(inner);
            return out;
        }
        case K_ARRAY: {
            if (t->len < 0) { Type pt = *t; pt.k = K_PTR; return map_type(&pt, hint); }
            char *inner = map_type(t->base, hint);
            if (!inner) return NULL;
            char *out = xmalloc(strlen(inner) + 32);
            sprintf(out, "%s[%ld]", inner, t->len);
            free(inner);
            return out;
        }
        case K_REC: {
            Rec *r = t->rec;
            if (!r->owned || !r->complete || r->unsupported) return NULL;
            r->used = 1;
            return xstrdup(rec_name(r, hint));
        }
        default: return NULL;
    }
}

/* the by-value record behind a field type (through arrays/typedefs), if any */
static Rec *by_value_rec(Type *t) {
    t = resolve(t);
    while (t && t->k == K_ARRAY) t = resolve(t->base);
    return t && t->k == K_REC ? t->rec : NULL;
}

/* ------------------------------------------------------------------ emission */
typedef struct { char *name; int is_str; int64_t val; char *str; } Const;
static Const *consts; static int nconsts, capconsts;
static void add_const(const char *cname, int is_str, int64_t v, const char *s) {
    if (nconsts == capconsts) { capconsts = capconsts ? capconsts * 2 : 1024; consts = xrealloc(consts, (size_t)capconsts * sizeof *consts); }
    consts[nconsts].name = claim_name(cname);
    consts[nconsts].is_str = is_str;
    consts[nconsts].val = v;
    consts[nconsts].str = s ? xstrdup(s) : NULL;
    nconsts++;
}

static void emit_rec(Rec *r, Buf *out) {
    if (r->state) return;
    r->state = 1;
    const char *name = rec_name(r, NULL);
    int full = r->complete && !r->unsupported;
    char **ftypes = NULL;
    if (full) {
        ftypes = xmalloc((size_t)(r->nf ? r->nf : 1) * sizeof(char *));
        for (int i = 0; i < r->nf && full; i++) {
            char hint[256];
            snprintf(hint, sizeof hint, "%s_%s", name, r->f[i].name);
            ftypes[i] = map_type(r->f[i].ty, hint);
            if (!ftypes[i] || !strcmp(ftypes[i], "void")) { full = 0; }
        }
        if (r->nf == 0) full = 0;
        if (full)
            for (int i = 0; i < r->nf; i++) {
                Rec *d = by_value_rec(r->f[i].ty);
                if (d && d != r && d->owned) emit_rec(d, out);
            }
    }
    const char *kw = r->is_union ? "union" : "struct";
    if (!full) {
        buf_addf(out, "%s %s { char _opaque; }   // opaque: use through pointers\n", "struct", name);
    } else {
        buf_addf(out, "%s %s%s {\n", kw, name, r->packed ? " @pack()" : "");
        for (int i = 0; i < r->nf; i++) {
            char *fn = safe_ident(r->f[i].name);
            buf_addf(out, "    %s %s;\n", ftypes[i], fn);
            free(fn);
        }
        buf_add(out, "}\n");
    }
    r->state = 2;
    if (ftypes) { for (int i = 0; i < r->nf; i++) free(ftypes[i]); free(ftypes); }
}

static void mkdir_p(const char *path) {
    char tmp[PATH_MAX];
    snprintf(tmp, sizeof tmp, "%s", path);
    for (char *p = tmp + 1; *p; p++) {
        if (*p == '/') { *p = 0; mkdir(tmp, 0777); *p = '/'; }
    }
}

static char *stem_of(const char *path) {
    const char *s = strrchr(path, '/');
    s = s ? s + 1 : path;
    char buf[128];
    size_t n = 0;
    for (; s[n] && s[n] != '.' && n + 1 < sizeof buf; n++) buf[n] = isalnum((unsigned char)s[n]) ? s[n] : '_';
    buf[n] = 0;
    return xstrdup(buf);
}

static void detect_prefix(void) {
    struct { char p[64]; int n; } cand[64];
    int nc = 0, total = 0;
    for (int i = 0; i < nfuncs; i++) {
        if (!funcs[i].owned || funcs[i].name[0] == '_') continue;
        total++;
        const char *u = strchr(funcs[i].name, '_');
        if (!u || u == funcs[i].name || (size_t)(u - funcs[i].name) + 2 > sizeof cand[0].p) continue;
        char pre[64];
        size_t len = (size_t)(u - funcs[i].name) + 1;
        memcpy(pre, funcs[i].name, len);
        pre[len] = 0;
        int k = 0;
        for (; k < nc; k++) if (!strcmp(cand[k].p, pre)) { cand[k].n++; break; }
        if (k == nc && nc < 64) { strcpy(cand[nc].p, pre); cand[nc].n = 1; nc++; }
    }
    int best = -1;
    for (int k = 0; k < nc; k++) if (best < 0 || cand[k].n > cand[best].n) best = k;
    if (best >= 0 && cand[best].n * 10 >= total * 6) snprintf(g_prefix, sizeof g_prefix, "%s", cand[best].p);
}

static int run_cpp(const char *cc, const char *flags, const char *input, int dD, const char *out, const char *err) {
    char cmd[8192];
    int n = snprintf(cmd, sizeof cmd, "%s -E %s %s -x c '%s' -o '%s' 2>'%s'", cc, dD ? "-dD" : "", flags ? flags : "", input, out, err);
    if (n < 0 || (size_t)n >= sizeof cmd) return -1;
    return system(cmd);
}

int loi_bindgen(const BindgenOptions *opt) {
    if (!opt || !opt->header) return 1;
    if (strchr(opt->header, '\'')) { fprintf(stderr, "error: unsupported character in header path\n"); return 1; }
    char *header_text = read_all(opt->header);
    if (!header_text) { fprintf(stderr, "error: cannot open %s\n", opt->header); return 1; }

    const char *cc = getenv("CC");
    if (!cc || !*cc) cc = "cc";
    mkdir("/tmp/opencode", 0777);
    char fa[128], fb[128], fe[128], fp[128];
    snprintf(fa, sizeof fa, "/tmp/opencode/bindgen_%d_a.i", (int)getpid());
    snprintf(fb, sizeof fb, "/tmp/opencode/bindgen_%d_b.i", (int)getpid());
    snprintf(fe, sizeof fe, "/tmp/opencode/bindgen_%d.err", (int)getpid());
    snprintf(fp, sizeof fp, "/tmp/opencode/bindgen_%d_probe.h", (int)getpid());

    /* 1. preprocess the user's header (includes + #defines kept) */
    if (run_cpp(cc, opt->include_flags, opt->header, 1, fa, fe) != 0) {
        char *e = read_all(fe);
        fprintf(stderr, "error: cannot preprocess %s with '%s':\n%s", opt->header, cc, e ? e : "");
        free(e);
        unlink(fa); unlink(fe);
        return 1;
    }
    char *unit = read_all(fa);
    if (!unit) { fprintf(stderr, "error: no preprocessor output\n"); return 1; }

    /* 2. which files belong to the library; tokenise; parse */
    char *unit2 = xstrdup(unit);
    scan_unit(unit, NULL, 1);
    compute_owned(header_text, opt->header);
    TokVec raw = { 0 };
    scan_unit(unit2, &raw, 0);
    filter_tokens(&raw);
    parse_translation_unit();

    /* 3. fold macros: let the C preprocessor expand each owned object-like macro */
    int *cand = xmalloc((size_t)(nmacros ? nmacros : 1) * sizeof(int));
    int ncand = 0;
    for (int i = 0; i < nmacros; i++) {
        Macro *m = &macros[i];
        if (!m->name || !m->owned || !m->body[0] || m->name[0] == '_') continue;
        if (ends_with(m->name, "_H") || ends_with(m->name, "_h_") || ends_with(m->name, "_H_")) continue;
        cand[ncand++] = i;
    }
    char absheader[PATH_MAX];
    if (!realpath(opt->header, absheader)) snprintf(absheader, sizeof absheader, "%s", opt->header);
    char **expansion = xmalloc((size_t)(ncand ? ncand : 1) * sizeof(char *));
    for (int i = 0; i < ncand; i++) expansion[i] = NULL;
    if (ncand) {
        FILE *pf = fopen(fp, "w");
        if (pf) {
            fprintf(pf, "#include \"%s\"\n", absheader);
            for (int i = 0; i < ncand; i++) fprintf(pf, "__loi_m__ %s\n", macros[cand[i]].name);
            fclose(pf);
            if (run_cpp(cc, opt->include_flags, fp, 0, fb, fe) == 0) {
                /* The compiler may wrap a macro expansion onto following lines (and
                   insert line markers), so an entry is everything up to the next marker. */
                char *out = read_all(fb);
                int k = -1;
                for (char *p = out; p && *p; ) {
                    char *eol = strchr(p, '\n');
                    if (eol) *eol = 0;
                    if (!strncmp(p, "__loi_m__", 9)) {
                        k++;
                        if (k < ncand) expansion[k] = xstrdup(p + 9);
                    } else if (k >= 0 && k < ncand && p[0] != '#') {
                        size_t l0 = strlen(expansion[k]), l1 = strlen(p);
                        expansion[k] = xrealloc(expansion[k], l0 + l1 + 2);
                        expansion[k][l0] = ' ';
                        memcpy(expansion[k] + l0 + 1, p, l1 + 1);
                    }
                    p = eol ? eol + 1 : NULL;
                }
                free(out);
            }
        }
    }

    /* 4. naming policy */
    if (opt->strip_prefix) snprintf(g_prefix, sizeof g_prefix, "%s", opt->strip_prefix);
    else detect_prefix();
    char *pkg = NULL;
    if (opt->package) pkg = xstrdup(opt->package);
    else if (opt->out_path) pkg = stem_of(opt->out_path);
    else pkg = stem_of(opt->header);

    /* reserve names in a stable order: types, aliases, constants, functions */
    for (int i = 0; i < nrecs; i++)
        if (recs[i]->owned && (recs[i]->tag || recs[i]->td_name)) rec_name(recs[i], NULL);
    for (int i = 0; i < ntds; i++)
        if (alias_ok(tds[i]) && !tds[i]->loi_name) tds[i]->loi_name = claim_name(tds[i]->name);
    for (int i = 0; i < necs; i++)
        if (ecs[i].owned && ecs[i].name[0] != '_') add_const(ecs[i].name, 0, ecs[i].val, NULL);
    for (int i = 0; i < ncand; i++) {
        if (!expansion[i]) continue;
        TokVec tv = { 0 };
        lex_line(expansion[i], 1, &tv);
        int64_t v;
        if (tv.n == 1 && tv.v[0].k == TK_STR) {
            const char *s = tv.v[0].s;
            int ok = 1;
            for (const char *q = s + 1; *q; q++)
                if (*q == '\\' && !strchr("\\\"nt", q[1])) ok = 0;
            if (ok) {
                char *inner = xstrndup(s + 1, strlen(s) - 2);
                add_const(macros[cand[i]].name, 1, 0, inner);
                free(inner);
            }
        } else if (tv.n && eval_const(tv.v, tv.n, &v)) {
            add_const(macros[cand[i]].name, 0, v, NULL);
        }
        for (size_t k = 0; k < tv.n; k++) free(tv.v[k].s);
        free(tv.v);
    }

    /* functions: map signatures first (this marks which opaque records are used) */
    typedef struct { int f; char *ret; char **pt; int ok; const char *why; } FSig;
    FSig *sigs = xmalloc((size_t)(nfuncs ? nfuncs : 1) * sizeof *sigs);
    int nsig = 0;
    Buf skipped = { 0 };
    int nskipped = 0;
    for (int i = 0; i < nfuncs; i++) {
        Func *f = &funcs[i];
        if (!f->owned || f->name[0] == '_') continue;
        char *stripped = strip_prefix_name(f->name);
        int is_main = !strcmp(stripped, "main");
        free(stripped);
        if (is_main) continue;
        FSig s = { i, NULL, xmalloc((size_t)(f->np ? f->np : 1) * sizeof(char *)), 1, NULL };
        if (f->variadic) { s.ok = 0; s.why = "variadic"; }
        /* structs passed or returned by value need ABI coercion that the loi
           backend does not do; such functions are listed as skipped instead */
        if (s.ok) {
            Type *rr = resolve(f->ret);
            if (rr && rr->k == K_REC) { s.ok = 0; s.why = "returns a struct by value"; }
        }
        if (s.ok) {
            s.ret = map_type(f->ret, NULL);
            if (!s.ret) { s.ok = 0; s.why = "return type not representable"; }
        }
        for (int k = 0; k < f->np && s.ok; k++) {
            Type *pr = resolve(f->params[k]);
            if (pr && pr->k == K_REC) { s.ok = 0; s.why = "takes a struct by value"; break; }
            s.pt[k] = map_type(f->params[k], NULL);
            if (!s.pt[k] || !strcmp(s.pt[k], "void")) { s.ok = 0; s.why = "unsupported parameter type"; }
        }
        if (!s.ok) {
            buf_addf(&skipped, "//   %s (%s)\n", f->name, s.why);
            nskipped++;
        }
        sigs[nsig++] = s;
    }
    for (int i = 0; i < nrecs; i++) {   /* field types of exported records may use opaque ones */
        Rec *r = recs[i];
        if (!r->owned || !r->complete || r->unsupported) continue;
        for (int k = 0; k < r->nf; k++) {
            char hint[256];
            snprintf(hint, sizeof hint, "%s_%s", rec_name(r, NULL), r->f[k].name);
            char *s = map_type(r->f[k].ty, hint);
            free(s);
        }
    }

    /* 5. write the package */
    Buf out = { 0 };
    const char *hbase = strrchr(opt->header, '/');
    hbase = hbase ? hbase + 1 : opt->header;
    buf_addf(&out, "// Generated by `loi --mode=generator %s`.\n", hbase);
    if (g_prefix[0]) buf_addf(&out, "// C prefix \"%s\" is dropped from every name; each function keeps its C symbol\n// in @attr(link_name(\"...\")). Names that clash with a loi keyword get a trailing '_'.\n", g_prefix);
    else buf_add(&out, "// Each function keeps its C symbol in @attr(link_name(\"...\")). Names that clash with a\n// loi keyword get a trailing '_'.\n");
    buf_add(&out, "// Safe to edit by hand; regenerate to refresh.\n\n");
    buf_addf(&out, "@package(%s)\n\n", pkg);

    if (nconsts) {
        buf_add(&out, "// ---- Constants ----\n");
        for (int i = 0; i < nconsts; i++) {
            if (consts[i].is_str) buf_addf(&out, "%s :: \"%s\";\n", consts[i].name, consts[i].str);
            else if (consts[i].val >= 0 && consts[i].val > 9) buf_addf(&out, "%s :: 0x%llX;\n", consts[i].name, (unsigned long long)consts[i].val);
            else buf_addf(&out, "%s :: %lld;\n", consts[i].name, (long long)consts[i].val);
        }
        buf_add(&out, "\n");
    }

    int any_alias = 0;
    for (int i = 0; i < ntds; i++) {
        Typedef *td = tds[i];
        if (!td->alias_ok || !td->loi_name) continue;
        char *m = map_type(td->ty, NULL);
        if (!m) continue;
        if (!any_alias) { buf_add(&out, "// ---- Type aliases ----\n"); any_alias = 1; }
        buf_addf(&out, "type %s = %s;\n", td->loi_name, m);
        free(m);
    }
    if (any_alias) buf_add(&out, "\n");

    buf_add(&out, "// ---- Structs and unions ----\n");
    for (int i = 0; i < nrecs; i++) {
        Rec *r = recs[i];
        if (!r->owned) continue;
        int full = r->complete && !r->unsupported;
        if (full || r->used) {
            if (!r->tag && !r->td_name && !r->loi_name) continue;   /* anonymous and never referenced */
            emit_rec(r, &out);
        }
    }
    buf_add(&out, "\n// ---- Functions ----\n");
    int nfn = 0;
    for (int i = 0; i < nsig; i++) {
        FSig *s = &sigs[i];
        if (!s->ok) continue;
        Func *f = &funcs[s->f];
        char *lname = claim_name(f->name);
        buf_addf(&out, "@attr(link_name(\"%s\"))\nextern %s %s(", f->name, s->ret, lname);
        char **seen_p = xmalloc((size_t)(f->np ? f->np : 1) * sizeof(char *));
        for (int k = 0; k < f->np; k++) {
            char pn[64];
            if (f->pnames[k] && f->pnames[k][0]) snprintf(pn, sizeof pn, "%s", f->pnames[k]);
            else snprintf(pn, sizeof pn, "a%d", k);
            char *sp = safe_ident(pn);
            for (int d = 0; d < k; d++) if (!strcmp(seen_p[d], sp)) { size_t l = strlen(sp); sp = xrealloc(sp, l + 16); snprintf(sp + l, 16, "_%d", k); break; }
            seen_p[k] = sp;
            buf_addf(&out, "%s%s: %s", k ? ", " : "", sp, s->pt[k]);
        }
        for (int k = 0; k < f->np; k++) free(seen_p[k]);
        free(seen_p);
        buf_add(&out, ");\n");
        free(lname);
        nfn++;
    }
    if (nskipped) {
        buf_addf(&out, "\n// Not generated (%d):\n%s", nskipped, skipped.s ? skipped.s : "");
    }

    int rc = 0;
    if (opt->out_path) {
        mkdir_p(opt->out_path);
        FILE *f = fopen(opt->out_path, "w");
        if (!f) { fprintf(stderr, "error: cannot write %s\n", opt->out_path); rc = 1; }
        else {
            fputs(out.s, f);
            fclose(f);
            fprintf(stderr, "generated %s: %d constants, %d functions (%d skipped)\n", opt->out_path, nconsts, nfn, nskipped);
        }
    } else {
        fputs(out.s, stdout);
    }
    unlink(fa); unlink(fb); unlink(fe); unlink(fp);
    return rc;
}
