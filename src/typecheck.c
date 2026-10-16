#include "typecheck.h"
#include "ast.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>

static int errors = 0;
static int cur_line = 0, cur_col = 0;

typedef struct Scope {
    char **names;
    Type **types;
    size_t count, cap;
    struct Scope *parent;
} Scope;

static Scope *scope = NULL;

static void scope_push(void) {
    Scope *s = calloc(1, sizeof(Scope));
    s->parent = scope;
    scope = s;
}
static void scope_pop(void) { if (scope) scope = scope->parent; }

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

/* function table */
typedef struct { char *name; Type *return_type; size_t param_count; Type **param_types; ASTNode *node; char *effects; } FuncInfo;
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

/* ---- type alias registry (`type X = ...`) ---- */
typedef struct { char *name; Type *type; } AliasEnt;
static AliasEnt g_aliases[128];
static int g_nalias = 0;

static void alias_add(const char *name, Type *t) {
    if (!name || !t || g_nalias >= 128) return;
    for (int i = 0; i < g_nalias; i++) {
        if (strcmp(g_aliases[i].name, name) == 0) { g_aliases[i].type = t; return; }
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
static int type_equal(Type *a, Type *b) {
    if (!a || !b) return 1; /* unknown allowed */
    a = resolve_type(a); b = resolve_type(b);
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
        default:
            buf_cat(buf, cap, "?");
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
static void check_call_args(const char *what, size_t argc, ASTNode **argv,
                            size_t pcount, Type **ptypes) {
    Type **ats = calloc(argc ? argc : 1, sizeof(Type *));
    for (size_t i = 0; i < argc; i++) ats[i] = type_of_expr(argv[i]);
    if (pcount != argc) add_error("%s expects %zu argument(s), got %zu", what, pcount, argc);
    for (size_t i = 0; i < argc && i < pcount; i++) {
        if (ats[i] && ptypes[i] && !type_equal(ats[i], ptypes[i])) {
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

static Type *number_type(const char *lex) {
    if (!lex) return NULL;
    if (strchr(lex, '.') || strchr(lex, 'e') || strchr(lex, 'E')) {
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
            if (!t) {
                /* a function name used as a value (e.g. passed as a callback):
                   give it its function type so assignments/calls are checked */
                for (size_t i = 0; i < func_count; i++)
                    if (e->name && strcmp(funcs[i].name, e->name) == 0) return func_type_of(&funcs[i]);
                add_error("undeclared identifier '%s'", e->name?e->name:"?");
            }
            return t;
        }
        case AST_EXPR_UNARY: {
            if (e->op_token == TOK_AMP) {
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
                if (lt && rt && !type_equal(lt, rt)) {
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
            FuncInfo *finfo = NULL;
            for (size_t i = 0; i < func_count; i++) {
                if (name && strcmp(funcs[i].name, name) == 0) { finfo = &funcs[i]; break; }
            }
            if (finfo) {
                char what[160];
                snprintf(what, sizeof(what), "call to '%s'", name);
                check_call_args(what, e->arg_count, e->args, finfo->param_count, finfo->param_types);
                if (finfo->effects && !effects_cover(current_function_effects, finfo->effects)) {
                    add_error("call to '%s' requires effect(s) '%s', not declared by '%s'",
                              name, finfo->effects, enclosing_name());
                }
                return finfo->return_type;
            }
            /* indirect call through a function-pointer value */
            if (name) {
                Type *ft = resolve_type(scope_lookup(name));
                if (ft && ft->kind == TY_FUNCTION) {
                    char what[160];
                    snprintf(what, sizeof(what), "call through '%s'", name);
                    check_call_args(what, e->arg_count, e->args, ft->param_count, ft->param_types);
                    if (ft->effects && !effects_cover(current_function_effects, ft->effects)) {
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
            if (lt && lt->kind == TY_NAMED && lt->name) {
                ASTNode *sd = type_registry_find(lt->name);
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
            check_stmt(s->body);
            scope_pop();
            break;
        case AST_VAR_DECL: {
            Type *t = s->ty;
            if (s->init) {
                Type *it = type_of_expr(s->init);
                if (t && it && !type_equal(t, it)) {
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
            if (s->name) scope_add(s->name, t);
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
        case AST_RETURN:
            if (s->left) {
                Type *rt = type_of_expr(s->left);
                /* Returning a function value must not widen its effects beyond
                   the function's declared return type. */
                if (rt) check_assign_effects(current_function_return_type, rt, "return value");
            }
            break;
        case AST_EXPR_STMT:
            type_of_expr(s->left);
            break;
        case AST_CONTRACT:
            /* compile-time contract: best-effort, skip for now */
            break;
        default:
            break;
    }
}

int typecheck_program(ASTNode *prog) {
    errors = 0;
    alias_clear();
    if (!prog) return 0;
    /* register type aliases so they resolve in every later pass */
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (d && d->type == AST_TYPE_DECL && d->name && d->alias_type) alias_add(d->name, d->alias_type);
    }
    /* register functions */
    for (size_t i = 0; i < prog->item_count; i++) {
        if (prog->items[i] && prog->items[i]->type == AST_FUNC_DECL) register_func(prog->items[i]);
    }
    scope_push(); /* global */
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (!d) continue;
        if (d->type == AST_VAR_DECL) {
            /* global var */
            if (d->init) {
                Type *it = type_of_expr(d->init);
                if (d->ty && it && !type_equal(d->ty, it)) {
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
                if (p && p->name) scope_add(p->name, p->ty);
            }
            char *saved_effects = current_function_effects;
            Type *saved_ret = current_function_return_type;
            const char *saved_name = current_function_name;
            current_function_effects = d->effects ? d->effects : NULL;
            current_function_return_type = d->ty;
            current_function_name = d->name;
            check_stmt(d->body);
            current_function_effects = saved_effects;
            current_function_return_type = saved_ret;
            current_function_name = saved_name;
            scope_pop();
        } else if (d->type == AST_STRUCT_DECL) {
            if (d->name) type_registry_add(d->name, d);
        }
    }
    scope_pop();
    int result = errors == 0;
    type_registry_clear();
    alias_clear();
    return result;
}