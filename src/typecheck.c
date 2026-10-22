#include "typecheck.h"
#include "ast.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>

static int errors = 0;
static int cur_line = 0, cur_col = 0;

typedef struct CapBind {
    char *name;
    Type *ty;        /* TY_CAPABILITY */
    int consumed;    /* 0 = live, 1 = consumed */
    int owned;       /* 1 = linear/owned, 0 = borrow/ambient */
    int soft;        /* 1 = need not be consumed at scope exit (params) */
    int line, col;   /* declaration site, for diagnostics */
} CapBind;

typedef struct Scope {
    char **names;
    Type **types;
    size_t count, cap;
    struct Scope *parent;
    CapBind *binds;  /* lexical capabilities introduced in this scope */
    size_t nbind, bindcap;
} Scope;

static Scope *scope = NULL;

static void scope_push(void) {
    Scope *s = calloc(1, sizeof(Scope));
    s->parent = scope;
    scope = s;
}
static void cap_check_scope_exit(void);   /* fwd: linearity at scope exit */
static const char *call_callee_name(ASTNode *call);  /* fwd: call diagnostics */
static void cap_validate_csv(const char *csv, const char *ctx);  /* fwd: @capability check */
static void scope_pop(void) { cap_check_scope_exit(); if (scope) scope = scope->parent; }

static void scope_add(const char *name, Type *t) {
    if (!scope) scope_push();
    if (scope->count == scope->cap) {
        scope->cap = scope->cap ? scope->cap*2 : 8;
        scope->names = realloc(scope->names, scope->cap*sizeof(char*));
        scope->types = realloc(scope->types, scope->cap*sizeof(Type*));
    }
    scope->names[scope->count] = strdup(name);
    scope->types[scope->count] = t;
    scope->count++;
}

static Type *scope_lookup(const char *name) {
    for (Scope *s = scope; s; s = s->parent) {
        for (size_t i = 0; i < s->count; i++) {
            if (strcmp(s->names[i], name) == 0) return s->types[i];
        }
    }
    return NULL;
}

/* true when `name` is bound by a local/parameter scope (i.e. not the global
   scope at the bottom of the chain); used to avoid flagging a local that
   happens to shadow a package-local global. */
static int scope_has_local(const char *name) {
    for (Scope *s = scope; s && s->parent; s = s->parent) {
        for (size_t i = 0; i < s->count; i++)
            if (strcmp(s->names[i], name) == 0) return 1;
    }
    return 0;
}

/* function table */
typedef struct { char *name; Type *return_type; size_t param_count; Type **param_types; ASTNode *node; char *effects; int must_use; } FuncInfo;
static FuncInfo *funcs = NULL;
static size_t func_count = 0, func_cap = 0;

static Type **alloc_types(size_t n);
static void register_func(ASTNode *f) {
    if (func_count == func_cap) {
        func_cap = func_cap ? func_cap*2 : 8;
        funcs = realloc(funcs, func_cap*sizeof(FuncInfo));
    }
    funcs[func_count].name = f->name ? strdup(f->name) : strdup("anon");
    funcs[func_count].return_type = f->ty;
    funcs[func_count].param_count = f->arg_count;
    funcs[func_count].param_types = alloc_types(f->arg_count);
    for (size_t j = 0; j < f->arg_count; j++) {
        funcs[func_count].param_types[j] = f->args[j] ? f->args[j]->ty : NULL;
    }
    funcs[func_count].node = f;
    funcs[func_count].effects = f->effects ? strdup(f->effects) : NULL;
    funcs[func_count].must_use = ast_has_attr(f, "must_use") ? 1 : 0;
    func_count++;
}

/* helper forward */
static Type **alloc_types(size_t n) { return calloc(n ? n : 1, sizeof(Type*)); }

static void add_error(const char *fmt, ...) {
    errors++;
    va_list ap; va_start(ap, fmt);
    fprintf(stderr, "Type error at %d:%d: ", cur_line, cur_col);
    vfprintf(stderr, fmt, ap);
    fprintf(stderr, "\n");
    va_end(ap);
}

static int warnings = 0;
/* a diagnostic that does not fail the build (`@attr(must_use)`, etc.) */
static void add_warning(const char *fmt, ...) {
    warnings++;
    va_list ap; va_start(ap, fmt);
    fprintf(stderr, "Warning at %d:%d: ", cur_line, cur_col);
    vfprintf(stderr, fmt, ap);
    fprintf(stderr, "\n");
    va_end(ap);
}

/* ---- lexical capabilities -------------------------------------------------
   Capabilities are permissions that exist only at compile time and erase to
   nothing in codegen. See docs/capabilities.md. */

/* capability kinds declared through `@capability(...)`. While empty, any name
   is accepted (permissive), so files that are `@inc`'d without the declaration
   still typecheck. */
static char *g_capkinds = NULL;

static int csv_has(const char *csv, const char *name) {
    if (!csv || !name) return 0;
    size_t n = strlen(name);
    const char *s = csv;
    while (s && *s) {
        const char *c = strchr(s, ',');
        size_t l = c ? (size_t)(c - s) : strlen(s);
        if (l == n && strncmp(s, name, n) == 0) return 1;
        s = c ? c + 1 : NULL;
    }
    return 0;
}

static void capkind_add(const char *csv) {
    if (!csv || !*csv) return;
    if (!g_capkinds || !*g_capkinds) { free(g_capkinds); g_capkinds = strdup(csv); return; }
    size_t a = strlen(g_capkinds), b = strlen(csv);
    char *n = malloc(a + b + 2);
    memcpy(n, g_capkinds, a); n[a] = ',';
    memcpy(n + a + 1, csv, b + 1);
    free(g_capkinds); g_capkinds = n;
}

static int capkind_known(const char *name) {
    if (!g_capkinds || !*g_capkinds) return 1;   /* permissive */
    return csv_has(g_capkinds, name);
}

/* The comma-separated capability names proven by `have`. */
static int cap_set_contains(Type *have, const char *need) {
    if (!have || have->kind != TY_CAPABILITY || !need) return 0;
    for (size_t i = 0; i < have->cap_count; i++)
        if (strcmp(have->cap_names[i], need) == 0) return 1;
    return 0;
}

/* Does the union of live capability bindings in scope prove every name in the
   comma-separated set `csv`? */
static int cap_live_covers(const char *csv) {
    if (!csv || !*csv) return 1;
    char *cpy = strdup(csv);
    int ok = 1;
    for (char *tok = strtok(cpy, ","); tok; tok = strtok(NULL, ",")) {
        int found = 0;
        for (Scope *s = scope; s && !found; s = s->parent)
            for (size_t i = 0; i < s->nbind; i++)
                if (!s->binds[i].consumed && cap_set_contains(s->binds[i].ty, tok)) { found = 1; break; }
        if (!found) { ok = 0; break; }
    }
    free(cpy);
    return ok;
}

static void cap_bind_add(const char *name, Type *t, int owned, int soft, int line, int col) {
    if (!name) return;
    if (!scope) scope_push();
    if (scope->nbind == scope->bindcap) {
        scope->bindcap = scope->bindcap ? scope->bindcap * 2 : 4;
        scope->binds = realloc(scope->binds, scope->bindcap * sizeof(CapBind));
    }
    scope->binds[scope->nbind].name = strdup(name);
    scope->binds[scope->nbind].ty = t;
    scope->binds[scope->nbind].consumed = 0;
    scope->binds[scope->nbind].owned = owned;
    scope->binds[scope->nbind].soft = soft;
    scope->binds[scope->nbind].line = line;
    scope->binds[scope->nbind].col = col;
    scope->nbind++;
    /* make the name visible to the ordinary expression typechecker too */
    scope_add(name, t);
}

static CapBind *cap_bind_find(const char *name) {
    if (!name) return NULL;
    for (Scope *s = scope; s; s = s->parent)
        for (size_t i = s->nbind; i-- > 0;)
            if (strcmp(s->binds[i].name, name) == 0) return &s->binds[i];
    return NULL;
}

/* Build a TY_CAPABILITY type from a comma-separated name set. */
static Type *cap_type_from_csv(const char *csv, int borrow) {
    Type *t = type_new(TY_CAPABILITY);
    t->name = strdup("cap");
    t->cap_borrow = borrow;
    size_t capn = 4, cnt = 0;
    char **arr = malloc(capn * sizeof(char *));
    char *cpy = csv ? strdup(csv) : strdup("");
    for (char *tok = strtok(cpy, ","); tok; tok = strtok(NULL, ",")) {
        if (cnt == capn) { capn *= 2; arr = realloc(arr, capn * sizeof(char *)); }
        arr[cnt++] = strdup(tok);
    }
    free(cpy);
    t->cap_names = arr;
    t->cap_count = cnt;
    return t;
}

/* does type `have` prove every permission required by `need`? (have ⊇ need) */
static int cap_set_satisfies(Type *have, Type *need) {
    if (!need || need->kind != TY_CAPABILITY) return 1;
    if (!have || have->kind != TY_CAPABILITY) return 0;
    for (size_t i = 0; i < need->cap_count; i++)
        if (!cap_set_contains(have, need->cap_names[i])) return 0;
    return 1;
}

/* Forward declaration for cap_live_candidates_ex (used before definition). */
static size_t cap_live_candidates_ex(Type *need, const char **name);

/* Count the live (unconsumed) capability bindings that prove `need`, and — when
   asked — the name of the single candidate. Ambient (non-owned) proofs take
   precedence over owned sources: an explicit `expr with S` or a region cap wins
   over the binding it was borrowed from, so it does not create a spurious
   second candidate. Used for argument elision, which requires an unambiguous
   choice. */
static size_t cap_live_candidates_ex(Type *need, const char **name) {
    size_t owned_n = 0, amb_n = 0;
    const char *owned_name = NULL, *amb_name = NULL;
    for (Scope *s = scope; s; s = s->parent)
        for (size_t i = 0; i < s->nbind; i++) {
            CapBind *b = &s->binds[i];
            if (b->consumed || !cap_set_satisfies(b->ty, need)) continue;
            if (b->owned) { owned_n++; owned_name = b->name; }
            else { amb_n++; amb_name = b->name; }
        }
    if (name) *name = amb_n ? amb_name : owned_name;
    return amb_n ? amb_n : owned_n;
}

/* Enter an ambient-capability scope for `@with_cap(S)` and `expr with S`:
   borrows S from a live binding (it does not conjure authority), then binds
   each atom of S as an ambient (non-owned) proof for the scope. Returns 1 when
   a scope was pushed (the caller must scope_pop). */
static int ambient_caps_begin(const char *csv, const char *ctx, int line, int col) {
    cap_validate_csv(csv, ctx);
    if (!csv || !*csv) {
        add_error("%s expects at least one capability", ctx);
        return 0;
    }
    if (!cap_live_covers(csv)) {
        add_error("%s('%s'): no live capability provides it; @grant it first (at a root) or drop the clause",
                  ctx, csv);
    }
    scope_push();
    Type *ct = cap_type_from_csv(csv, 1);
    char *cpy = strdup(csv);
    for (char *tok = strtok(cpy, ","); tok; tok = strtok(NULL, ","))
        cap_bind_add(tok, ct, 0, 1, line, col);
    free(cpy);
    return 1;
}

static void cap_validate_csv(const char *csv, const char *ctx) {
    if (!csv || !*csv) return;
    char *cpy = strdup(csv);
    for (char *tok = strtok(cpy, ","); tok; tok = strtok(NULL, ","))
        if (!capkind_known(tok))
            add_error("capability '%s' (in %s) is not declared; add it to @capability(...)", tok, ctx);
    free(cpy);
}

static void cap_validate_type(Type *t, const char *ctx) {
    if (t && t->kind == TY_CAPABILITY)
        for (size_t i = 0; i < t->cap_count; i++)
            if (!capkind_known(t->cap_names[i]))
                add_error("capability '%s' (in %s) is not declared; add it to @capability(...)",
                          t->cap_names[i], ctx);
}

/* ---- type alias registry (`type X = ...`) ---- */
typedef struct { char *name; Type *type; } AliasEnt;
static AliasEnt *g_aliases = NULL;
static int g_nalias = 0, g_alias_cap = 0;

static void alias_add(const char *name, Type *t) {
    if (!name || !t) return;
    for (int i = 0; i < g_nalias; i++) {
        if (strcmp(g_aliases[i].name, name) == 0) { g_aliases[i].type = t; return; }
    }
    if (g_nalias == g_alias_cap) {
        g_alias_cap = g_alias_cap ? g_alias_cap * 2 : 128;
        g_aliases = realloc(g_aliases, (size_t)g_alias_cap * sizeof(AliasEnt));
    }
    g_aliases[g_nalias].name = strdup(name);
    g_aliases[g_nalias].type = t;
    g_nalias++;
}
static Type *alias_find(const char *name) {
    for (int i = 0; i < g_nalias; i++)
        if (strcmp(g_aliases[i].name, name) == 0) return g_aliases[i].type;
    return NULL;
}
static void alias_clear(void) {
    for (int i = 0; i < g_nalias; i++) free(g_aliases[i].name);
    g_nalias = 0;
}

/* ---- symbol visibility registry (@package + @attr local/global) ----
   Every top-level declaration belongs to the @package that was in effect
   where it appeared; files sharing a @package form one namespace. A symbol
   marked `local` (via @attr(local)) may only be referenced from code in the
   same package; `global` (the default) is visible everywhere. */
typedef struct { char *name; char *package; char *visibility; } SymVis;
static SymVis *g_syms = NULL;
static size_t g_nsyms = 0, g_syms_cap = 0;
static const char *g_cur_pkg = NULL;

static void symvis_clear(void) {
    for (size_t i = 0; i < g_nsyms; i++) {
        free(g_syms[i].name);
        free(g_syms[i].package);
        free(g_syms[i].visibility);
    }
    g_nsyms = 0;
}

static void symvis_add(const char *name, const char *pkg, const char *vis) {
    if (!name) return;
    for (size_t i = 0; i < g_nsyms; i++)
        if (strcmp(g_syms[i].name, name) == 0) return;
    if (g_nsyms == g_syms_cap) {
        g_syms_cap = g_syms_cap ? g_syms_cap * 2 : 16;
        g_syms = realloc(g_syms, g_syms_cap * sizeof(SymVis));
    }
    g_syms[g_nsyms].name = strdup(name);
    g_syms[g_nsyms].package = pkg ? strdup(pkg) : NULL;
    g_syms[g_nsyms].visibility = vis ? strdup(vis) : NULL;
    g_nsyms++;
}

static SymVis *symvis_find(const char *name) {
    if (!name) return NULL;
    for (size_t i = 0; i < g_nsyms; i++)
        if (strcmp(g_syms[i].name, name) == 0) return &g_syms[i];
    return NULL;
}

/* a symbol is visible unless it is `local` and defined in another package */
static int symvis_visible(const char *name) {
    SymVis *e = symvis_find(name);
    if (!e || !e->visibility || strcmp(e->visibility, "local") != 0) return 1;
    return g_cur_pkg && e->package && strcmp(g_cur_pkg, e->package) == 0;
}

static const char *symvis_pkg(const char *name) {
    SymVis *e = symvis_find(name);
    return e && e->package ? e->package : "?";
}
/* follow alias chains; returns the aliased type (borrowed) or t itself */
static Type *resolve_type(Type *t) {
    for (int i = 0; i < 16 && t && t->name &&
         (t->kind == TY_NAMED || t->kind == TY_PRIMITIVE); i++) {
        Type *a = alias_find(t->name);
        if (!a) break;
        t = a;
    }
    return t;
}

/* type representation checks: aliases resolved, compound types compared
   structurally. Effects are deliberately not part of equality — they are
   checked separately at call sites and assignments. */
static int lit_fits(Type *want, ASTNode *e);
static int type_equal(Type *a, Type *b) {
    if (!a || !b) return 1; /* unknown allowed */
    a = resolve_type(a); b = resolve_type(b);
    /* the untyped `ptr` and `void*` convert to and from any pointer, as in C */
    if (a->kind == TY_POINTER || b->kind == TY_POINTER) {
        Type *pa = a->kind == TY_POINTER ? a : b;
        Type *po = pa == a ? b : a;
        int po_is_raw = (po->kind == TY_PRIMITIVE && po->name && strcmp(po->name, "ptr") == 0);
        Type *vb = pa->base ? resolve_type(pa->base) : NULL;
        int pa_is_void = (vb && vb->kind == TY_PRIMITIVE && vb->name && strcmp(vb->name, "void") == 0);
        if (po_is_raw) return 1;
        if (po->kind == TY_POINTER) {
            Type *ob = po->base ? resolve_type(po->base) : NULL;
            int po_is_void = (ob && ob->kind == TY_PRIMITIVE && ob->name && strcmp(ob->name, "void") == 0);
            if (pa_is_void || po_is_void) return 1;
        }
    }
    if (a->kind != b->kind) return 0;
    if (a->kind == TY_FUNCTION) {
        if (a->param_count != b->param_count) return 0;
        for (size_t i = 0; i < a->param_count; i++)
            if (!type_equal(a->param_types[i], b->param_types[i])) return 0;
        return type_equal(a->return_type, b->return_type);
    }
    if (a->kind == TY_POINTER) return type_equal(a->base, b->base);
    if (a->kind == TY_ARRAY) {
        if (!type_equal(a->base, b->base)) return 0;
        if (a->array_size && b->array_size &&
            a->array_size->str_value && b->array_size->str_value &&
            strcmp(a->array_size->str_value, b->array_size->str_value) != 0) return 0;
        return 1;
    }
    if (a->kind == TY_CAPABILITY) {
        if (a->cap_borrow != b->cap_borrow) return 0;
        if (a->cap_count != b->cap_count) return 0;
        for (size_t i = 0; i < a->cap_count; i++)
            if (!cap_set_contains(b, a->cap_names[i])) return 0;
        return 1;
    }
    if (a->name && b->name && strcmp(a->name, b->name) != 0) return 0;
    return 1;
}

/* Append `src` to the NUL-terminated buffer `dst[cap]`, truncating silently. */
static void buf_cat(char *dst, size_t cap, const char *src) {
    if (!cap) return;
    size_t l = strlen(dst);
    if (l + 1 >= cap) return;
    size_t room = cap - 1 - l;
    size_t n = strlen(src);
    if (n > room) n = room;
    memcpy(dst + l, src, n);
    dst[l + n] = '\0';
}

/* Render a short, human-readable name for `t` into `buf`.
   Named aliases are printed by their alias name rather than expanded, so
   diagnostics talk about the type the user wrote. */
static void type_name(Type *t, char *buf, size_t cap) {
    if (!cap) return;
    buf[0] = '\0';
    if (!t) { buf_cat(buf, cap, "?"); return; }
    switch (t->kind) {
        case TY_PRIMITIVE:
        case TY_NAMED:
            buf_cat(buf, cap, t->name ? t->name : "?");
            break;
        case TY_POINTER:
            type_name(t->base, buf, cap);
            buf_cat(buf, cap, "*");
            break;
        case TY_ARRAY:
            type_name(t->base, buf, cap);
            buf_cat(buf, cap, "[]");
            break;
        case TY_FUNCTION: {
            char inner[80];
            type_name(t->return_type, inner, sizeof(inner));
            buf_cat(buf, cap, "fn(...) -> ");
            buf_cat(buf, cap, inner);
            if (t->effects && *t->effects) {
                buf_cat(buf, cap, " !");
                buf_cat(buf, cap, t->effects);
            }
            break;
        }
        case TY_CAPABILITY: {
            buf_cat(buf, cap, "@cap(");
            if (t->cap_borrow) buf_cat(buf, cap, "borrow ");
            for (size_t i = 0; i < t->cap_count; i++) {
                if (i) buf_cat(buf, cap, " + ");
                buf_cat(buf, cap, t->cap_names[i]);
            }
            buf_cat(buf, cap, ")");
            break;
        }
        default:
            buf_cat(buf, cap, "?");
    }
}

/* Linearity: an owned capability that reaches the end of its scope without
   being consumed, returned, or @drop'ed is an error. */
static void cap_check_scope_exit(void) {
    if (!scope) return;
    for (size_t i = 0; i < scope->nbind; i++) {
        CapBind *b = &scope->binds[i];
        if (b->owned && !b->consumed && !b->soft) {
            cur_line = b->line; cur_col = b->col;
            char tn[96];
            type_name(b->ty, tn, sizeof(tn));
            add_error("live capability '%s' (%s) must be consumed, returned, or @drop'ed before the end of its scope",
                      b->name, tn);
        }
    }
}

/* Type registry for struct/union field lookup */
typedef struct {
    char *name;
    ASTNode *struct_decl;
} TypeRegEntry;


static TypeRegEntry *type_registry = NULL;
static size_t type_registry_count = 0;
static size_t type_registry_cap = 0;

static void type_registry_add(const char *name, ASTNode *struct_decl) {
    if (type_registry_count == type_registry_cap) {
        size_t nc = type_registry_cap ? type_registry_cap * 2 : 16;
        type_registry = realloc(type_registry, nc * sizeof(*type_registry));
        type_registry_cap = nc;
    }
    type_registry[type_registry_count].name = strdup(name);
    type_registry[type_registry_count].struct_decl = struct_decl;
    type_registry_count++;
}

static ASTNode *type_registry_find(const char *name) {
    for (size_t i = 0; i < type_registry_count; i++) {
        if (type_registry[i].name && strcmp(type_registry[i].name, name) == 0) {
            return type_registry[i].struct_decl;
        }
    }
    return NULL;
}

static void type_registry_clear(void) {
    for (size_t i = 0; i < type_registry_count; i++) {
        free(type_registry[i].name);
    }
    free(type_registry);
    type_registry = NULL;
    type_registry_count = 0;
    type_registry_cap = 0;
}

/* helper: check if caller effects cover callee effects */
static int effects_cover(const char *caller, const char *callee) {
    if (!callee || !*callee) return 1; /* callee has no effects */
    if (!caller || !*caller) return 0; /* caller has no effects but callee does */
    /* simple comma-separated list containment check */
    char *caller_cpy = strdup(caller);
    char *callee_cpy = strdup(callee);
    char *tok = strtok(callee_cpy, ",");
    int ok = 1;
    while (tok && ok) {
        char *c = strstr(caller_cpy, tok);
        if (!c || (c > caller_cpy && c[-1] != ',' && c[-1] != ' ') || (c[strlen(tok)] && c[strlen(tok)] != ',' && c[strlen(tok)] != ' ')) {
            ok = 0; break;
        }
        tok = strtok(NULL, ",");
    }
    free(caller_cpy);
    free(callee_cpy);
    return ok;
}

static char *current_function_effects = NULL;
static char *current_function_needs = NULL;
static char *current_function_gives = NULL;
static ASTNode *current_function_node = NULL;
static Type *current_function_return_type = NULL;
static const char *current_function_name = NULL;

/* The name of the function whose body is being checked, for diagnostics. */
static const char *enclosing_name(void) {
    return current_function_name ? current_function_name : "<top level>";
}

/* Reject storing a function value whose effects are not covered by the
   target's effects (e.g. assigning an `!IO` function to a pure function
   pointer). This is what makes effects meaningful for function values:
   the target's declared type is the promise callers rely on.
   `what` is a full noun phrase for the diagnostic, e.g. "assignment" or
   "variable 'f'". */
static void check_assign_effects(Type *target, Type *value, const char *what) {
    Type *tt = resolve_type(target);
    Type *vv = resolve_type(value);
    if (tt && vv && tt->kind == TY_FUNCTION && vv->kind == TY_FUNCTION &&
        vv->effects && !effects_cover(tt->effects, vv->effects)) {
        add_error("%s would perform effect(s) '%s', but the target is declared '%s'",
                  what, vv->effects, tt->effects ? tt->effects : "pure");
    }
}

/* Type-check a call's arguments against the callee's parameter list:
   arity, per-argument type, and (for function-typed parameters) effects.
   `what` names the call in diagnostics, e.g. "call 'f'". */
static Type *type_of_expr(ASTNode *e);

/* Enforce a capability argument against a `@cap(...)` parameter: satisfaction
   (the value must prove the required set) and linearity (an owned parameter
   consumes an owned binding). */
static void cap_arg_check(size_t idx, const char *what, ASTNode *arg, Type *ptype) {
    char eb[96];
    type_name(ptype, eb, sizeof(eb));
    if (arg && arg->type == AST_EXPR_IDENT) {
        CapBind *b = cap_bind_find(arg->name);
        if (b) {
            if (b->consumed) {
                add_error("capability '%s' was already consumed", arg->name);
                return;
            }
            if (!cap_set_satisfies(b->ty, ptype)) {
                char ab[96];
                type_name(b->ty, ab, sizeof(ab));
                add_error("argument %zu of %s requires %s, but '%s' proves %s", idx + 1, what, eb, arg->name, ab);
                return;
            }
            if (!ptype->cap_borrow) {
                if (!b->owned) {
                    add_error("cannot consume '%s': it is a borrowed/ambient capability", arg->name);
                    return;
                }
                b->consumed = 1;   /* ownership transfer */
            }
            return;
        }
        Type *ct0 = type_of_expr(arg);
        if (ct0 && ct0->kind == TY_CAPABILITY) return;
        add_error("argument %zu of %s expects %s, but '%s' is not a capability", idx + 1, what, eb, arg->name ? arg->name : "?");
        return;
    }
    Type *aty = type_of_expr(arg);
    if (aty && aty->kind == TY_CAPABILITY) {
        if (!cap_set_satisfies(aty, ptype)) {
            char ab[96];
            type_name(aty, ab, sizeof(ab));
            add_error("argument %zu of %s requires %s, but the value proves %s", idx + 1, what, eb, ab);
        }
        return;
    }
    char ab[96];
    type_name(aty, ab, sizeof(ab));
    add_error("argument %zu of %s expects %s, but is %s", idx + 1, what, eb, ab);
}

/* `@needs(R)` is satisfied when the caller declares a superset of R or when
   live capabilities in scope cover R. */
static int needs_satisfied(const char *needs) {
    if (!needs || !*needs) return 1;
    if (current_function_needs && effects_cover(current_function_needs, needs)) return 1;
    return cap_live_covers(needs);
}

/* `@grant` is only allowed at a root: `main`, a function marked `@root`, or a
   function that declares `@gives(...)` covering the granted set. */
static int grant_authorized(const char *set) {
    if (current_function_name && strcmp(current_function_name, "main") == 0) return 1;
    if (current_function_node && ast_has_attr(current_function_node, "root")) return 1;
    if (current_function_gives && effects_cover(current_function_gives, set)) return 1;
    /* a function whose return type *is* the capability produces it */
    if (current_function_return_type && current_function_return_type->kind == TY_CAPABILITY) {
        int ok = 1;
        char *cpy = set ? strdup(set) : strdup("");
        for (char *tok = strtok(cpy, ","); tok; tok = strtok(NULL, ","))
            if (!cap_set_contains(current_function_return_type, tok)) { ok = 0; break; }
        free(cpy);
        if (ok) return 1;
    }
    return 0;
}

static void check_call_args(const char *what, size_t argc, ASTNode **argv,
                            size_t pcount, Type **ptypes) {
    Type **ats = calloc(argc ? argc : 1, sizeof(Type *));
    for (size_t i = 0; i < argc; i++) ats[i] = type_of_expr(argv[i]);
    if (pcount != argc) {
        /* If the missing tail is all capability parameters, `infer_cap_args`
           has already produced a specific diagnostic (or filled the tail in);
           a bare arity complaint would be redundant. */
        int elidable = (argc < pcount);
        if (elidable)
            for (size_t i = argc; i < pcount; i++)
                if (!ptypes[i] || ptypes[i]->kind != TY_CAPABILITY) { elidable = 0; break; }
        if (!elidable)
            add_error("%s expects %zu argument(s), got %zu", what, pcount, argc);
    }
    for (size_t i = 0; i < argc && i < pcount; i++) {
        if (ptypes[i] && ptypes[i]->kind == TY_CAPABILITY) {
            cap_arg_check(i, what, argv[i], ptypes[i]);
            continue;
        }
        if (ats[i] && ptypes[i] && !type_equal(ats[i], ptypes[i]) && !lit_fits(ptypes[i], argv[i])) {
            char eb[96], ab[96];
            type_name(ptypes[i], eb, sizeof(eb));
            type_name(ats[i], ab, sizeof(ats[i]) == 0 ? 0 : sizeof(ab));
            add_error("argument %zu of %s should be %s, but is %s", i + 1, what, eb, ab);
        } else if (ats[i] && ptypes[i]) {
            char phrase[192];
            snprintf(phrase, sizeof(phrase), "argument %zu of %s", i + 1, what);
            check_assign_effects(ptypes[i], ats[i], phrase);
        }
    }
    free(ats);
}

/* Elide trailing capability arguments when exactly one live binding in scope
   proves the required set, so `poke(&x)` can stand for `poke(&x, w)`. This is
   deliberately conservative: zero candidates, several candidates, or a
   non-capability parameter in the missing tail all leave the call alone (and
   the ordinary arity/type diagnostics fire instead). The synthesized
   identifiers are appended to the call node, so codegen sees them too. */
static void infer_cap_args(ASTNode *call, size_t pcount, Type **ptypes) {
    if (!call || call->arg_count >= pcount) return;
    for (size_t i = call->arg_count; i < pcount; i++)
        if (!ptypes[i] || ptypes[i]->kind != TY_CAPABILITY) return;
    const char *callee = call_callee_name(call);
    for (size_t i = call->arg_count; i < pcount; i++) {
        Type *need = ptypes[i];
        const char *cand = NULL;
        size_t n = cap_live_candidates_ex(need, &cand);
        if (n != 1) cand = NULL;
        if (!cand) {
            char eb[96];
            type_name(need, eb, sizeof(eb));
            add_error("cannot infer capability argument %zu of call to '%s': "
                      "expected exactly one live %s in scope, found %zu",
                      i + 1, callee ? callee : "?", eb, n);
            return;
        }
        ASTNode *id = ast_new(AST_EXPR_IDENT, call->line, call->column, call->file);
        id->name = strdup(cand);
        if (call->arg_count == call->arg_cap) {
            call->arg_cap = call->arg_cap ? call->arg_cap * 2 : 4;
            call->args = realloc(call->args, call->arg_cap * sizeof(ASTNode *));
        }
        call->args[call->arg_count++] = id;
    }
}

static int is_float_lexeme(const char *lex) {
    if (!lex) return 0;
    if (lex[0] == '0' && (lex[1] == 'x' || lex[1] == 'X' || lex[1] == 'b' || lex[1] == 'B')) return 0;
    return strchr(lex, '.') || strchr(lex, 'e') || strchr(lex, 'E');
}

/* ---- literal adaptation ------------------------------------------------
   A numeric literal (or an untyped `::` constant, which becomes one) has no
   fixed width: it converts to whichever numeric type the context wants, the
   way `unsigned int flags = 32;` or `ev.kind == QUIT` read naturally. Only
   literals adapt; two typed operands of different types still mismatch. */
static int is_lit_expr(ASTNode *e, int *is_float) {
    if (!e) return 0;
    switch (e->type) {
        case AST_EXPR_NUMBER:
            if (is_float_lexeme(e->str_value)) { *is_float = 1; }
            return 1;
        case AST_EXPR_CHAR: return 1;
        case AST_EXPR_UNARY:
            if (e->op_token == TOK_MINUS || e->op_token == TOK_PLUS || e->op_token == TOK_TILDE)
                return is_lit_expr(e->left, is_float);
            return 0;
        case AST_EXPR_BINARY:
            if (e->op_token == TOK_PLUS || e->op_token == TOK_MINUS || e->op_token == TOK_STAR ||
                e->op_token == TOK_SLASH || e->op_token == TOK_PERCENT || e->op_token == TOK_PIPE ||
                e->op_token == TOK_AMP || e->op_token == TOK_CARET || e->op_token == TOK_LSHIFT ||
                e->op_token == TOK_RSHIFT)
                return is_lit_expr(e->left, is_float) && is_lit_expr(e->right, is_float);
            return 0;
        default: return 0;
    }
}

static int numeric_prim_kind(Type *t) {   /* 1 = integer-like, 2 = floating, 0 = other */
    t = resolve_type(t);
    if (!t || t->kind != TY_PRIMITIVE || !t->name) return 0;
    const char *n = t->name;
    if (!strcmp(n, "float") || !strcmp(n, "double") ||
        !strcmp(n, "f32") || !strcmp(n, "f64")) return 2;
    if (!strcmp(n, "int") || !strcmp(n, "long") || !strcmp(n, "short") || !strcmp(n, "char") ||
        !strcmp(n, "byte") || !strcmp(n, "i8") || !strcmp(n, "i16") || !strcmp(n, "i32") ||
        !strcmp(n, "i64") || !strcmp(n, "u8") || !strcmp(n, "u16") || !strcmp(n, "u32") ||
        !strcmp(n, "u64") || !strncmp(n, "unsigned", 8) || !strncmp(n, "signed", 6)) return 1;
    return 0;
}

static int lit_fits(Type *want, ASTNode *e) {
    int is_float = 0;
    if (!is_lit_expr(e, &is_float)) return 0;
    int k = numeric_prim_kind(want);
    if (k == 0) return 0;
    if (is_float && k == 1) return 0;     /* a float literal never narrows to an integer */
    return 1;
}

static Type *number_type(const char *lex) {
    if (!lex) return NULL;
    if (is_float_lexeme(lex)) {
        Type *t = type_new(TY_PRIMITIVE); t->name = strdup("double"); return t;
    }
    Type *t = type_new(TY_PRIMITIVE); t->name = strdup("int"); return t;
}

/* synthesize the function type of a declared function (for use as a value) */
static Type *func_type_of(const FuncInfo *fi) {
    Type *t = type_new(TY_FUNCTION);
    t->param_count = fi->param_count;
    t->param_types = calloc(fi->param_count ? fi->param_count : 1, sizeof(Type *));
    for (size_t i = 0; i < fi->param_count; i++) t->param_types[i] = type_clone(fi->param_types[i]);
    if (fi->return_type) t->return_type = type_clone(fi->return_type);
    else { t->return_type = type_new(TY_PRIMITIVE); t->return_type->name = strdup("int"); }
    t->effects = fi->effects ? strdup(fi->effects) : NULL;
    return t;
}

static void check_stmt(ASTNode *s);

static Type *type_of_expr(ASTNode *e) {
    if (!e) return NULL;
    cur_line = e->line; cur_col = e->column;
    switch (e->type) {
        case AST_EXPR_NUMBER: return number_type(e->str_value);
        case AST_EXPR_STRING: { Type *t=type_new(TY_PRIMITIVE); t->name=strdup("ptr"); return t; }
        case AST_EXPR_CHAR: { Type *t=type_new(TY_PRIMITIVE); t->name=strdup("char"); return t; }
        case AST_EXPR_NULL: return NULL; /* null converts to any pointer/function type */
        case AST_EXPR_IDENT: {
            Type *t = scope_lookup(e->name);
            if (t) {
                /* a top-level (global-scope) binding may be package-local */
                if (e->name && !scope_has_local(e->name) && !symvis_visible(e->name))
                    add_error("'%s' is local to package '%s' and is not visible here",
                              e->name, symvis_pkg(e->name));
                return t;
            }
            {
                /* a function name used as a value (e.g. passed as a callback):
                   give it its function type so assignments/calls are checked */
                for (size_t i = 0; i < func_count; i++)
                    if (e->name && strcmp(funcs[i].name, e->name) == 0) {
                        if (!symvis_visible(e->name))
                            add_error("'%s' is local to package '%s' and is not visible here",
                                      e->name, symvis_pkg(e->name));
                        return func_type_of(&funcs[i]);
                    }
                add_error("undeclared identifier '%s'", e->name?e->name:"?");
            }
            return t;
        }
        case AST_EXPR_UNARY: {
            if (e->op_token == TOK_AMP) {
                Type *ot = type_of_expr(e->left);
                if (ot && (e->left->type == AST_EXPR_IDENT || e->left->type == AST_EXPR_FIELD)) {
                    Type *pt = type_new(TY_POINTER);
                    pt->base = ot;
                    pt->name = ot->name ? strdup(ot->name) : NULL;
                    return pt;
                }
                Type *t = type_new(TY_PRIMITIVE); t->name = strdup("ptr"); return t;
            }
            if (e->op_token == TOK_STAR) {
                (void)type_of_expr(e->left);
                /* conservatively return pointee as int */
                Type *u = type_new(TY_PRIMITIVE); u->name = strdup("int"); return u;
            }
            Type *t = type_of_expr(e->left);
            return t;
        }
        case AST_EXPR_BINARY: {
            if (e->op_token == TOK_ASSIGN || e->op_token == TOK_PLUSEQ || e->op_token == TOK_MINUSEQ ||
                e->op_token == TOK_STAREQ || e->op_token == TOK_SLASHEQ || e->op_token == TOK_PERCENTEQ) {
                Type *lt = type_of_expr(e->left);
                Type *rt = type_of_expr(e->right);
                if (lt && rt && !type_equal(lt, rt) && !lit_fits(lt, e->right)) {
                    char lb[96], rb[96];
                    type_name(lt, lb, sizeof(lb));
                    type_name(rt, rb, sizeof(rb));
                    add_error("cannot assign %s to %s", rb, lb);
                } else if (lt && rt) {
                    /* storing a function into a function pointer must not widen effects */
                    check_assign_effects(lt, rt, "assignment");
                }
                return lt ? lt : rt;
            }
            Type *lt = type_of_expr(e->left);
            Type *rt = type_of_expr(e->right);
            /* comparison -> bool */
            if (e->op_token == TOK_EQEQ || e->op_token == TOK_BANGEQ ||
                e->op_token == TOK_LT || e->op_token == TOK_LTEQ ||
                e->op_token == TOK_GT || e->op_token == TOK_GTEQ) {
                Type *b = type_new(TY_PRIMITIVE); b->name = strdup("bool");
                return b;
            }
            if (lt && rt && !type_equal(lt, rt)) {
                if (lit_fits(rt, e->left)) return rt;     /* literal adapts to the other side */
                if (lit_fits(lt, e->right)) return lt;
                char lb[96], rb[96];
                type_name(lt, lb, sizeof(lb));
                type_name(rt, rb, sizeof(rb));
                add_error("operator does not accept %s and %s", lb, rb);
            }
            return lt ? lt : rt;
        }
        case AST_EXPR_TERNARY: {
            (void)type_of_expr(e->cond);
            Type *tt = type_of_expr(e->then_branch);
            Type *et = type_of_expr(e->else_branch);
            if (tt && et && !type_equal(tt, et)) {
                char tb[96], eb[96];
                type_name(tt, tb, sizeof(tb));
                type_name(et, eb, sizeof(eb));
                add_error("ternary branches have different types: %s and %s", tb, eb);
            }
            return tt ? tt : et;
        }

        case AST_EXPR_CALL: {
            const char *name = (e->left && e->left->type == AST_EXPR_IDENT) ? e->left->name : NULL;
            const char *qual = NULL;   /* `pkg.fn(...)`: the package qualifier */
            if (!name && e->left && e->left->type == AST_EXPR_FIELD && e->left->name &&
                e->left->left && e->left->left->type == AST_EXPR_IDENT && e->left->left->name &&
                !scope_lookup(e->left->left->name) && strcmp(e->left->left->name, "std") != 0) {
                name = e->left->name;
                qual = e->left->left->name;
            }
            FuncInfo *finfo = NULL;
            for (size_t i = 0; i < func_count; i++) {
                if (name && strcmp(funcs[i].name, name) == 0) { finfo = &funcs[i]; break; }
            }
            if (finfo && qual && finfo->node && finfo->node->package &&
                strcmp(finfo->node->package, qual) != 0) {
                add_error("'%s' is not a member of package '%s' (it belongs to '%s')",
                          name, qual, finfo->node->package);
            }
            if (finfo) {
                char what[160];
                snprintf(what, sizeof(what), "call to '%s'", name);
                infer_cap_args(e, finfo->param_count, finfo->param_types);
                /* Effects and @needs are judged with the capabilities that are live
                   *before* this call: an owned capability handed to the callee is
                   consumed by check_call_args below, and a call such as
                   `stop_gfx(gfx)` (!Gfx) is still covered by the `gfx` it consumes. */
                int fx_ok = !(finfo->effects && !effects_cover(current_function_effects, finfo->effects)
                              && !cap_live_covers(finfo->effects));
                int needs_ok = !(finfo->node && finfo->node->needs && !needs_satisfied(finfo->node->needs));
                check_call_args(what, e->arg_count, e->args, finfo->param_count, finfo->param_types);
                if (!symvis_visible(name)) {
                    add_error("'%s' is local to package '%s' and is not visible here",
                              name, symvis_pkg(name));
                }
                if (!fx_ok) {
                    add_error("call to '%s' requires effect(s) '%s', not declared by '%s'",
                              name, finfo->effects, enclosing_name());
                }
                if (!needs_ok) {
                    add_error("call to '%s' @needs('%s'), which is not provided by '%s'",
                              name, finfo->node->needs, enclosing_name());
                }
                return finfo->return_type;
            }
            /* indirect call through a function-pointer value */
            if (name && !qual) {
                Type *ft = resolve_type(scope_lookup(name));
                if (ft && ft->kind == TY_FUNCTION) {
                    char what[160];
                    snprintf(what, sizeof(what), "call through '%s'", name);
                    infer_cap_args(e, ft->param_count, ft->param_types);
                    check_call_args(what, e->arg_count, e->args, ft->param_count, ft->param_types);
                    if (ft->effects && !effects_cover(current_function_effects, ft->effects)
                        && !cap_live_covers(ft->effects)) {
                        add_error("call through '%s' requires effect(s) '%s', not declared by '%s'",
                                  name, ft->effects, enclosing_name());
                    }
                    return ft->return_type;
                }
            }
            /* extern / unknown callee: still check that every argument is valid */
            for (size_t i = 0; i < e->arg_count; i++) type_of_expr(e->args[i]);
            return NULL;
        }
        case AST_EXPR_INDEX: {
            Type *lt = type_of_expr(e->left);
            type_of_expr(e->right);
            if (lt && lt->kind == TY_ARRAY && lt->base) {
                return lt->base;
            }
            /* fallback */
            Type *t = type_new(TY_PRIMITIVE); t->name = strdup("int"); return t;
        }
        case AST_EXPR_FIELD: {
            Type *lt = type_of_expr(e->left);
            /* the struct behind the base: a struct value or (for `p.f` / `p->f`) a pointer to one */
            const char *sname = NULL;
            Type *st = lt ? resolve_type(lt) : NULL;
            if (st && st->kind == TY_POINTER) {
                Type *pb = st->base ? resolve_type(st->base) : NULL;
                sname = (pb && pb->name) ? pb->name : st->name;
            } else if (st && (st->kind == TY_NAMED || st->kind == TY_PRIMITIVE)) {
                sname = st->name;
            }
            if (sname) {
                ASTNode *sd = type_registry_find(sname);
                if (sd && sd->items) {
                    for (size_t i = 0; i < sd->item_count; i++) {
                        ASTNode *f = sd->items[i];
                        if (f && f->name && e->name && strcmp(f->name, e->name) == 0) {
                            return f->ty;
                        }
                    }
                }
            }
            /* fallback */
            Type *t = type_new(TY_PRIMITIVE); t->name = strdup("int"); return t;
        }
        /* compile-time reflection constants always fold to integers */
        case AST_EXPR_SIZEOF:
        case AST_EXPR_ALIGNOF:
        case AST_EXPR_OFFSETOF: {
            Type *t = type_new(TY_PRIMITIVE); t->name = strdup("int"); return t;
        }
        default: return NULL;
    }
}

/* return type is `void`? (must_use is meaningless for void results) */
static int type_is_void(Type *t) {
    t = resolve_type(t);
    return t && t->kind == TY_PRIMITIVE && t->name && strcmp(t->name, "void") == 0;
}

/* name of the function being called, mirroring call resolution; NULL when the
   call is not a plain or package-qualified function call (e.g. a field call). */
static const char *call_callee_name(ASTNode *call) {
    if (!call || call->type != AST_EXPR_CALL) return NULL;
    ASTNode *l = call->left;
    if (!l) return NULL;
    if (l->type == AST_EXPR_IDENT) return l->name;
    if (l->type == AST_EXPR_FIELD) {
        ASTNode *base = l->left;
        /* `pkg.fn(...)`: the base is a package name, not a value */
        if (base && base->type == AST_EXPR_IDENT && !scope_lookup(base->name))
            return l->name;
    }
    return NULL;
}

static FuncInfo *find_func_info(const char *name) {
    if (!name) return NULL;
    for (size_t i = 0; i < func_count; i++)
        if (strcmp(funcs[i].name, name) == 0) return &funcs[i];
    return NULL;
}

static void check_stmt(ASTNode *s) {
    if (!s) return;
    cur_line = s->line; cur_col = s->column;
    switch (s->type) {
        case AST_BLOCK:
            scope_push();
            for (size_t i = 0; i < s->item_count; i++) check_stmt(s->items[i]);
            scope_pop();
            break;
        case AST_REGION:
            scope_push();
            {
                /* a region handle is a void* to the region's first byte */
                Type *rt = type_new(TY_POINTER);
                Type *vb = type_new(TY_PRIMITIVE);
                vb->name = strdup("void");
                rt->base = vb;
                rt->name = strdup("void");
                if (s->name) scope_add(s->name, rt);
            }
            if (s->cond) type_of_expr(s->cond);
            /* `region r(64) with Write as w { ... }`: the region carries lexical
               capabilities that are released with its memory (ambient, not owned). */
            if (s->caps) {
                cap_validate_csv(s->caps, "region");
                Type *ct = cap_type_from_csv(s->caps, 1);
                if (s->str_value) cap_bind_add(s->str_value, ct, 0, 1, s->line, s->column);
                else {
                    for (size_t i = 0; i < ct->cap_count; i++)
                        cap_bind_add(ct->cap_names[i], ct, 0, 1, s->line, s->column);
                }
            }
            check_stmt(s->body);
            scope_pop();
            break;
        case AST_VAR_DECL: {
            Type *t = s->ty;
            if (s->init) {
                Type *it = type_of_expr(s->init);
                if (t && it && !type_equal(t, it) && !lit_fits(t, s->init)) {
                    char tb[96], ib[96];
                    type_name(t, tb, sizeof(tb));
                    type_name(it, ib, sizeof(ib));
                    add_error("cannot initialize %s '%s' with a value of type %s",
                              tb, s->name ? s->name : "?", ib);
                } else if (t && it) {
                    char phrase[160];
                    snprintf(phrase, sizeof(phrase), "variable '%s' init", s->name ? s->name : "?");
                    check_assign_effects(t, it, phrase);
                }
            }
            if (t && t->kind == TY_CAPABILITY) {
                cap_validate_type(t, "variable declaration");
                cap_bind_add(s->name, t, t->cap_borrow ? 0 : 1, 0, s->line, s->column);
            } else if (s->name) {
                scope_add(s->name, t);
            }
            break;
        }
        case AST_IF:
            type_of_expr(s->cond);
            check_stmt(s->then_branch);
            if (s->else_branch) check_stmt(s->else_branch);
            break;
        case AST_WHILE:
            type_of_expr(s->cond);
            check_stmt(s->body);
            break;
        case AST_DO_WHILE:
            type_of_expr(s->cond);
            check_stmt(s->body);
            break;
        case AST_FOR:
            scope_push();
            if (s->init_stmt) check_stmt(s->init_stmt);
            if (s->cond_expr) type_of_expr(s->cond_expr);
            if (s->update) check_stmt(s->update);
            check_stmt(s->body);
            scope_pop();
            break;
        case AST_WHEN: {
            type_of_expr(s->cond);
            for (size_t i = 0; i < s->item_count; i++) {
                ASTNode *br = s->items[i];
                if (br->type == AST_WHEN_BRANCH) {
                    if (br->right) type_of_expr(br->right);
                    check_stmt(br->then_branch);
                }
            }
            break;
        }
        case AST_RANGE: {
            type_of_expr(s->cond);
            type_of_expr(s->right);
            if (s->name) {
                Type *iv = type_new(TY_PRIMITIVE); iv->name = strdup("i32");
                scope_add(s->name, iv);
            }
            check_stmt(s->body);
            break;
        }
        case AST_FOREACH: {
            type_of_expr(s->cond);
            if (s->name) {
                Type *et = type_new(TY_PRIMITIVE); et->name = strdup("i32");
                scope_add(s->name, et);
            }
            check_stmt(s->body);
            break;
        }
        case AST_RETURN:
            if (s->left) {
                /* returning a capability moves it out (satisfies linearity) */
                if (s->left->type == AST_EXPR_IDENT) {
                    CapBind *b = cap_bind_find(s->left->name);
                    if (b && !b->consumed && b->owned &&
                        current_function_return_type &&
                        current_function_return_type->kind == TY_CAPABILITY)
                        b->consumed = 1;
                }
                Type *rt = type_of_expr(s->left);
                /* Returning a function value must not widen its effects beyond
                   the function's declared return type. */
                if (rt) check_assign_effects(current_function_return_type, rt, "return value");
            }
            break;
        case AST_GRANT: {
            cap_validate_csv(s->caps, "@grant");
            if (!grant_authorized(s->caps)) {
                add_error("'%s' cannot @grant('%s'): not @root and no matching @gives(...) declared",
                          enclosing_name(), s->caps ? s->caps : "");
            }
            Type *ct = cap_type_from_csv(s->caps, 0);
            cap_bind_add(s->name, ct, 1, 0, s->line, s->column);
            break;
        }
        case AST_REFINE: {
            cap_validate_csv(s->caps, "@refine");
            cap_validate_csv(s->cap_to, "@refine");
            CapBind *b = cap_bind_find(s->name);
            if (!b) {
                add_error("@refine target '%s' is not a capability binding", s->name ? s->name : "?");
                break;
            }
            if (b->consumed) { add_error("capability '%s' was already consumed", s->name); break; }
            if (!cap_set_contains(b->ty, s->caps)) {
                add_error("cannot @refine('%s -> %s') '%s': it does not hold '%s'",
                          s->caps, s->cap_to, s->name, s->caps ? s->caps : "");
                break;
            }
            /* weaken: drop `from`, gain `to` */
            char csv[256]; csv[0] = 0;
            for (size_t i = 0; i < b->ty->cap_count; i++) {
                if (s->caps && strcmp(b->ty->cap_names[i], s->caps) == 0) continue;
                if (csv[0]) strncat(csv, ",", sizeof(csv) - strlen(csv) - 1);
                strncat(csv, b->ty->cap_names[i], sizeof(csv) - strlen(csv) - 1);
            }
            if (csv[0]) strncat(csv, ",", sizeof(csv) - strlen(csv) - 1);
            strncat(csv, s->cap_to ? s->cap_to : "", sizeof(csv) - strlen(csv) - 1);
            b->ty = cap_type_from_csv(csv, b->ty->cap_borrow);
            break;
        }
        case AST_DROP: {
            CapBind *b = cap_bind_find(s->name);
            if (!b) { add_error("@drop('%s'): not a capability binding", s->name ? s->name : "?"); break; }
            if (b->consumed) { add_error("capability '%s' was already consumed", s->name); break; }
            if (!b->owned) { add_error("cannot @drop('%s'): it is a borrowed/ambient capability", s->name); break; }
            b->consumed = 1;
            break;
        }
        case AST_CAP_DECL:
            /* purely lexical: nothing to do in check_stmt */
            break;
        case AST_WITH_CAP: {
            if (ambient_caps_begin(s->caps, "@with_cap", s->line, s->column)) {
                check_stmt(s->body);
                scope_pop();
            } else {
                check_stmt(s->body);
            }
            break;
        }
        case AST_EXPR_STMT: {
            int ambient = s->caps && ambient_caps_begin(s->caps, "with", s->line, s->column);
            if (s->left && s->left->type == AST_EXPR_CALL) {
                const char *cn = call_callee_name(s->left);
                FuncInfo *fi = find_func_info(cn);
                if (fi && fi->must_use && !type_is_void(fi->return_type)) {
                    cur_line = s->left->line;
                    cur_col = s->left->column;
                    add_warning("unused result of '%s' (marked @attr(must_use))", cn);
                }
            }
            type_of_expr(s->left);
            if (ambient) scope_pop();
            break;
        }
        case AST_CONTRACT:
            /* compile-time contract: best-effort, skip for now */
            break;
        default:
            break;
    }
}

/* ---- `NAME :: literal;` constants --------------------------------------
   Constants are substituted by their literal before any checking happens, so
   they are zero-cost and adapt to the numeric type of their context (see
   "literal adaptation" above). `pkg.NAME` selects the constant NAME declared
   in package `pkg`; a bare `NAME` also works inside the package and for
   packages whose names are not shadowed by a local. */
typedef struct { const char *name; const char *package; ASTNode *init; } ConstEnt;
static ConstEnt *g_consts = NULL;
static size_t g_nconsts = 0;
static const char **g_pkgnames = NULL;
static size_t g_npkgs = 0;

static ASTNode *clone_lit(ASTNode *e, int line, int col, const char *file) {
    ASTNode *n = ast_new(e->type, line, col, file);
    n->op_token = e->op_token;
    if (e->str_value) n->str_value = strdup(e->str_value);
    if (e->left) n->left = clone_lit(e->left, line, col, e->file);
    return n;
}

static ConstEnt *const_find(const char *name, const char *pkg) {
    ConstEnt *any = NULL;
    for (size_t i = 0; i < g_nconsts; i++) {
        if (strcmp(g_consts[i].name, name) != 0) continue;
        if (pkg && g_consts[i].package && strcmp(g_consts[i].package, pkg) == 0) return &g_consts[i];
        if (!pkg && !any) any = &g_consts[i];
    }
    return any;
}

static int is_pkg_name(const char *name) {
    for (size_t i = 0; i < g_npkgs; i++) if (strcmp(g_pkgnames[i], name) == 0) return 1;
    return 0;
}

static void consts_walk(ASTNode **slot) {
    ASTNode *n = *slot;
    if (!n) return;
    if (n->type == AST_EXPR_IDENT && n->name) {
        ConstEnt *c = const_find(n->name, NULL);
        if (c) { *slot = clone_lit(c->init, n->line, n->column, c->init->file); ast_free(n); return; }
    } else if (n->type == AST_EXPR_FIELD && n->name && n->left &&
               n->left->type == AST_EXPR_IDENT && n->left->name && is_pkg_name(n->left->name)) {
        ConstEnt *c = const_find(n->name, n->left->name);
        if (c) { *slot = clone_lit(c->init, n->line, n->column, c->init->file); ast_free(n); return; }
    }
    if (n->type == AST_CONST_DECL) return;
    consts_walk(&n->left); consts_walk(&n->right); consts_walk(&n->cond);
    consts_walk(&n->then_branch); consts_walk(&n->else_branch); consts_walk(&n->body);
    consts_walk(&n->step); consts_walk(&n->update); consts_walk(&n->init);
    consts_walk(&n->init_stmt); consts_walk(&n->cond_expr);
    for (size_t i = 0; i < n->arg_count; i++) consts_walk(&n->args[i]);
    for (size_t i = 0; i < n->item_count; i++) consts_walk(&n->items[i]);
}

static void consts_resolve(ASTNode *prog) {
    free(g_consts); g_consts = NULL; g_nconsts = 0;
    free(g_pkgnames); g_pkgnames = NULL; g_npkgs = 0;
    size_t n = 0;
    for (size_t i = 0; i < prog->item_count; i++) if (prog->items[i]) n++;
    g_consts = calloc(n ? n : 1, sizeof(ConstEnt));
    g_pkgnames = calloc(n ? n : 1, sizeof(char *));
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (!d) continue;
        if (d->type == AST_PACKAGE && d->name) g_pkgnames[g_npkgs++] = d->name;
        if (d->type == AST_CONST_DECL && d->name && d->init) {
            g_consts[g_nconsts].name = d->name;
            g_consts[g_nconsts].package = d->package;
            g_consts[g_nconsts].init = d->init;
            g_nconsts++;
        }
    }
    if (g_nconsts == 0) return;
    for (size_t i = 0; i < prog->item_count; i++) consts_walk(&prog->items[i]);
}

int typecheck_program(ASTNode *prog) {
    errors = 0;
    warnings = 0;
    alias_clear();
    free(g_capkinds); g_capkinds = NULL;
    if (!prog) return 0;
    consts_resolve(prog);
    /* register type aliases so they resolve in every later pass */
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (d && d->type == AST_TYPE_DECL && d->name && d->alias_type) alias_add(d->name, d->alias_type);
    }
    /* register declared capability kinds (`@capability(...)`) */
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (d && d->type == AST_CAP_DECL) capkind_add(d->caps);
    }
    /* register functions, and structs so field lookups work wherever they are declared */
    type_registry_clear();
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (d && d->type == AST_FUNC_DECL) register_func(d);
        /* `extern` prototypes are checked at call sites too (arity, argument types) */
        if (d && d->type == AST_EXTERN_DECL && d->name) register_func(d);
        if (d && d->type == AST_STRUCT_DECL && d->name) type_registry_add(d->name, d);
    }
    /* register top-level symbols with their package + visibility */
    symvis_clear();
    g_cur_pkg = NULL;
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (!d || !d->name) continue;
        if (d->type == AST_FUNC_DECL || d->type == AST_VAR_DECL ||
            d->type == AST_STRUCT_DECL || d->type == AST_EXTERN_DECL)
            symvis_add(d->name, d->package, d->visibility);
    }
    scope_push(); /* global */
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (!d) continue;
        g_cur_pkg = d->package;
        if (d->type == AST_VAR_DECL) {
            /* global var */
            if (d->init) {
                Type *it = type_of_expr(d->init);
                if (d->ty && it && !type_equal(d->ty, it) && !lit_fits(d->ty, d->init)) {
                    char tb[96], ib[96];
                    type_name(d->ty, tb, sizeof(tb));
                    type_name(it, ib, sizeof(ib));
                    add_error("cannot initialize %s '%s' with a value of type %s",
                              tb, d->name ? d->name : "?", ib);
                } else if (d->ty && it) {
                    char phrase[160];
                    snprintf(phrase, sizeof(phrase), "global variable '%s' init", d->name ? d->name : "?");
                    check_assign_effects(d->ty, it, phrase);
                }
            }
            if (d->name) scope_add(d->name, d->ty);
        } else if (d->type == AST_FUNC_DECL) {
            scope_push();
            for (size_t j = 0; j < d->arg_count; j++) {
                ASTNode *p = d->args[j];
                if (p && p->name) {
                    if (p->ty && p->ty->kind == TY_CAPABILITY) {
                        cap_validate_type(p->ty, "parameter");
                        cap_bind_add(p->name, p->ty, p->ty->cap_borrow ? 0 : 1, 1, p->line, p->column);
                    } else {
                        scope_add(p->name, p->ty);
                    }
                }
            }
            char *saved_effects = current_function_effects;
            char *saved_needs = current_function_needs;
            char *saved_gives = current_function_gives;
            ASTNode *saved_node = current_function_node;
            Type *saved_ret = current_function_return_type;
            const char *saved_name = current_function_name;
            current_function_effects = d->effects ? d->effects : NULL;
            current_function_needs = d->needs ? d->needs : NULL;
            current_function_gives = d->gives ? d->gives : NULL;
            current_function_node = d;
            current_function_return_type = d->ty;
            current_function_name = d->name;
            check_stmt(d->body);
            current_function_effects = saved_effects;
            current_function_needs = saved_needs;
            current_function_gives = saved_gives;
            current_function_node = saved_node;
            current_function_return_type = saved_ret;
            current_function_name = saved_name;
            scope_pop();
        }
    }
    scope_pop();
    int result = errors == 0;
    type_registry_clear();
    alias_clear();
    g_cur_pkg = NULL;
    return result;
}