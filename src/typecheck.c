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

/* type representation checks */
static int type_equal(Type *a, Type *b) {
    if (!a || !b) return 1; /* unknown allowed */
    if (a->kind != b->kind) return 0;
    if (a->name && b->name && strcmp(a->name, b->name) != 0) return 0;
    return 1;
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

static Type *type_of_expr(ASTNode *e);

static Type *number_type(const char *lex) {
    if (!lex) return NULL;
    if (strchr(lex, '.') || strchr(lex, 'e') || strchr(lex, 'E')) {
        Type *t = type_new(TY_PRIMITIVE); t->name = strdup("double"); return t;
    }
    Type *t = type_new(TY_PRIMITIVE); t->name = strdup("int"); return t;
}

static void check_stmt(ASTNode *s);

static Type *type_of_expr(ASTNode *e) {
    if (!e) return NULL;
    cur_line = e->line; cur_col = e->column;
    switch (e->type) {
        case AST_EXPR_NUMBER: return number_type(e->str_value);
        case AST_EXPR_STRING: { Type *t=type_new(TY_PRIMITIVE); t->name=strdup("ptr"); return t; }
        case AST_EXPR_CHAR: { Type *t=type_new(TY_PRIMITIVE); t->name=strdup("char"); return t; }
        case AST_EXPR_IDENT: {
            Type *t = scope_lookup(e->name);
            if (!t) {
                /* a function name used as a value (e.g. passed as a callback);
                   function values are not modeled, so accept without a type */
                for (size_t i = 0; i < func_count; i++)
                    if (e->name && strcmp(funcs[i].name, e->name) == 0) return NULL;
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
                if (lt && rt && !type_equal(lt, rt)) add_error("assignment type mismatch");
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
            if (lt && rt && !type_equal(lt, rt)) add_error("binary operand type mismatch");
            return lt ? lt : rt;
        }
        case AST_EXPR_TERNARY: {
            (void)type_of_expr(e->cond);
            Type *tt = type_of_expr(e->then_branch);
            Type *et = type_of_expr(e->else_branch);
            if (tt && et && !type_equal(tt, et)) add_error("ternary branch types differ");
            return tt ? tt : et;
        }
        /* helper: check if caller effects cover callee effects */

        case AST_EXPR_CALL: {
            const char *name = (e->left && e->left->type == AST_EXPR_IDENT) ? e->left->name : NULL;
            FuncInfo *finfo = NULL;
            for (size_t i = 0; i < func_count; i++) {
                if (name && strcmp(funcs[i].name, name) == 0) { finfo = &funcs[i]; break; }
            }
            if (finfo) {
                if (finfo->param_count != e->arg_count) add_error("call '%s' expects %zu args, got %zu", name, finfo->param_count, e->arg_count);
                for (size_t i = 0; i < e->arg_count && i < finfo->param_count; i++) {
                    Type *at = type_of_expr(e->args[i]);
                    if (at && finfo->param_types[i] && !type_equal(at, finfo->param_types[i])) add_error("arg %zu type mismatch in call '%s'", i+1, name);
                }
                /* effects checking */
                if (finfo->effects && !effects_cover(current_function_effects, finfo->effects)) {
                    add_error("call '%s' requires effects '%s' not present in caller", name, finfo->effects);
                }
                return finfo->return_type ? finfo->return_type : NULL;
            }
            /* extern: check args recursively */
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
                if (t && it && !type_equal(t, it)) add_error("variable '%s' init type mismatch", s->name ? s->name : "?");
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
            if (s->left) type_of_expr(s->left);
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
    if (!prog) return 0;
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
            if (d->init) type_of_expr(d->init);
            if (d->name) scope_add(d->name, d->ty);
        } else if (d->type == AST_FUNC_DECL) {
            scope_push();
            for (size_t j = 0; j < d->arg_count; j++) {
                ASTNode *p = d->args[j];
                if (p && p->name) scope_add(p->name, p->ty);
            }
            char *saved_effects = current_function_effects;
            current_function_effects = d->effects ? d->effects : NULL;
            check_stmt(d->body);
            current_function_effects = saved_effects;
            scope_pop();
        } else if (d->type == AST_STRUCT_DECL) {
            if (d->name) type_registry_add(d->name, d);
        }
    }
    scope_pop();
    int result = errors == 0;
    type_registry_clear();
    return result;
}