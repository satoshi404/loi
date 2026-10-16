#include "ast.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

ASTNode *ast_new(ASTNodeType type, int line, int col) {
    ASTNode *n = calloc(1, sizeof(ASTNode));
    n->type = type;
    n->line = line;
    n->column = col;
    return n;
}

void ast_add_item(ASTNode *parent, ASTNode *item) {
    if (parent->item_count == parent->item_cap) {
        size_t nc = parent->item_cap ? parent->item_cap * 2 : 8;
        parent->items = realloc(parent->items, nc * sizeof(ASTNode *));
        parent->item_cap = nc;
    }
    parent->items[parent->item_count++] = item;
}

Type *type_new(int kind) {
    Type *t = calloc(1, sizeof(Type));
    t->kind = kind;
    return t;
}

Type *type_clone(Type *t) {
    if (!t) return NULL;
    Type *c = type_new(t->kind);
    c->name = t->name ? strdup(t->name) : NULL;
    c->base = type_clone(t->base);
    c->return_type = type_clone(t->return_type);
    if (t->array_size && t->array_size->str_value) {
        ASTNode *sz = ast_new(AST_EXPR_NUMBER, t->array_size->line, t->array_size->column);
        sz->str_value = strdup(t->array_size->str_value);
        c->array_size = sz;
    }
    if (t->param_count) {
        c->param_types = malloc(t->param_count * sizeof(Type *));
        for (size_t i = 0; i < t->param_count; i++) c->param_types[i] = type_clone(t->param_types[i]);
        c->param_count = t->param_count;
    }
    c->effects = t->effects ? strdup(t->effects) : NULL;
    return c;
}

void type_free(Type *t) {
    if (!t) return;
    free(t->name);
    free(t->effects);
    type_free(t->base);
    type_free(t->return_type);
    for (size_t i = 0; i < t->param_count; i++) type_free(t->param_types[i]);
    free(t->param_types);
    ast_free(t->array_size);
    free(t);
}

static void free_list(ASTNode **items, size_t count) {
    for (size_t i = 0; i < count; i++) ast_free(items[i]);
    free(items);
}

void ast_free(ASTNode *node) {
    if (!node) return;
    free(node->name);
    free(node->effects);
    free(node->section);
    type_free(node->ty);
    type_free(node->alias_type);
    ast_free(node->init);
    ast_free(node->left);
    ast_free(node->right);
    ast_free(node->cond);
    ast_free(node->cond_expr);
    ast_free(node->init_stmt);
    ast_free(node->update);
    ast_free(node->step);
    ast_free(node->then_branch);
    ast_free(node->else_branch);
    ast_free(node->body);
    free_list(node->items, node->item_count);
    for (size_t i = 0; i < node->arg_count; i++) ast_free(node->args[i]);
    free(node->args);
    free(node->str_value);
    free(node);
}

const char *ast_node_type_str(ASTNodeType type) {
    switch (type) {
        case AST_PROGRAM: return "PROGRAM";
        case AST_IMPORT: return "IMPORT";
    case AST_PACKAGE: return "PACKAGE";
        case AST_VAR_DECL: return "VAR_DECL";
        case AST_FUNC_DECL: return "FUNC_DECL";
        case AST_STRUCT_DECL: return "STRUCT_DECL";
        case AST_TYPE_DECL: return "TYPE_DECL";
        case AST_EXTERN_DECL: return "EXTERN_DECL";
        case AST_BLOCK: return "BLOCK";
        case AST_IF: return "IF";
        case AST_REGION: return "REGION";
        case AST_CONTRACT: return "CONTRACT";
        case AST_WHILE: return "WHILE";
        case AST_FOR: return "FOR";
        case AST_DO_WHILE: return "DO_WHILE";
        case AST_SWITCH: return "SWITCH";
        case AST_CASE: return "CASE";
        case AST_DEFAULT: return "DEFAULT";
        case AST_RETURN: return "RETURN";
        case AST_BREAK: return "BREAK";
        case AST_CONTINUE: return "CONTINUE";
        case AST_ASM: return "ASM";
        case AST_EXPR_STMT: return "EXPR_STMT";
        case AST_EXPR_BINARY: return "BINARY";
        case AST_EXPR_UNARY: return "UNARY";
        case AST_EXPR_TERNARY: return "TERNARY";
        case AST_EXPR_CALL: return "CALL";
        case AST_EXPR_INDEX: return "INDEX";
        case AST_EXPR_FIELD: return "FIELD";
        case AST_EXPR_IDENT: return "IDENT";
        case AST_EXPR_NUMBER: return "NUMBER";
        case AST_EXPR_STRING: return "STRING";
        case AST_EXPR_CHAR: return "CHAR";
        case AST_EXPR_NULL: return "NULL";
        case AST_EXPR_SIZEOF: return "SIZEOF";
        case AST_EXPR_ALIGNOF: return "ALIGNOF";
        case AST_EXPR_OFFSETOF: return "OFFSETOF";
        default: return "UNKNOWN";
    }
}

static void indent(int n) { for (int i=0;i<n;i++) putchar(' '); }

/* render a type as source-like text for AST dumps */
static void type_to_str_r(Type *t, char *buf, size_t cap) {
    if (!t) { snprintf(buf, cap, "?"); return; }
    if (t->kind == TY_POINTER) {
        char inner[256]; type_to_str_r(t->base, inner, sizeof(inner));
        snprintf(buf, cap, "%s*", inner);
        return;
    }
    if (t->kind == TY_ARRAY) {
        char inner[256]; type_to_str_r(t->base, inner, sizeof(inner));
        if (t->array_size && t->array_size->str_value)
            snprintf(buf, cap, "%s[%s]", inner, t->array_size->str_value);
        else
            snprintf(buf, cap, "%s[]", inner);
        return;
    }
    if (t->kind == TY_FUNCTION) {
        char *p = buf; size_t rem = cap;
        int n = snprintf(p, rem, "(");
        p += n; rem -= (size_t)n;
        for (size_t i = 0; i < t->param_count && rem > 1; i++) {
            char inner[256]; type_to_str_r(t->param_types[i], inner, sizeof(inner));
            n = snprintf(p, rem, "%s%s", i ? ", " : "", inner);
            p += n; rem -= (size_t)n;
        }
        char ret[256]; type_to_str_r(t->return_type, ret, sizeof(ret));
        n = snprintf(p, rem, ") -> %s", ret);
        p += n; rem -= (size_t)n;
        if (t->effects && rem > 2) snprintf(p, rem, " !%s", t->effects);
        return;
    }
    snprintf(buf, cap, "%s", t->name ? t->name : "?");
}

static char *type_to_str(Type *t) {
    char buf[512];
    type_to_str_r(t, buf, sizeof(buf));
    return strdup(buf);
}

void ast_print(ASTNode *node, int in) {
    if (!node) { indent(in); printf("<null>\n"); return; }
    indent(in);
    printf("%s", ast_node_type_str(node->type));
    if (node->name) printf(" %s", node->name);
    if (node->str_value) printf(" %s", node->str_value);
    printf("  (L%d)\n", node->line);
    if (node->ty) {
        char *ts = type_to_str(node->ty);
        indent(in+2);
        printf("type=%s\n", ts);
        free(ts);
    }
    if (node->init) { indent(in+2); printf("init:\n"); ast_print(node->init, in+4); }
    if (node->type == AST_RETURN && node->left) { indent(in+2); printf("value:\n"); ast_print(node->left, in+4); }
    if (node->type == AST_EXPR_STMT && node->left) { ast_print(node->left, in+2); }
    if (node->type == AST_EXPR_INDEX) {
        if (node->left) ast_print(node->left, in+2);
        indent(in+2); printf("index:\n"); ast_print(node->right, in+4);
    }
    /* binary */
    if (node->type == AST_EXPR_BINARY) {
        if (node->left) ast_print(node->left, in+2);
        indent(in+2); printf("op=%d\n", node->op_token);
        if (node->right) ast_print(node->right, in+2);
    }
    if (node->type == AST_EXPR_UNARY) {
        indent(in+2); printf("op=%d\n", node->op_token);
        if (node->left) ast_print(node->left, in+2);
    }
    if (node->type == AST_EXPR_TERNARY) {
        if (node->cond) ast_print(node->cond, in+2);
        if (node->then_branch) { indent(in+2); printf("then:\n"); ast_print(node->then_branch, in+4); }
        if (node->else_branch) { indent(in+2); printf("else:\n"); ast_print(node->else_branch, in+4); }
    }
    if (node->type == AST_EXPR_CALL || node->type == AST_EXPR_INDEX) {
        if (node->left) ast_print(node->left, in+2);
        for (size_t i = 0; i < node->arg_count; i++) ast_print(node->args[i], in+2);
    }
    if (node->type == AST_EXPR_FIELD) {
        if (node->left) ast_print(node->left, in+2);
        indent(in+2); printf(".%s\n", node->name ? node->name : "?");
    }
    if (node->type == AST_EXPR_IDENT || node->type == AST_EXPR_NUMBER ||
        node->type == AST_EXPR_STRING || node->type == AST_EXPR_CHAR) {
        /* leaf already printed */
    }
    if (node->type == AST_FUNC_DECL) {
        indent(in+2); printf("params=%zu\n", node->arg_count);
        for (size_t i = 0; i < node->arg_count; i++) ast_print(node->args[i], in+4);
        if (node->effects) { indent(in+2); printf("effects=%s\n", node->effects); }
    }
    if (node->type == AST_REGION && node->cond) {
        indent(in+2); printf("capacity:\n");
        ast_print(node->cond, in+4);
    }
    if (node->items) {
        for (size_t i = 0; i < node->item_count; i++) ast_print(node->items[i], in+2);
    }
    if (node->body) ast_print(node->body, in+2);
    if (node->init_stmt) ast_print(node->init_stmt, in+2);
    if (node->cond_expr) ast_print(node->cond_expr, in+2);
    if (node->update) ast_print(node->update, in+2);
    if (node->init) { /* expr already handled maybe */ }
    if (node->then_branch && node->type != AST_EXPR_TERNARY) ast_print(node->then_branch, in+2);
    if (node->else_branch && node->type != AST_EXPR_TERNARY) ast_print(node->else_branch, in+2);
    if (node->step) ast_print(node->step, in+2);
}
