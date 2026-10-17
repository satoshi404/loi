#include "codegen.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ---------- data structures ---------- */
typedef struct {
    char *name;
    int field_count;
    char **field_names;
    char **field_types;
    int align;
    int packed;
} StructInfo;

static StructInfo struct_infos[64];
static int struct_info_count = 0;

static StructInfo *find_struct(const char *name) {
    for (int i = 0; i < struct_info_count; i++) {
        if (name && strcmp(struct_infos[i].name, name) == 0) return &struct_infos[i];
    }
    return NULL;
}

/* type aliases (`type X = ...`) — borrowed from the AST, which outlives codegen */
typedef struct { char *name; Type *ty; } AliasInfo;
static AliasInfo cg_aliases[64];
static int cg_alias_count = 0;

static Type *cg_resolve_type(Type *t) {
    for (int i = 0; i < 16 && t && t->name &&
         (t->kind == TY_NAMED || t->kind == TY_PRIMITIVE); i++) {
        int found = 0;
        for (int j = 0; j < cg_alias_count; j++) {
            if (strcmp(cg_aliases[j].name, t->name) == 0) { t = cg_aliases[j].ty; found = 1; break; }
        }
        if (!found) break;
    }
    return t;
}

typedef struct {
    char *name;
    char *ir;
    char *type;
    Type *asty;
} CGVar;

typedef struct CGScope {
    CGVar *vars;
    size_t count, cap;
    struct CGScope *parent;
} CGScope;

typedef struct {
    char *name;
    char *retty;
    char **param_types;
    size_t param_count;
    int defined;
    char *link_name;   /* @attr(link_name("...")) — emitted symbol, or NULL */
    unsigned attrs;    /* CGATTR_* flags from @attr(...) */
} CGFunc;

#define CGATTR_INLINE   1u
#define CGATTR_NORETURN 2u
#define CGATTR_PURE     4u
#define CGATTR_MUST_USE 8u

typedef struct {
    char *text;
    size_t len, cap;
} GStr;

typedef struct {
    char **items;
    size_t count, cap;
} StrList;

typedef struct {
    FILE *out;
    int tmp_counter;
    int label_counter;
    int terminated;        /* current block has terminator */
    char *current_retty;
    CGScope *scope;
    CGFunc *funcs;
    size_t func_count, func_cap;
    StrList strings;      /* global strings */
    StrList string_names;
    StrList string_values;
    /* loop label stack for break/continue */
    char **loop_exit_labels;
    char **loop_cont_labels;
    char **loop_update_labels;
    size_t loop_depth;
    size_t loop_cap;
} CG;

static void *xmalloc(size_t n) { void *p = malloc(n); if (!p) abort(); return p; }

/* ---------- scopes ---------- */
static void scope_push(CG *cg) {
    CGScope *s = xmalloc(sizeof(CGScope));
    memset(s, 0, sizeof(CGScope));
    s->parent = cg->scope;
    cg->scope = s;
}
static void scope_pop(CG *cg) { if (cg->scope) cg->scope = cg->scope->parent; }
static CGVar *find_var(CG *cg, const char *name) {
    for (CGScope *s = cg->scope; s; s = s->parent) {
        for (size_t i = 0; i < s->count; i++) {
            if (strcmp(s->vars[i].name, name) == 0) return &s->vars[i];
        }
    }
    return NULL;
}
static void add_var(CG *cg, const char *name, const char *ir, const char *type, Type *asty) {
    CGScope *s = cg->scope;
    if (s->count == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->vars = realloc(s->vars, s->cap * sizeof(CGVar));
    }
    s->vars[s->count].name = strdup(name);
    s->vars[s->count].ir = strdup(ir);
    s->vars[s->count].type = strdup(type);
    s->vars[s->count].asty = asty;
    s->count++;
}

/* ---------- function table ---------- */
static void add_func(CG *cg, const char *name, const char *retty, char **ptypes, size_t pc, int defined) {
    if (cg->func_count == cg->func_cap) {
        cg->func_cap = cg->func_cap ? cg->func_cap * 2 : 8;
        cg->funcs = realloc(cg->funcs, cg->func_cap * sizeof(CGFunc));
    }
    cg->funcs[cg->func_count].name = strdup(name);
    cg->funcs[cg->func_count].retty = strdup(retty);
    cg->funcs[cg->func_count].param_types = ptypes;
    cg->funcs[cg->func_count].param_count = pc;
    cg->funcs[cg->func_count].defined = defined;
    cg->funcs[cg->func_count].link_name = NULL;
    cg->funcs[cg->func_count].attrs = 0;
    cg->func_count++;
}
static CGFunc *find_func(CG *cg, const char *name) {
    for (size_t i = 0; i < cg->func_count; i++) {
        if (strcmp(cg->funcs[i].name, name) == 0) return &cg->funcs[i];
    }
    return NULL;
}

/* ---------- string globals ---------- */
static char *get_string_global(CG *cg, const char *val) {
    /* find existing */
    for (size_t i = 0; i < cg->string_values.count; i++) {
        if (strcmp(cg->string_values.items[i], val) == 0) return cg->string_names.items[i];
    }
    /* add */
    char name[64];
    snprintf(name, sizeof(name), "@.str.%zu", cg->string_values.count);
    char **pn = realloc(cg->string_names.items, (cg->string_values.count + 1) * sizeof(char *));
    cg->string_names.items = pn;
    char **pv = realloc(cg->string_values.items, (cg->string_values.count + 1) * sizeof(char *));
    cg->string_values.items = pv;
    cg->string_names.items[cg->string_values.count] = strdup(name);
    cg->string_values.items[cg->string_values.count] = strdup(val);
    cg->string_values.count++;
    cg->string_names.count = cg->string_values.count;
    return cg->string_names.items[cg->string_values.count - 1];
}
static void collect_strings_expr(CG *cg, ASTNode *e);
static void collect_strings_stmt(CG *cg, ASTNode *s);
static void collect_strings_block(CG *cg, ASTNode *b) {
    if (!b) return;
    for (size_t i = 0; i < b->item_count; i++) collect_strings_stmt(cg, b->items[i]);
}
static void collect_strings_stmt(CG *cg, ASTNode *s) {
    if (!s) return;
    switch (s->type) {
        case AST_BLOCK: collect_strings_block(cg, s); break;
        case AST_FUNC_DECL: collect_strings_block(cg, s->body); break;
        case AST_VAR_DECL: collect_strings_expr(cg, s->init); break;
        case AST_IF: collect_strings_expr(cg, s->cond); collect_strings_block(cg, s->then_branch); collect_strings_block(cg, s->else_branch); break;
        case AST_WHILE: collect_strings_expr(cg, s->cond); collect_strings_block(cg, s->body); break;
        case AST_FOR: collect_strings_stmt(cg, s->init_stmt); collect_strings_expr(cg, s->cond_expr); collect_strings_stmt(cg, s->update); collect_strings_block(cg, s->body); break;
        case AST_REGION: collect_strings_expr(cg, s->cond); collect_strings_block(cg, s->body); break;
        case AST_RETURN: collect_strings_expr(cg, s->left); break;
        case AST_EXPR_STMT: collect_strings_expr(cg, s->left); break;
        default: collect_strings_expr(cg, s); break;
    }
}
static void collect_strings_expr(CG *cg, ASTNode *e) {
    if (!e) return;
    switch (e->type) {
        case AST_EXPR_STRING: get_string_global(cg, e->str_value ? e->str_value : ""); break;
        case AST_EXPR_BINARY: collect_strings_expr(cg, e->left); collect_strings_expr(cg, e->right); break;
        case AST_EXPR_UNARY: collect_strings_expr(cg, e->left); break;
        case AST_EXPR_TERNARY: collect_strings_expr(cg, e->cond); collect_strings_expr(cg, e->then_branch); collect_strings_expr(cg, e->else_branch); break;
        case AST_EXPR_CALL: collect_strings_expr(cg, e->left); for (size_t i=0;i<e->arg_count;i++) collect_strings_expr(cg, e->args[i]); break;
        case AST_EXPR_INDEX: collect_strings_expr(cg, e->left); collect_strings_expr(cg, e->right); break;
        case AST_EXPR_FIELD: collect_strings_expr(cg, e->left); break;
        default: break;
    }
}

static int is_assign_op_cg(TokenType t) {
    return t == TOK_ASSIGN || t == TOK_PLUSEQ || t == TOK_MINUSEQ ||
           t == TOK_STAREQ || t == TOK_SLASHEQ || t == TOK_PERCENTEQ ||
           t == TOK_AMPEQ || t == TOK_PIPEEQ || t == TOK_CARETEQ ||
           t == TOK_LSHIFTEQ || t == TOK_RSHIFTEQ;
}
/* ---------- helpers ---------- */
static char *new_tmp(CG *cg) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%%t%d", ++cg->tmp_counter);
    return strdup(buf);
}
static char *new_label(CG *cg, const char *prefix) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%s%d", prefix, ++cg->label_counter);
    return strdup(buf);
}

static char *llvm_type(Type *t) {
    if (!t) return strdup("i32");
    t = cg_resolve_type(t);
    /* Lexical capabilities are erased: they carry no runtime data and lower to
       a bare i8 (a placeholder the optimizer removes). See docs/capabilities.md. */
    if (t->kind == TY_CAPABILITY) return strdup("i8");
    if (t->kind == TY_POINTER || t->kind == TY_FUNCTION) return strdup("ptr");
    if (t->kind == TY_ARRAY) {
        char *base = llvm_type(t->base);
        int n = 1;
        if (t->array_size && t->array_size->str_value) n = atoi(t->array_size->str_value);
        char buf[128];
        snprintf(buf, sizeof(buf), "[%d x %s]", n, base);
        free(base);
        return strdup(buf);
    }
    const char *n = t->name ? t->name : "?";
    for (int si = 0; si < struct_info_count; si++) {
        if (strcmp(struct_infos[si].name, n) == 0) {
            char buf[128];
            snprintf(buf, sizeof(buf), "%%struct.%s", n);
            return strdup(buf);
        }
    }
    if (strcmp(n, "ptr") == 0) return strdup("ptr");
    if (strcmp(n, "int") == 0) return strdup("i32");
    if (strcmp(n, "float") == 0) return strdup("float");
    if (strcmp(n, "double") == 0) return strdup("double");
    if (strcmp(n, "char") == 0) return strdup("i8");
    if (strcmp(n, "bool") == 0) return strdup("i1");
    if (strcmp(n, "byte") == 0) return strdup("i8");
    if (strcmp(n, "short") == 0) return strdup("i16");
    if (strcmp(n, "long") == 0) return strdup("i64");
    if (strcmp(n, "void") == 0) return strdup("void");
    if (strstr(n, "unsigned")) {
        if (strstr(n, "char")) return strdup("i8");
        if (strstr(n, "short")) return strdup("i16");
        if (strstr(n, "long")) return strdup("i64");
        return strdup("i32");
    }
    if (strstr(n, "signed")) {
        if (strstr(n, "char")) return strdup("i8");
        if (strstr(n, "short")) return strdup("i16");
        if (strstr(n, "long")) return strdup("i64");
        return strdup("i32");
    }
    if (strcmp(n, "?") == 0) return strdup("i32");
    return strdup("ptr");
}

static int is_float_type(const char *t) { return strcmp(t, "float") == 0 || strcmp(t, "double") == 0; }

static char *cast_value(CG *cg, const char *val, const char *from, const char *to) {
    if (strcmp(from, to) == 0) return strdup(val);
    if (strcmp(to, "ptr") == 0) {
        if (strcmp(val, "0") == 0) return strdup("null");
        if (from[0] == 'i') {
            char *r = new_tmp(cg);
            fprintf(cg->out, "  %s = inttoptr %s %s to ptr\n", r, from, val);
            return r;
        }
    }
    if (strcmp(from, "ptr") == 0 && to[0] == 'i') {
        char *r = new_tmp(cg);
        fprintf(cg->out, "  %s = ptrtoint ptr %s to %s\n", r, val, to);
        return r;
    }
    if (is_float_type(from) && is_float_type(to)) {
        /* float->double or double->float */
        char *r = new_tmp(cg);
        if (strcmp(from, "float") == 0 && strcmp(to, "double") == 0) {
            fprintf(cg->out, "  %s = fpext float %s to double\n", r, val);
        } else {
            fprintf(cg->out, "  %s = fptrunc double %s to float\n", r, val);
        }
        return r;
    }
    if (is_float_type(from) && !is_float_type(to)) {
        char *r = new_tmp(cg);
        fprintf(cg->out, "  %s = fptosi %s %s to %s\n", r, from, val, to);
        return r;
    }
    if (!is_float_type(from) && is_float_type(to)) {
        char *r = new_tmp(cg);
        fprintf(cg->out, "  %s = sitofp %s %s to %s\n", r, from, val, to);
        return r;
    }
    if (from[0]=='i' && to[0]=='i') {
        int fw = atoi(from+1);
        int tw = atoi(to+1);
        if (fw > 0 && tw > 0 && fw < tw) {
            char *r = new_tmp(cg);
            /* i1 is a boolean: zero-extend so true == 1, not -1 */
            const char *op = (fw == 1) ? "zext" : "sext";
            fprintf(cg->out, "  %s = %s %s %s to %s\n", r, op, from, val, to);
            return r;
        }
        if (fw > 0 && tw > 0 && fw > tw) {
            char *r = new_tmp(cg);
            fprintf(cg->out, "  %s = trunc %s %s to %s\n", r, from, val, to);
            return r;
        }
    }
    /* default: just return val */
    return strdup(val);
    return strdup(val);
}

/* ---------- forward declarations ---------- */
static char *codegen_expr(CG *cg, ASTNode *e, char **out_type);
static long type_size_llvm(const char *t);
static long type_align_llvm(const char *t);
static long struct_size_align(const char *name, long *align_out);
static long get_size(Type *t);
static long get_align(Type *t);
static long struct_offset(Type *t, const char *field);
static void codegen_stmt(CG *cg, ASTNode *s);

/* ---------- expr codegen ---------- */
static char *char_to_int(const char *lex) {
    /* lex includes quotes; compute basic */
    if (!lex || strlen(lex) < 2) return strdup("0");
    if (lex[0] == '\'') {
        /* handle simple and escapes */
        if (lex[1] == '\\') {
            char e = lex[2];
            switch (e) {
                case 'n': return strdup("10");
                case 't': return strdup("9");
                case 'r': return strdup("13");
                case '\\': return strdup("92");
                case '\'': return strdup("39");
                case '0': return strdup("0");
                default: {
                    static char b[16]; snprintf(b, sizeof(b), "%d", e); return strdup(b);
                }
            }
        } else {
            static char b[16]; snprintf(b, sizeof(b), "%d", (unsigned char)lex[1]); return strdup(b);
        }
    }
    return strdup("0");
}

static char *codegen_expr(CG *cg, ASTNode *e, char **out_type) {
    if (!e) { if (out_type) *out_type = strdup("i32"); return strdup("0"); }
    if (e->type == AST_EXPR_SIZEOF) {
        long sz = get_size(e->ty);
        if (out_type) *out_type = strdup("i32");
        char buf[32]; snprintf(buf, sizeof(buf), "%ld", sz);
        return strdup(buf);
    }
    if (e->type == AST_EXPR_ALIGNOF) {
        long sz = get_align(e->ty);
        if (out_type) *out_type = strdup("i32");
        char buf[32]; snprintf(buf, sizeof(buf), "%ld", sz);
        return strdup(buf);
    }
    if (e->type == AST_EXPR_OFFSETOF) {
        long off = struct_offset(e->ty, e->name);
        if (out_type) *out_type = strdup("i32");
        char buf[32]; snprintf(buf, sizeof(buf), "%ld", off);
        return strdup(buf);
    }
    if (e->type == AST_EXPR_NUMBER) {
        const char *lex = e->str_value ? e->str_value : "0";
        /* detect float */
        int isf = (strchr(lex, '.') || strchr(lex, 'e') || strchr(lex, 'E') || strchr(lex, 'x') || strchr(lex,'X') || strchr(lex,'b') || strchr(lex,'B'));
        /* hex/binary ints are not float */
        if (strchr(lex, 'x') || strchr(lex, 'X') || strchr(lex, 'b') || strchr(lex, 'B')) isf = 0;
        if (isf) {
            if (out_type) *out_type = strdup("double");
            return strdup(lex);
        } else {
            if (out_type) *out_type = strdup("i32");
            /* normalize hex/binary? LLVM supports 0xFF? yes hex integer literal with 0x */
            return strdup(lex);
        }
    }
    if (e->type == AST_EXPR_CHAR) {
        if (out_type) *out_type = strdup("i8");
        return char_to_int(e->str_value);
    }
    if (e->type == AST_EXPR_NULL) {
        if (out_type) *out_type = strdup("ptr");
        return strdup("null");
    }
    if (e->type == AST_EXPR_STRING) {
        char *g = get_string_global(cg, e->str_value ? e->str_value : "");
        if (out_type) *out_type = strdup("ptr");
        return strdup(g);
    }
    if (e->type == AST_EXPR_IDENT) {
        if (strcmp(e->name ? e->name : "", "") == 0) {
            if (out_type) *out_type = strdup("i32");
            return strdup("0");
        }
        CGVar *v = find_var(cg, e->name);
        if (v) {
            char *ld = new_tmp(cg);
            fprintf(cg->out, "  %s = load %s, ptr %s\n", ld, v->type, v->ir);
            if (out_type) *out_type = strdup(v->type);
            return ld;
        }
        /* function reference */
        if (find_func(cg, e->name)) {
            if (out_type) *out_type = strdup("ptr");
            char *buf = malloc(strlen(e->name)+2);
            sprintf(buf, "@%s", e->name);
            return buf;
        }
        /* unknown var -> 0 */
        if (out_type) *out_type = strdup("i32");
        return strdup("0");
    }
    if (e->type == AST_EXPR_UNARY) {
        if (e->op_token == TOK_BANG) {
            char *t;
            char *ov = codegen_expr(cg, e->left, &t);
            char *cmp = new_tmp(cg);
            if (is_float_type(t)) {
                fprintf(cg->out, "  %s = fcmp oeq %s %s, 0.0\n", cmp, t, ov);
            } else {
                fprintf(cg->out, "  %s = icmp eq %s %s, 0\n", cmp, t, ov);
            }
            if (out_type) *out_type = strdup("i1");
            return cmp;
        }
        if (e->op_token == TOK_MINUS) {
            char *t;
            char *ov = codegen_expr(cg, e->left, &t);
            char *r = new_tmp(cg);
            if (is_float_type(t)) {
                fprintf(cg->out, "  %s = fneg %s %s\n", r, t, ov);
            } else {
                fprintf(cg->out, "  %s = sub %s 0, %s\n", r, t, ov);
            }
            if (out_type) *out_type = strdup(t);
            return r;
        }
        if (e->op_token == TOK_TILDE) {
            char *t;
            char *ov = codegen_expr(cg, e->left, &t);
            char *r = new_tmp(cg);
            fprintf(cg->out, "  %s = xor %s %s, -1\n", r, t, ov);
            if (out_type) *out_type = strdup(t);
            return r;
        }
        if (e->op_token == TOK_AMP) {
            if (e->left && e->left->type == AST_EXPR_IDENT) {
                CGVar *v = find_var(cg, e->left->name);
                if (v) {
                    if (out_type) *out_type = strdup("ptr");
                    return strdup(v->ir);
                }
            }
            if (out_type) *out_type = strdup("ptr");
            return strdup("null");
        }
        if (e->op_token == TOK_STAR) {
            char *t;
            char *pv = codegen_expr(cg, e->left, &t);
            char *ld = new_tmp(cg);
            const char *bt = "i32";
            fprintf(cg->out, "  %s = load %s, ptr %s\n", ld, bt, pv);
            if (out_type) *out_type = strdup(bt);
            return ld;
        }
        /* others: deref/amper fallback */
        {
            char *t;
            char *ov = codegen_expr(cg, e->left, &t);
            if (out_type) *out_type = strdup(t);
            return ov;
        }
    }
    if (e->type == AST_EXPR_BINARY) {
        TokenType op = e->op_token;
        /* assignment */
        if (is_assign_op_cg(op) && e->left && e->left->type == AST_EXPR_INDEX &&
            e->left->left && e->left->left->type == AST_EXPR_IDENT) {
            CGVar *v = find_var(cg, e->left->left->name);
            ASTNode *idxnode = e->left->right;
            char *it = NULL; char *iv = codegen_expr(cg, idxnode, &it);
            char *idx = cast_value(cg, iv, it ? it : "i32", "i32");
            char *base_type = (v && v->asty && v->asty->kind == TY_ARRAY) ? llvm_type(v->asty->base) : strdup("i32");
            char *gep = new_tmp(cg);
            fprintf(cg->out, "  %s = getelementptr inbounds %s, ptr %s, i32 0, i32 %s\n", gep, v ? v->type : "i32", v ? v->ir : "null", idx);
            /* rhs */
            char *rt = NULL;
            char *rv = codegen_expr(cg, e->right, &rt);
            char *cv = cast_value(cg, rv, rt ? rt : "i32", base_type);
            /* compound? simple assign for now; compound can extend */
            if (op != TOK_ASSIGN) {
                char *ld = new_tmp(cg);
                fprintf(cg->out, "  %s = load %s, ptr %s\n", ld, base_type, gep);
                /* map to base op? keep assign-only for index */
                cv = rv;
            }
            fprintf(cg->out, "  store %s %s, ptr %s\n", base_type, cv, gep);
            if (out_type) *out_type = strdup(base_type);
            return cv ? cv : strdup("0");
        }
        if (is_assign_op_cg(op) && e->left && e->left->type == AST_EXPR_FIELD &&
            e->left->left && e->left->left->type == AST_EXPR_IDENT) {
            CGVar *v = find_var(cg, e->left->left->name);
            StructInfo *si = v && v->asty && v->asty->name ? find_struct(v->asty->name) : NULL;
            int idx = -1;
            if (si) {
                for (int i = 0; i < si->field_count; i++) {
                    if (e->left->name && strcmp(si->field_names[i], e->left->name) == 0) { idx = i; break; }
                }
            }
            if (si && idx >= 0) {
                char *field_type = si->field_types[idx];
                char *gep = new_tmp(cg);
                fprintf(cg->out, "  %s = getelementptr inbounds %s, ptr %s, i32 0, i32 %d\n", gep, v->type, v->ir, idx);
                char *rt = NULL;
                char *rv = codegen_expr(cg, e->right, &rt);
                char *cv = cast_value(cg, rv, rt ? rt : "i32", field_type);
                fprintf(cg->out, "  store %s %s, ptr %s\n", field_type, cv, gep);
                if (out_type) *out_type = strdup(field_type);
                return cv ? cv : strdup("0");
            }
        }
        if (is_assign_op_cg(op) && e->left && e->left->type == AST_EXPR_UNARY &&
            e->op_token == TOK_ASSIGN && e->left->op_token == TOK_STAR) {
            char *pt = NULL;
            char *ptrv = codegen_expr(cg, e->left->left, &pt);
            char *rt = NULL;
            char *rv = codegen_expr(cg, e->right, &rt);
            char *cv = cast_value(cg, rv, rt ? rt : "i32", "i32");
            fprintf(cg->out, "  store i32 %s, ptr %s\n", cv, ptrv);
            if (out_type) *out_type = strdup("i32");
            return cv ? cv : strdup("0");
        }
        if (is_assign_op_cg(op) && e->left && e->left->type == AST_EXPR_IDENT) {
            const char *name = e->left->name;
            CGVar *v = find_var(cg, name);
            char *vtype = v ? v->type : strdup("i32");
            /* for compound, load current */
            char *cur = NULL;
            if (op != TOK_ASSIGN) {
                if (v) {
                    char *ld = new_tmp(cg);
                    fprintf(cg->out, "  %s = load %s, ptr %s\n", ld, v->type, v->ir);
                    cur = ld;
                } else cur = strdup("0");
            }
            char *rt = NULL;
            char *rv = codegen_expr(cg, e->right, &rt);
            /* map compound op to base arithmetic */
            TokenType base = op;
            if (op == TOK_PLUSEQ) base = TOK_PLUS;
            if (op == TOK_MINUSEQ) base = TOK_MINUS;
            if (op == TOK_STAREQ) base = TOK_STAR;
            if (op == TOK_SLASHEQ) base = TOK_SLASH;
            if (op == TOK_PERCENTEQ) base = TOK_PERCENT;
            if (op == TOK_AMPEQ) base = TOK_AMP;
            if (op == TOK_PIPEEQ) base = TOK_PIPE;
            if (op == TOK_CARETEQ) base = TOK_CARET;
            if (op == TOK_LSHIFTEQ) base = TOK_LSHIFT;
            if (op == TOK_RSHIFTEQ) base = TOK_RSHIFT;
            char *res = NULL;
            if (op == TOK_ASSIGN) {
                res = cast_value(cg, rv, rt, vtype);
            } else {
                /* compute cur base rv */
                char *ct = vtype;
                char *rvc = cast_value(cg, rv, rt, ct);
                char *tmp = new_tmp(cg);
                if (base == TOK_PLUS) {
                    if (is_float_type(ct)) fprintf(cg->out, "  %s = fadd %s %s, %s\n", tmp, ct, cur, rvc);
                    else fprintf(cg->out, "  %s = add %s %s, %s\n", tmp, ct, cur, rvc);
                } else if (base == TOK_MINUS) {
                    if (is_float_type(ct)) fprintf(cg->out, "  %s = fsub %s %s, %s\n", tmp, ct, cur, rvc);
                    else fprintf(cg->out, "  %s = sub %s %s, %s\n", tmp, ct, cur, rvc);
                } else if (base == TOK_STAR) {
                    if (is_float_type(ct)) fprintf(cg->out, "  %s = fmul %s %s, %s\n", tmp, ct, cur, rvc);
                    else fprintf(cg->out, "  %s = mul %s %s, %s\n", tmp, ct, cur, rvc);
                } else if (base == TOK_SLASH) {
                    if (is_float_type(ct)) fprintf(cg->out, "  %s = fdiv %s %s, %s\n", tmp, ct, cur, rvc);
                    else fprintf(cg->out, "  %s = sdiv %s %s, %s\n", tmp, ct, cur, rvc);
                } else {
                    if (is_float_type(ct)) fprintf(cg->out, "  %s = fadd %s %s, %s\n", tmp, ct, cur, rvc);
                    else fprintf(cg->out, "  %s = add %s %s, %s\n", tmp, ct, cur, rvc);
                }
                res = tmp;
            }
            if (v) fprintf(cg->out, "  store %s %s, ptr %s\n", vtype, res, v->ir);
            if (out_type) *out_type = strdup(vtype);
            return res ? res : strdup("0");
        }
        /* normal binary */
        char *lt = NULL, *rt = NULL;
        char *lv = codegen_expr(cg, e->left, &lt);
        char *rv = codegen_expr(cg, e->right, &rt);
        const char *res_type = (is_float_type(lt) || is_float_type(rt)) ? (strcmp(lt,"double")==0||strcmp(rt,"double")==0 ? "double" : "float") : "i32";
        /* exception comparisons produce i1 */
        int is_cmp = (op==TOK_EQEQ||op==TOK_BANGEQ||op==TOK_LT||op==TOK_LTEQ||op==TOK_GT||op==TOK_GTEQ||op==TOK_AMPAMP||op==TOK_PIPEPIPE);
        if (is_cmp && !(op==TOK_AMPAMP||op==TOK_PIPEPIPE)) {
            /* result i1 */
            cast_value(cg, lv, lt, is_float_type(lt)&&is_float_type(res_type)?lt:"i32");
            /* ensure both int/float consistent */
            char *ll = lv, *rr = rv;
            const char *ltt = lt, *rrt = rt;
            /* if any float, cast both to float */
            if (is_float_type(lt) || is_float_type(rt)) {
                const char *ft = (is_float_type(lt)&&strcmp(lt,"double")==0) || (is_float_type(rt)&&strcmp(rt,"double")==0) ? "double" : "float";
                ll = cast_value(cg, lv, lt, ft);
                rr = cast_value(cg, rv, rt, ft);
                ltt = rrt = ft;
            }
            char *r = new_tmp(cg);
            if (is_float_type(ltt)) {
                if (op==TOK_EQEQ) fprintf(cg->out, "  %s = fcmp oeq %s %s, %s\n", r, ltt, ll, rr);
                else if (op==TOK_BANGEQ) fprintf(cg->out, "  %s = fcmp one %s %s, %s\n", r, ltt, ll, rr);
                else if (op==TOK_LT) fprintf(cg->out, "  %s = fcmp olt %s %s, %s\n", r, ltt, ll, rr);
                else if (op==TOK_LTEQ) fprintf(cg->out, "  %s = fcmp ole %s %s, %s\n", r, ltt, ll, rr);
                else if (op==TOK_GT) fprintf(cg->out, "  %s = fcmp ogt %s %s, %s\n", r, ltt, ll, rr);
                else fprintf(cg->out, "  %s = fcmp oge %s %s, %s\n", r, ltt, ll, rr);
            } else {
                const char *icmp_op = "eq";
                if (op==TOK_EQEQ) icmp_op="eq";
                else if (op==TOK_BANGEQ) icmp_op="ne";
                else if (op==TOK_LT) icmp_op="slt";
                else if (op==TOK_LTEQ) icmp_op="sle";
                else if (op==TOK_GT) icmp_op="sgt";
                else icmp_op="sge";
                fprintf(cg->out, "  %s = icmp %s %s %s, %s\n", r, icmp_op, ltt, ll, rr);
            }
            if (out_type) *out_type = strdup("i1");
            return r;
        }
        if (op == TOK_AMPAMP || op == TOK_PIPEPIPE) {
            cast_value(cg, lv, lt, "i1"); /* not proper; cmp to zero */
            /* instead create i1 by icmp */
            char *li = new_tmp(cg);
            fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", li, lt, lv);
            char *ri = new_tmp(cg);
            fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", ri, rt, rv);
            char *r = new_tmp(cg);
            if (op == TOK_AMPAMP) fprintf(cg->out, "  %s = and i1 %s, %s\n", r, li, ri);
            else fprintf(cg->out, "  %s = or i1 %s, %s\n", r, li, ri);
            if (out_type) *out_type = strdup("i1");
            return r;
        }
        /* pow */
        if (op == TOK_STARSTAR) {
            /* cast both to double, call pow, cast back if needed */
            char *ld = cast_value(cg, lv, lt, "double");
            char *rd = cast_value(cg, rv, rt, "double");
            /* ensure declare */
            CGFunc *pf = find_func(cg, "pow");
            if (!pf) {
                char *dtypes[2]; dtypes[0]=strdup("double"); dtypes[1]=strdup("double");
                char **arr = xmalloc(2*sizeof(char*));
                arr[0]=dtypes[0]; arr[1]=dtypes[1];
                add_func(cg, "pow", "double", arr, 2, 0);
                fprintf(cg->out, "declare double @pow(double, double)\n");
            }
            char *r = new_tmp(cg);
            fprintf(cg->out, "  %s = call double @pow(double %s, double %s)\n", r, ld, rd);
            char *res = cast_value(cg, r, "double", res_type);
            if (out_type) *out_type = strdup(res_type);
            return res;
        }
        /* arithmetic */
        char *ll = cast_value(cg, lv, lt, res_type);
        char *rr = cast_value(cg, rv, rt, res_type);
        char *r = new_tmp(cg);
        if (is_float_type(res_type)) {
            if (op == TOK_PLUS) fprintf(cg->out, "  %s = fadd %s %s, %s\n", r, res_type, ll, rr);
            else if (op == TOK_MINUS) fprintf(cg->out, "  %s = fsub %s %s, %s\n", r, res_type, ll, rr);
            else if (op == TOK_STAR) fprintf(cg->out, "  %s = fmul %s %s, %s\n", r, res_type, ll, rr);
            else if (op == TOK_SLASH) fprintf(cg->out, "  %s = fdiv %s %s, %s\n", r, res_type, ll, rr);
            else fprintf(cg->out, "  %s = fadd %s %s, %s\n", r, res_type, ll, rr);
        } else {
            if (op == TOK_PLUS) fprintf(cg->out, "  %s = add i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_MINUS) fprintf(cg->out, "  %s = sub i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_STAR) fprintf(cg->out, "  %s = mul i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_SLASH) fprintf(cg->out, "  %s = sdiv i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_PERCENT) fprintf(cg->out, "  %s = srem i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_AMP) fprintf(cg->out, "  %s = and i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_PIPE) fprintf(cg->out, "  %s = or i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_CARET) fprintf(cg->out, "  %s = xor i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_LSHIFT) fprintf(cg->out, "  %s = shl i32 %s, %s\n", r, ll, rr);
            else if (op == TOK_RSHIFT) fprintf(cg->out, "  %s = ashr i32 %s, %s\n", r, ll, rr);
            else fprintf(cg->out, "  %s = add i32 %s, %s\n", r, ll, rr);
        }
        if (out_type) *out_type = strdup(res_type);
        return r;
    }
    if (e->type == AST_EXPR_TERNARY) {
        char *ct; char *cv = codegen_expr(cg, e->cond, &ct);
        /* convert cond to i1 */
        char *ci = new_tmp(cg);
        if (is_float_type(ct)) fprintf(cg->out, "  %s = fcmp one %s %s, 0.0\n", ci, ct, cv);
        else fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", ci, ct, cv);
        char *tt; char *tv = codegen_expr(cg, e->then_branch, &tt);
        char *et; char *ev = codegen_expr(cg, e->else_branch, &et);
        const char *rt = (is_float_type(tt)||is_float_type(et)) ? "float" : "i32";
        tv = cast_value(cg, tv, tt, rt);
        ev = cast_value(cg, ev, et, rt);
        char *r = new_tmp(cg);
        fprintf(cg->out, "  %s = select i1 %s, %s %s, %s %s\n", r, ci, rt, tv, rt, ev);
        if (out_type) *out_type = strdup(rt);
        return r;
    }
    if (e->type == AST_EXPR_CALL) {
        /* indirect call: callee is a variable holding a function pointer */
        if (e->left && e->left->type == AST_EXPR_IDENT) {
            CGVar *fv = find_var(cg, e->left->name);
            Type *fty = fv ? cg_resolve_type(fv->asty) : NULL;
            if (fv && fty && fty->kind == TY_FUNCTION) {
                size_t argc = e->arg_count;
                char **rawvals = xmalloc((argc?argc:1)*sizeof(char*));
                char **rawtypes = xmalloc((argc?argc:1)*sizeof(char*));
                for (size_t i = 0; i < argc; i++) {
                    char *t = NULL;
                    rawvals[i] = codegen_expr(cg, e->args[i], &t);
                    rawtypes[i] = t ? t : strdup("i32");
                }
                char *ret = fty->return_type ? llvm_type(fty->return_type) : strdup("i32");
                char **argvals = xmalloc((argc?argc:1)*sizeof(char*));
                char **argtypes = xmalloc((argc?argc:1)*sizeof(char*));
                for (size_t i = 0; i < argc; i++) {
                    const char *pt = (i < fty->param_count && fty->param_types[i]) ? llvm_type(fty->param_types[i]) : strdup("i32");
                    argvals[i] = cast_value(cg, rawvals[i], rawtypes[i], pt);
                    argtypes[i] = strdup(pt);
                }
                char *fp = new_tmp(cg);
                fprintf(cg->out, "  %s = load ptr, ptr %s\n", fp, fv->ir);
                int is_void = (strcmp(ret, "void") == 0);
                char *r = is_void ? strdup("0") : new_tmp(cg);
                fprintf(cg->out, "  ");
                if (!is_void) fprintf(cg->out, "%s = ", r);
                fprintf(cg->out, "call %s %s(", ret, fp);
                for (size_t i = 0; i < argc; i++) {
                    if (i) fprintf(cg->out, ", ");
                    fprintf(cg->out, "%s %s", argtypes[i], argvals[i]);
                }
                fprintf(cg->out, ")\n");
                if (out_type) *out_type = strdup(ret);
                return is_void ? strdup("0") : r;
            }
        }
        /* callee name */
        char *callee = NULL;
        if (e->left && e->left->type == AST_EXPR_IDENT) {
            callee = e->left->name;
        } else if (e->left && e->left->type == AST_EXPR_FIELD) {
            /* Package-qualified call, e.g. `std.print_int(...)`. The standard
             * package is a thin surface over the C runtime, whose symbols keep
             * their `loi_` prefix internally. A member is only rewritten when
             * the package is `std` and a matching runtime symbol exists. */
            ASTNode *base = e->left->left;
            if (base && base->type == AST_EXPR_IDENT && base->name &&
                strcmp(base->name, "std") == 0 && e->left->name) {
                char rt[128];
                snprintf(rt, sizeof(rt), "loi_%s", e->left->name);
                if (find_func(cg, rt)) callee = strdup(rt);
                else callee = e->left->name;
            } else {
                callee = e->left->name;
            }
        } else callee = "fn";
        /* evaluate args first */
        size_t argc = e->arg_count;
        char **rawvals = xmalloc((argc?argc:1)*sizeof(char*));
        char **rawtypes = xmalloc((argc?argc:1)*sizeof(char*));
        for (size_t i=0;i<argc;i++) {
            char *t = NULL;
            char *v = codegen_expr(cg, e->args[i], &t);
            rawvals[i] = v;
            rawtypes[i] = t ? t : strdup("i32");
        }
        /* find or declare function */
        CGFunc *f = find_func(cg, callee);
        if (!f) {
            char **ptypes = xmalloc((argc?argc:1)*sizeof(char*));
            for (size_t i=0;i<argc;i++) ptypes[i] = strdup(rawtypes[i]);
            add_func(cg, callee, "i32", ptypes, argc, 0);
            f = find_func(cg, callee);
        }
        char **argvals = xmalloc((argc?argc:1)*sizeof(char*));
        char **argtypes = xmalloc((argc?argc:1)*sizeof(char*));
        for (size_t i=0;i<argc;i++) {
            const char *ptype = (i < f->param_count) ? f->param_types[i] : "i32";
            char *cv = cast_value(cg, rawvals[i], rawtypes[i], ptype);
            argvals[i] = cv;
            argtypes[i] = strdup(ptype);
        }
        char *r = NULL;
        if (strcmp(f->retty, "void") == 0) {
            r = strdup("0");
        } else {
            r = new_tmp(cg);
        }
        fprintf(cg->out, "  ");
        if (strcmp(f->retty, "void") != 0) fprintf(cg->out, "%s = ", r);
        fprintf(cg->out, "call %s @%s(", f->retty, f->link_name ? f->link_name : callee);
        for (size_t i=0;i<argc;i++) {
            if (i) fprintf(cg->out, ", ");
            fprintf(cg->out, "%s %s", argtypes[i], argvals[i]);
        }
        fprintf(cg->out, ")\n");
        if (out_type) *out_type = strdup(f->retty);
        return (strcmp(f->retty, "void")==0) ? strdup("0") : r;
    }
    if (e->type == AST_EXPR_INDEX) {
        if (e->left && e->left->type == AST_EXPR_IDENT) {
            CGVar *v = find_var(cg, e->left->name);
            if (v && v->asty && (v->asty->kind == TY_ARRAY || v->asty->kind == TY_POINTER)) {
                char *it = NULL; char *iv = codegen_expr(cg, e->right, &it);
                char *idx = cast_value(cg, iv, it ? it : "i32", "i32");
                char *base_type = v->asty->kind == TY_ARRAY ? llvm_type(v->asty->base) : strdup("i32");
                char *gep = new_tmp(cg);
                fprintf(cg->out, "  %s = getelementptr inbounds %s, ptr %s, i32 0, i32 %s\n", gep, v->type, v->ir, idx);
                char *ld = new_tmp(cg);
                fprintf(cg->out, "  %s = load %s, ptr %s\n", ld, base_type, gep);
                if (out_type) *out_type = strdup(base_type);
                return ld;
            }
        }
        /* fallback load as ptr */
        char *bt; codegen_expr(cg, e->left, &bt);
        if (out_type) *out_type = strdup("i32");
        return strdup("0");
    }
    if (e->type == AST_EXPR_FIELD && e->left && e->left->type == AST_EXPR_IDENT) {
        CGVar *v = find_var(cg, e->left->name);
        if (v && v->asty && v->asty->name) {
            StructInfo *si = find_struct(v->asty->name);
            if (si) {
                int idx = -1;
                for (int i = 0; i < si->field_count; i++) {
                    if (e->name && strcmp(si->field_names[i], e->name) == 0) { idx = i; break; }
                }
                if (idx >= 0) {
                    char *field_type = si->field_types[idx];
                    char *gep = new_tmp(cg);
                    fprintf(cg->out, "  %s = getelementptr inbounds %s, ptr %s, i32 0, i32 %d\n", gep, v->type, v->ir, idx);
                    char *ld = new_tmp(cg);
                    fprintf(cg->out, "  %s = load %s, ptr %s\n", ld, field_type, gep);
                    if (out_type) *out_type = strdup(field_type);
                    return ld;
                }
            }
        }
    }
    /* fallback */
    if (out_type) *out_type = strdup("i32");
    return strdup("0");
}

/* ---------- stmt codegen ---------- */
static void codegen_block(CG *cg, ASTNode *b) {
    if (!b) return;
    for (size_t i = 0; i < b->item_count; i++) codegen_stmt(cg, b->items[i]);
}

static void codegen_stmt(CG *cg, ASTNode *s) {
    if (!s || cg->terminated) return;
    switch (s->type) {
        case AST_BLOCK: codegen_block(cg, s); break;
        case AST_REGION: {
            char *ct = NULL;
            char *cv;
            int cap_provided = (s->cond != NULL);
            if (cap_provided) cv = codegen_expr(cg, s->cond, &ct);
            else cv = strdup("4096");
            char *cv_cast = cast_value(cg, cv, ct ? ct : "i32", "i64");
            char *region_ptr = new_tmp(cg);
            fprintf(cg->out, "  %s = call ptr @loi_region_new(i64 %s)\n", region_ptr, cv_cast);
            char *alloc = new_tmp(cg);
            fprintf(cg->out, "  %s = alloca ptr\n", alloc);
            if (s->name) add_var(cg, s->name, alloc, strdup("ptr"), NULL);
            fprintf(cg->out, "  store ptr %s, ptr %s\n", region_ptr, alloc);
            scope_push(cg);
            /* `region ... with Write as w` binds `w` to an erased i8 capability */
            if (s->caps && s->str_value) {
                char *ca = new_tmp(cg);
                fprintf(cg->out, "  %s = alloca i8\n", ca);
                fprintf(cg->out, "  store i8 0, ptr %s\n", ca);
                add_var(cg, s->str_value, ca, strdup("i8"), NULL);
            }
            codegen_block(cg, s->body);
            scope_pop(cg);
            if (!cg->terminated) {
                fprintf(cg->out, "  call void @loi_region_free(ptr %s)\n", region_ptr);
            }
            break;
        }
        case AST_VAR_DECL: {
            const char *vtype = s->ty ? llvm_type(s->ty) : strdup("i32");
            char *alloc = new_tmp(cg);
            int al = 0;
            if (s->ty && s->ty->name) {
                StructInfo *si = find_struct(s->ty->name);
                if (si) al = si->align;
            }
            if (al > 0) {
                fprintf(cg->out, "  %s = alloca %s, align %d\n", alloc, vtype, al);
            } else {
                fprintf(cg->out, "  %s = alloca %s\n", alloc, vtype);
            }
            /* register before init */
            if (s->name) add_var(cg, s->name, alloc, vtype, s->ty);
            if (s->init) {
                char *it = NULL;
                char *iv = codegen_expr(cg, s->init, &it);
                char *cv = cast_value(cg, iv, it ? it : "i32", vtype);
                fprintf(cg->out, "  store %s %s, ptr %s\n", vtype, cv, alloc);
            } else {
                if (vtype && (vtype[0]=='[' || vtype[0]=='%')) {
                    fprintf(cg->out, "  store %s zeroinitializer, ptr %s\n", vtype, alloc);
                } else if (strcmp(vtype,"ptr")==0) {
                    fprintf(cg->out, "  store ptr null, ptr %s\n", alloc);
                } else {
                    fprintf(cg->out, "  store %s 0, ptr %s\n", vtype, alloc);
                }
            }
            break;
        }
        case AST_GRANT: {
            /* Introduce a capability binding as a zeroed i8 (erased later). */
            char *alloc = new_tmp(cg);
            fprintf(cg->out, "  %s = alloca i8\n", alloc);
            fprintf(cg->out, "  store i8 0, ptr %s\n", alloc);
            if (s->name) add_var(cg, s->name, alloc, strdup("i8"), s->ty);
            break;
        }
        case AST_REFINE:
        case AST_DROP:
        case AST_CAP_DECL:
            /* purely lexical: nothing is emitted */
            break;
        case AST_IF: {
            char *ct; char *cv = codegen_expr(cg, s->cond, &ct);
            char *ci = new_tmp(cg);
            if (is_float_type(ct)) fprintf(cg->out, "  %s = fcmp one %s %s, 0.0\n", ci, ct, cv);
            else fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", ci, ct, cv);
            char *thenL = new_label(cg, "then");
            char *elseL = new_label(cg, "else");
            char *mergeL = new_label(cg, "merge");
            fprintf(cg->out, "  br i1 %s, label %%%s, label %%%s\n", ci, thenL, elseL);
            fprintf(cg->out, "%s:\n", thenL);
            cg->terminated = 0;
            scope_push(cg);
            codegen_block(cg, s->then_branch);
            scope_pop(cg);
            if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", mergeL);
            cg->terminated = 1;
            fprintf(cg->out, "%s:\n", elseL);
            cg->terminated = 0;
            scope_push(cg);
            if (s->else_branch) codegen_block(cg, s->else_branch);
            scope_pop(cg);
            if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", mergeL);
            cg->terminated = 1;
            fprintf(cg->out, "%s:\n", mergeL);
            cg->terminated = 0;
            break;
        }
        case AST_WHILE: {
            char *hdr = new_label(cg, "while_hdr");
            char *body = new_label(cg, "while_body");
            char *exit = new_label(cg, "while_exit");
            /* push loop labels */
            if (cg->loop_depth == cg->loop_cap) {
                size_t nc = cg->loop_cap ? cg->loop_cap * 2 : 8;
                cg->loop_exit_labels = realloc(cg->loop_exit_labels, nc * sizeof(char*));
                cg->loop_cont_labels = realloc(cg->loop_cont_labels, nc * sizeof(char*));
                cg->loop_update_labels = realloc(cg->loop_update_labels, nc * sizeof(char*));
                cg->loop_cap = nc;
            }
            cg->loop_exit_labels[cg->loop_depth] = exit;
            cg->loop_cont_labels[cg->loop_depth] = hdr;
            cg->loop_update_labels[cg->loop_depth] = NULL;
            cg->loop_depth++;

            fprintf(cg->out, "  br label %%%s\n", hdr);
            fprintf(cg->out, "%s:\n", hdr);
            cg->terminated = 0;
            char *ct; char *cv = codegen_expr(cg, s->cond, &ct);
            char *ci = new_tmp(cg);
            if (is_float_type(ct)) fprintf(cg->out, "  %s = fcmp one %s %s, 0.0\n", ci, ct, cv);
            else fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", ci, ct, cv);
            fprintf(cg->out, "  br i1 %s, label %%%s, label %%%s\n", ci, body, exit);
            fprintf(cg->out, "%s:\n", body);
            cg->terminated = 0;
            scope_push(cg);
            codegen_block(cg, s->body);
            scope_pop(cg);
            if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", hdr);
            cg->terminated = 1;
            fprintf(cg->out, "%s:\n", exit);
            cg->terminated = 0;

            cg->loop_depth--;
            break;
        }
        case AST_DO_WHILE: {

            char *body = new_label(cg, "dowhile_body");

            char *hdr = new_label(cg, "dowhile_hdr");

            char *exit = new_label(cg, "dowhile_exit");

            /* push loop labels */

            if (cg->loop_depth == cg->loop_cap) {

                size_t nc = cg->loop_cap ? cg->loop_cap * 2 : 8;

                cg->loop_exit_labels = realloc(cg->loop_exit_labels, nc * sizeof(char*));

                cg->loop_cont_labels = realloc(cg->loop_cont_labels, nc * sizeof(char*));

                cg->loop_update_labels = realloc(cg->loop_update_labels, nc * sizeof(char*));

                cg->loop_cap = nc;

            }

            cg->loop_exit_labels[cg->loop_depth] = exit;

            cg->loop_cont_labels[cg->loop_depth] = hdr;

            cg->loop_update_labels[cg->loop_depth] = NULL;

            cg->loop_depth++;



            /* do-while executes body first, then checks condition */

            fprintf(cg->out, "  br label %%%s\n", body);

            fprintf(cg->out, "%s:\n", body);

            cg->terminated = 0;

            scope_push(cg);

            codegen_block(cg, s->body);

            scope_pop(cg);

            /* if body didn't end with a terminator, branch to header */
            if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", hdr);

            cg->terminated = 0;
            fprintf(cg->out, "%s:\n", hdr);

            char *ct; char *cv = codegen_expr(cg, s->cond, &ct);

            char *ci = new_tmp(cg);

            if (is_float_type(ct)) fprintf(cg->out, "  %s = fcmp one %s %s, 0.0\n", ci, ct, cv);

            else fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", ci, ct, cv);

            fprintf(cg->out, "  br i1 %s, label %%%s, label %%%s\n", ci, body, exit);

            fprintf(cg->out, "%s:\n", exit);

            cg->terminated = 0;



            cg->loop_depth--;

            break;
        }
        case AST_SWITCH: {
            char *cond_type = NULL;
            char *cond_val = codegen_expr(cg, s->cond, &cond_type);
            /* collect cases and default */
            int case_count = 0;
            ASTNode *default_case = NULL;
            ASTNode *cases[32];
            for (size_t i = 0; i < s->item_count; i++) {
                ASTNode *item = s->items[i];
                if (item->type == AST_CASE) {
                    if (case_count < 32) cases[case_count++] = item;
                } else if (item->type == AST_DEFAULT) {
                    default_case = item;
                }
            }
            if (case_count == 0 && !default_case) {
                /* empty switch - just return 0 */
                if (cg->current_retty && strcmp(cg->current_retty, "void") != 0) {
                    fprintf(cg->out, "  ret %s 0\n", cg->current_retty);
                } else {
                    fprintf(cg->out, "  ret void\n");
                }
                cg->terminated = 1;
                break;
            }

            /* create unique labels for each case comparison and body */
            char *exit_label = new_label(cg, "switch_exit");
            char *case_cmp_labels[32];
            char *case_body_labels[32];
            for (int i = 0; i < case_count; i++) {
                case_cmp_labels[i] = new_label(cg, "switch_cmp");
                case_body_labels[i] = new_label(cg, "switch_body");
            }
            char *default_cmp_label = default_case ? new_label(cg, "switch_default_cmp") : NULL;
            char *default_body_label = default_case ? new_label(cg, "switch_default_body") : NULL;

            /* evaluate condition once */
            char *cond_var = new_tmp(cg);
            fprintf(cg->out, "  %s = alloca %s\n", cond_var, cond_type);
            fprintf(cg->out, "  store %s %s, ptr %s\n", cond_type, cond_val, cond_var);
            char *cond_val_load = new_tmp(cg);
            fprintf(cg->out, "  %s = load %s, ptr %s\n", cond_val_load, cond_type, cond_var);

            /* generate comparison chain */
            fprintf(cg->out, "  br label %%%s\n", case_cmp_labels[0]);

            for (int i = 0; i < case_count; i++) {
                ASTNode *c = cases[i];
                fprintf(cg->out, "%s:\n", case_cmp_labels[i]);
                char *case_val = codegen_expr(cg, c->cond, NULL);
                char *cmp = new_tmp(cg);
                if (is_float_type(cond_type)) {
                    fprintf(cg->out, "  %s = fcmp oeq %s %s, %s\n", cmp, cond_type, cond_val, case_val);
                } else {
                    fprintf(cg->out, "  %s = icmp eq %s %s, %s\n", cmp, cond_type, cond_val, case_val);
                }
                char *next_label = (i + 1 < case_count) ? case_cmp_labels[i + 1] : (default_case ? default_cmp_label : exit_label);
                fprintf(cg->out, "  br i1 %s, label %%%s, label %%%s\n", cmp, case_body_labels[i], next_label);
            }

            if (default_case) {
                fprintf(cg->out, "%s:\n", default_cmp_label);
                fprintf(cg->out, "  br label %%%s\n", default_body_label);
            } else {
                fprintf(cg->out, "  br label %%%s\n", exit_label);
            }

            /* case bodies */
            for (int i = 0; i < case_count; i++) {
                fprintf(cg->out, "%s:\n", case_body_labels[i]);
                cg->terminated = 0;
                scope_push(cg);
                codegen_block(cg, cases[i]->body);
                scope_pop(cg);
                if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", exit_label);
                cg->terminated = 1;
            }
            if (default_case) {
                fprintf(cg->out, "%s:\n", default_body_label);
                cg->terminated = 0;
                scope_push(cg);
                codegen_block(cg, default_case->body);
                scope_pop(cg);
                if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", exit_label);
                cg->terminated = 1;
            }

            fprintf(cg->out, "%s:\n", exit_label);
            cg->terminated = 0;
            break;
        }        case AST_FOR: {
            /* init */
            if (s->init_stmt) codegen_stmt(cg, s->init_stmt);
            char *hdr = new_label(cg, "for.hdr");
            char *body = new_label(cg, "for.body");
            char *update = new_label(cg, "for_update_label");
            char *exit = new_label(cg, "for.exit");
            /* push loop labels */
            if (cg->loop_depth == cg->loop_cap) {
                size_t nc = cg->loop_cap ? cg->loop_cap * 2 : 8;
                cg->loop_exit_labels = realloc(cg->loop_exit_labels, nc * sizeof(char*));
                cg->loop_cont_labels = realloc(cg->loop_cont_labels, nc * sizeof(char*));
                cg->loop_update_labels = realloc(cg->loop_update_labels, nc * sizeof(char*));
                cg->loop_cap = nc;
            }
            cg->loop_exit_labels[cg->loop_depth] = exit;
            cg->loop_cont_labels[cg->loop_depth] = hdr;
            cg->loop_update_labels[cg->loop_depth] = update;
            cg->loop_depth++;

            fprintf(cg->out, "  br label %%%s\n", hdr);
            fprintf(cg->out, "%s:\n", hdr);
            cg->terminated = 0;
            if (s->cond_expr) {
                char *ct; char *cv = codegen_expr(cg, s->cond_expr, &ct);
                char *ci = new_tmp(cg);
                if (is_float_type(ct)) fprintf(cg->out, "  %s = fcmp one %s %s, 0.0\n", ci, ct, cv);
                else fprintf(cg->out, "  %s = icmp ne %s %s, 0\n", ci, ct, cv);
                fprintf(cg->out, "  br i1 %s, label %%%s, label %%%s\n", ci, body, exit);
            } else {
                fprintf(cg->out, "  br label %%%s\n", body);
            }
            fprintf(cg->out, "%s:\n", body);
            cg->terminated = 0;
            scope_push(cg);
            codegen_block(cg, s->body);
            codegen_block(cg, s->body);
            scope_pop(cg);
            /* if body doesn't end with a terminator, branch to update */
            if (!cg->terminated) {
                fprintf(cg->out, "  br label %%%s\n", update);
                cg->terminated = 1;
            }
            /* update */
            fprintf(cg->out, "%s:\n", update);
            cg->terminated = 0;
            if (s->update && !cg->terminated) codegen_stmt(cg, s->update);
            if (!cg->terminated) fprintf(cg->out, "  br label %%%s\n", hdr);
            cg->terminated = 1;
            fprintf(cg->out, "%s:\n", exit);
            cg->terminated = 0;

            cg->loop_depth--;
            break;
        }
        case AST_RETURN: {
            const char *ftarget = cg->current_retty ? cg->current_retty : "i32";
            if (s->left) {
                char *rt = NULL;
                char *rv = codegen_expr(cg, s->left, &rt);
                if (strcmp(ftarget, "void") == 0) {
                    fprintf(cg->out, "  ret void\n");
                } else {
                    char *cv = cast_value(cg, rv, rt ? rt : "i32", ftarget);
                    fprintf(cg->out, "  ret %s %s\n", ftarget, cv);
                }
            } else {
                if (strcmp(ftarget, "void") == 0) fprintf(cg->out, "  ret void\n");
                else fprintf(cg->out, "  ret %s 0\n", ftarget);
            }
            cg->terminated = 1;
            break;
        }
        case AST_EXPR_STMT: {
            if (s->left) {
                char *t = NULL;
                char *v = codegen_expr(cg, s->left, &t);
                (void)v;
            }
            break;
        }
        case AST_ASM: {
            /* ignore */
            break;
        }
        case AST_BREAK: {
            if (cg->loop_depth == 0) {
                fprintf(stderr, "error: break outside loop\n");
                exit(1);
            }
            fprintf(cg->out, "  br label %%%s\n", cg->loop_exit_labels[cg->loop_depth - 1]);
            cg->terminated = 1;
            break;
        }
        case AST_CONTINUE: {
            if (cg->loop_depth == 0) {
                fprintf(stderr, "error: continue outside loop\n");
                exit(1);
            }
            /* for for-loops, continue goes to update section; for while/do-while, to header */
            char *target = cg->loop_update_labels[cg->loop_depth - 1];
            if (!target) target = cg->loop_cont_labels[cg->loop_depth - 1];
            fprintf(cg->out, "  br label %%%s\n", target);
            cg->terminated = 1;
            break;
        }
        default:
            break;
    }
}

/* We need cg available. Implement collect funcs with cg param. */
static void apply_attrs_to_func(CGFunc *cf, ASTNode *d) {
    if (!cf || !d) return;
    if (d->link_name && d->link_name[0]) {
        free(cf->link_name);
        cf->link_name = strdup(d->link_name);
    }
    if (ast_has_attr(d, "inline"))   cf->attrs |= CGATTR_INLINE;
    if (ast_has_attr(d, "no_return")) cf->attrs |= CGATTR_NORETURN;
    if (ast_has_attr(d, "pure"))     cf->attrs |= CGATTR_PURE;
    if (ast_has_attr(d, "must_use")) cf->attrs |= CGATTR_MUST_USE;
}

static void collect_funcs_cg(CG *cg, ASTNode *prog) {
    if (!prog) return;
    for (size_t i = 0; i < prog->item_count; i++) {
        ASTNode *d = prog->items[i];
        if (d && d->type == AST_FUNC_DECL) {
            char *retty = d->ty ? llvm_type(d->ty) : strdup("i32");
            size_t argc = d->arg_count;
            char **ptypes = argc ? xmalloc(argc * sizeof(char*)) : NULL;
            for (size_t j = 0; j < argc; j++) {
                ASTNode *p = d->args[j];
                ptypes[j] = p && p->ty ? llvm_type(p->ty) : strdup("i32");
            }
            add_func(cg, d->name ? d->name : "f", retty, ptypes, argc, 1);
            CGFunc *cf = find_func(cg, d->name ? d->name : "f");
            if (cf) apply_attrs_to_func(cf, d);
        } else if (d && d->type == AST_EXTERN_DECL && d->name) {
            char *retty = d->ty ? llvm_type(d->ty) : strdup("i32");
            size_t argc = d->arg_count;
            char **ptypes = argc ? xmalloc(argc * sizeof(char*)) : NULL;
            for (size_t j = 0; j < argc; j++) {
                ASTNode *p = d->args[j];
                ptypes[j] = p && p->ty ? llvm_type(p->ty) : strdup("i32");
            }
            add_func(cg, d->name, retty, ptypes, argc, 0);
            CGFunc *cf = find_func(cg, d->name);
            if (cf) apply_attrs_to_func(cf, d);
        }
    }
}

/* ---------- function codegen ---------- */
static void codegen_function(CG *cg, ASTNode *f) {
    const char *name = f->name ? f->name : "f";
    char *retty = f->ty ? llvm_type(f->ty) : strdup("i32");
    CGFunc *cf = find_func(cg, name);
    const char *sym = (cf && cf->link_name) ? cf->link_name : name;
    unsigned at = cf ? cf->attrs : 0;
    fprintf(cg->out, "define %s @%s(", retty, sym);
    for (size_t i = 0; i < f->arg_count; i++) {
        ASTNode *p = f->args[i];
        char *pt = p && p->ty ? llvm_type(p->ty) : strdup("i32");
        if (i) fprintf(cg->out, ", ");
        fprintf(cg->out, "%s %%arg%zu", pt, i);
    }
    fprintf(cg->out, ")");
    if (at & CGATTR_NORETURN) fprintf(cg->out, " noreturn");
    if (at & CGATTR_INLINE) fprintf(cg->out, " alwaysinline");
    if (at & CGATTR_PURE) fprintf(cg->out, " readnone");
    if (f->section && f->section[0]) fprintf(cg->out, " section \"%s\"", f->section);
    fprintf(cg->out, " {\nentry:\n");
    cg->terminated = 0;
    scope_push(cg);
    /* alloc params */
    for (size_t i = 0; i < f->arg_count; i++) {
        ASTNode *p = f->args[i];
        char *pt = p && p->ty ? llvm_type(p->ty) : strdup("i32");
        char *alloc = new_tmp(cg);
        fprintf(cg->out, "  %s = alloca %s\n", alloc, pt);
        fprintf(cg->out, "  store %s %%arg%zu, ptr %s\n", pt, i, alloc);
        if (p->name) add_var(cg, p->name, alloc, pt, p->ty);
    }
    cg->current_retty = retty;
    if (f->body) {
        if (f->body->type == AST_BLOCK) codegen_block(cg, f->body);
        else codegen_stmt(cg, f->body);
    }
    if (!cg->terminated) {
        if (strcmp(retty, "void") == 0) fprintf(cg->out, "  ret void\n");
        else fprintf(cg->out, "  ret %s 0\n", retty);
        cg->terminated = 1;
    }
    scope_pop(cg);
    fprintf(cg->out, "}\n\n");
}

static void add_runtime_funcs(CG *cg) {
    /* i64 helpers */
    char *p_i64 = strdup("i64");
    /* print_int(void(i64)) */
    /* runtime symbols are always linked; std.* maps to these. */
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("i64");
        add_func(cg,"loi_print_int","void",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_print_str","void",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("i64");
        add_func(cg,"loi_abs","i64",pt,1,0);
    }
    {
        char **pt = xmalloc(2*sizeof(char*)); pt[0]=strdup("i64"); pt[1]=strdup("i64");
        add_func(cg,"loi_min","i64",pt,2,0);
        add_func(cg,"loi_max","i64",pt,2,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("double");
        add_func(cg,"loi_sqrt","double",pt,1,0);
        add_func(cg,"loi_sin","double",pt,1,0);
        add_func(cg,"loi_cos","double",pt,1,0);
        add_func(cg,"loi_tan","double",pt,1,0);
        add_func(cg,"loi_exp","double",pt,1,0);
        add_func(cg,"loi_log","double",pt,1,0);
        add_func(cg,"loi_floor","double",pt,1,0);
        add_func(cg,"loi_ceil","double",pt,1,0);
    }
    {
        char **pt = xmalloc(2*sizeof(char*)); pt[0]=strdup("double"); pt[1]=strdup("double");
        add_func(cg,"loi_pow","double",pt,2,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_strlen","i64",pt,1,0);
    }
    {
        /* memcpy(ptr,ptr,i64) */
        char **pt = xmalloc(3*sizeof(char*)); pt[0]=strdup("ptr"); pt[1]=strdup("ptr"); pt[2]=strdup("i64");
        add_func(cg,"loi_memcpy","ptr",pt,3,0);
        add_func(cg,"loi_memset","ptr",pt,3,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("i64");
        add_func(cg,"loi_alloc","ptr",pt,1,0);
        add_func(cg,"loi_gc_alloc","ptr",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_free","void",pt,1,0);
    }
    {
        add_func(cg,"loi_gc_collect","void",NULL,0,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_gc_add_root","void",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("i64");
        add_func(cg,"loi_region_new","ptr",pt,1,0);
    }
    {
        char **pt = xmalloc(2*sizeof(char*)); pt[0]=strdup("ptr"); pt[1]=strdup("i64");
        add_func(cg,"loi_region_alloc","ptr",pt,2,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_region_free","void",pt,1,0);
    }
    {
        char **pt = xmalloc(2*sizeof(char*)); pt[0]=strdup("ptr"); pt[1]=strdup("ptr");
        add_func(cg,"loi_write_file","i32",pt,2,0);
    }
    {
        char **pt = xmalloc(2*sizeof(char*)); pt[0]=strdup("ptr"); pt[1]=strdup("ptr");
        add_func(cg,"loi_thread_spawn","i64",pt,2,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("i64");
        add_func(cg,"loi_thread_join","i32",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("i64");
        add_func(cg,"loi_chan_new","ptr",pt,1,0);
    }
    {
        char **pt = xmalloc(2*sizeof(char*)); pt[0]=strdup("ptr"); pt[1]=strdup("ptr");
        add_func(cg,"loi_chan_send","void",pt,2,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_chan_recv","ptr",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_chan_close","void",pt,1,0);
    }
    {
        char **pt = xmalloc(1*sizeof(char*)); pt[0]=strdup("ptr");
        add_func(cg,"loi_chan_free","void",pt,1,0);
    }
    (void)p_i64;
}

/* emit a top-level global variable; it is registered in the base scope so
   function bodies can load/store it through `ptr @name` */
static void emit_global_var(CG *cg, ASTNode *d) {
    if (!d || !d->name) return;
    char *t = d->ty ? llvm_type(d->ty) : strdup("i32");
    ASTNode *ini = d->init;
    int emitted = 0;
    if (ini && ini->type == AST_EXPR_NULL) {
        fprintf(cg->out, "@%s = global %s null\n", d->name, t);
        emitted = 1;
    } else if (ini && ini->type == AST_EXPR_NUMBER && ini->str_value) {
        const char *lex = ini->str_value;
        if (strcmp(t, "ptr") == 0) {
            fprintf(cg->out, "@%s = global ptr inttoptr (i32 %s to ptr)\n", d->name, lex);
            emitted = 1;
        } else if (t[0] == 'i' || strcmp(t, "float") == 0 || strcmp(t, "double") == 0) {
            fprintf(cg->out, "@%s = global %s %s\n", d->name, t, lex);
            emitted = 1;
        }
    } else if (ini && ini->type == AST_EXPR_STRING && strcmp(t, "ptr") == 0) {
        char *g = get_string_global(cg, ini->str_value ? ini->str_value : "");
        fprintf(cg->out, "@%s = global ptr %s\n", d->name, g);
        emitted = 1;
    } else if (ini && ini->type == AST_EXPR_IDENT && ini->name) {
        if (find_func(cg, ini->name)) {
            fprintf(cg->out, "@%s = global ptr @%s\n", d->name, ini->name);
            emitted = 1;
        } else {
            CGVar *gv = find_var(cg, ini->name);
            if (gv && strcmp(gv->type, "ptr") == 0) {
                fprintf(cg->out, "@%s = global ptr %s\n", d->name, gv->ir);
                emitted = 1;
            }
        }
    }
    if (!emitted) {
        if (strcmp(t, "void") != 0) fprintf(cg->out, "@%s = global %s zeroinitializer\n", d->name, t);
        else { free(t); return; }
    }
    char *irname = malloc(strlen(d->name) + 2);
    sprintf(irname, "@%s", d->name);
    add_var(cg, d->name, irname, t, d->ty);
    free(irname);
}

/* ---------- top-level ---------- */
void codegen_program(ASTNode *prog, FILE *out) {
    CG cg;
    memset(&cg, 0, sizeof(cg));
    cg.out = out;
    add_runtime_funcs(&cg);
    fprintf(out, "target triple = \"aarch64-unknown-linux-gnu\"\n\n");
    fflush(out);
    /* type aliases */
    if (prog && prog->items) {
        for (size_t i = 0; i < prog->item_count; i++) {
            ASTNode *d = prog->items[i];
            if (d && d->type == AST_TYPE_DECL && d->name && d->alias_type && cg_alias_count < 64) {
                cg_aliases[cg_alias_count].name = strdup(d->name);
                cg_aliases[cg_alias_count].ty = d->alias_type;
                cg_alias_count++;
            }
        }
    }
    /* struct types */
    if (prog && prog->items) {
        for (size_t i = 0; i < prog->item_count; i++) {
            ASTNode *d = prog->items[i];
            if (d && d->type == AST_STRUCT_DECL && d->name) {
                StructInfo *si = &struct_infos[struct_info_count++];
                si->name = strdup(d->name);
                si->field_count = (int)d->item_count;
                si->field_names = xmalloc((d->item_count ? d->item_count : 1) * sizeof(char*));
                si->field_types = xmalloc((d->item_count ? d->item_count : 1) * sizeof(char*));
                si->align = d->align;
                si->packed = d->packed;
                for (size_t j = 0; j < d->item_count; j++) {
                    ASTNode *f = d->items[j];
                    if (f && f->packed) si->packed = 1;
                    if (f && f->align > si->align) si->align = f->align;
                }
                if (si->packed) fprintf(out, "%%struct.%s = type <{ ", d->name);
                else fprintf(out, "%%struct.%s = type { ", d->name);
                for (size_t j = 0; j < d->item_count; j++) {
                    ASTNode *f = d->items[j];
                    char *ft = f && f->ty ? llvm_type(f->ty) : strdup("i32");
                    si->field_names[j] = f && f->name ? strdup(f->name) : strdup("field");
                    si->field_types[j] = ft;
                    if (j) fprintf(out, ", ");
                    fprintf(out, "%s", ft);
                }
                if (d->item_count == 0) fprintf(out, "i8");
                if (si->packed) fprintf(out, " }>\n");
                else fprintf(out, " }\n");
            }
        }
        if (struct_info_count > 0) fprintf(out, "\n");
    }
    /* collect strings */
    if (prog) {
        for (size_t i = 0; i < prog->item_count; i++) collect_strings_stmt(&cg, prog->items[i]);
    }
    collect_funcs_cg(&cg, prog);

    /* external functions are declared on demand */
    /* global strings */
    for (size_t i = 0; i < cg.string_values.count; i++) {
        const char *val = cg.string_values.items[i];
        /* emit global array */
        size_t n = strlen(val) + 1;
        fprintf(out, "%s = private unnamed_addr constant [%zu x i8] [", cg.string_names.items[i], n);
        for (size_t j = 0; j < n; j++) {
            unsigned char c = (j < strlen(val)) ? (unsigned char)val[j] : 0;
            if (j) fprintf(out, ", ");
            fprintf(out, "i8 %d", c);
        }
        fprintf(out, "], align 1\n");
    }
    fprintf(out, "\n");

    /* top-level declarations */
    if (prog) {
        /* base scope: globals live here for the whole codegen */
        scope_push(&cg);
        for (size_t i = 0; i < prog->item_count; i++) {
            ASTNode *d = prog->items[i];
            if (d && d->type == AST_VAR_DECL) emit_global_var(&cg, d);
        }
        for (size_t i = 0; i < prog->item_count; i++) {
            ASTNode *d = prog->items[i];
            if (!d) continue;
            if (d->type == AST_FUNC_DECL) {
                codegen_function(&cg, d);
            } else if (d->type == AST_VAR_DECL) {
                /* globals emitted above */
            } else if (d->type == AST_STRUCT_DECL || d->type == AST_TYPE_DECL || d->type == AST_EXTERN_DECL || d->type == AST_PACKAGE) {
                /* skip */
            }
        }
    }
    /* emit declares for undefined (external) functions */
    for (size_t i = 0; i < cg.func_count; i++) {
        if (cg.funcs[i].defined) continue;
        fprintf(out, "declare %s @%s(", cg.funcs[i].retty,
                cg.funcs[i].link_name ? cg.funcs[i].link_name : cg.funcs[i].name);
        for (size_t j = 0; j < cg.funcs[i].param_count; j++) {
            if (j) fprintf(out, ", ");
            fprintf(out, "%s", cg.funcs[i].param_types[j]);
        }
        fprintf(out, ")\n");
    }
}

static long type_size_llvm(const char *t) {
    if (!t) return 4;
    if (strcmp(t, "i1") == 0 || strcmp(t, "i8") == 0) return 1;
    if (strcmp(t, "i16") == 0) return 2;
    if (strcmp(t, "i32") == 0 || strcmp(t, "float") == 0) return 4;
    if (strcmp(t, "i64") == 0 || strcmp(t, "double") == 0) return 8;
    if (strcmp(t, "ptr") == 0) return 8;
    if (t[0] == '[') {
        const char *x = strchr(t, 'x');
        if (x) return atoi(t + 1) * type_size_llvm(x + 1);
        return 0;
    }
    if (t[0] == '%' && strstr(t, "struct.")) {
        const char *nm = t + 7; /* after "struct." */
        long al;
        return struct_size_align(nm, &al);
    }
    return 4;
}

static long type_align_llvm(const char *t) {
    if (!t) return 4;
    if (strcmp(t, "i1") == 0 || strcmp(t, "i8") == 0) return 1;
    if (strcmp(t, "i16") == 0) return 2;
    if (strcmp(t, "i32") == 0 || strcmp(t, "float") == 0) return 4;
    if (strcmp(t, "i64") == 0 || strcmp(t, "double") == 0) return 8;
    if (strcmp(t, "ptr") == 0) return 8;
    if (t[0] == '[') {
        const char *x = strchr(t, 'x');
        if (x) return type_align_llvm(x + 1);
        return 4;
    }
    return 4;
}

static long struct_size_align(const char *name, long *align_out) {
    StructInfo *si = find_struct(name);
    if (!si) { if (align_out) *align_out = 8; return 8; }
    long off = 0, maxal = 1;
    for (int i = 0; i < si->field_count; i++) {
        const char *ft = si->field_types[i];
        long al = si->packed ? 1 : type_align_llvm(ft);
        long fs = type_size_llvm(ft);
        off = (off + al - 1) / al * al;
        off += fs;
        if (al > maxal) maxal = al;
    }
    /* `@align(n)` raises the struct's alignment (never lowers it), so the
       size is padded up to the effective alignment. */
    if (!si->packed && si->align > 0 && si->align > maxal) maxal = si->align;
    if (!si->packed) off = (off + maxal - 1) / maxal * maxal;
    if (align_out) *align_out = maxal;
    return off;
}

static long struct_offset(Type *t, const char *field) {
    StructInfo *si = (t && t->name) ? find_struct(t->name) : NULL;
    if (!si) return 0;
    long off = 0;
    for (int i = 0; i < si->field_count; i++) {
        const char *ft = si->field_types[i];
        long al = si->packed ? 1 : type_align_llvm(ft);
        long fs = type_size_llvm(ft);
        off = (off + al - 1) / al * al;
        if (field && si->field_names[i] && strcmp(si->field_names[i], field) == 0) return off;
        off += fs;
    }
    return 0;
}

static long get_size(Type *t) {
    if (!t) return 4;
    if (t->name && (t->kind == TY_NAMED || t->kind == TY_PRIMITIVE) && find_struct(t->name)) {
        return struct_size_align(t->name, NULL);
    }
    char *lt = llvm_type(t);
    long s = type_size_llvm(lt);
    free(lt);
    return s;
}

static long get_align(Type *t) {
    if (!t) return 4;
    if (t->name && (t->kind == TY_NAMED || t->kind == TY_PRIMITIVE) && find_struct(t->name)) {
        long al;
        struct_size_align(t->name, &al);
        return al;
    }
    char *lt = llvm_type(t);
    long s = type_align_llvm(lt);
    free(lt);
    return s > 0 ? s : 4;
}
