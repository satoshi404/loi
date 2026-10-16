#ifndef LOI_AST_H
#define LOI_AST_H

#include "token.h"

typedef enum {
    AST_PROGRAM,
    AST_IMPORT,
    AST_PACKAGE,
    AST_VAR_DECL,
    AST_FUNC_DECL,
    AST_STRUCT_DECL,
    AST_TYPE_DECL,
    AST_EXTERN_DECL,
    AST_BLOCK,
    AST_IF,
    AST_REGION,
    AST_CONTRACT,
    AST_WHILE,
    AST_FOR,
    AST_DO_WHILE,
    AST_SWITCH,
    AST_CASE,
    AST_DEFAULT,
    AST_RETURN,
    AST_BREAK,
    AST_CONTINUE,
    AST_ASM,
    AST_EXPR_STMT,
    /* expressions */
    AST_EXPR_BINARY,
    AST_EXPR_UNARY,
    AST_EXPR_TERNARY,
    AST_EXPR_CALL,
    AST_EXPR_INDEX,
    AST_EXPR_FIELD,
    AST_EXPR_IDENT,
    AST_EXPR_NUMBER,
    AST_EXPR_STRING,
    AST_EXPR_CHAR,
    AST_EXPR_NULL,
    AST_EXPR_SIZEOF,
    AST_EXPR_ALIGNOF,
    AST_EXPR_OFFSETOF
} ASTNodeType;

typedef struct Type Type;
typedef struct ASTNode ASTNode;

struct Type {
    enum {
        TY_PRIMITIVE, TY_NAMED, TY_POINTER, TY_ARRAY, TY_FUNCTION
    } kind;
    /* primitive named by token type maybe; store string name */
    char *name;          /* for named/primitive/as kind */
    Type *base;          /* for pointer/array/function */
    ASTNode *array_size; /* for array */
    /* function type */
    Type **param_types;
    size_t param_count;
    Type *return_type;
    char *effects;      /* function type effects, e.g. "IO,Alloc" (TY_FUNCTION) */
};

struct ASTNode {
    ASTNodeType type;
    int line;
    int column;
    int align;     /* requested alignment (for fields/structs) */
    int packed;   /* packed layout hint */
    char *effects; /* function effects, e.g. "IO,Alloc" */
    char *section; /* target assembly section (metaprogramming @section) */
    /* common */
    char *name;         /* identifier name */
    Type *ty;           /* declared type */
    ASTNode *init;      /* initializer expression */
    /* lists */
    ASTNode **items;
    size_t item_count;
    size_t item_cap;
    /* expressions */
    int op_token;       /* TokenType as int for operators */
    ASTNode *left;
    ASTNode *right;
    ASTNode *cond;
    ASTNode *then_branch;
    ASTNode *else_branch;
    ASTNode *body;
    ASTNode *step;
    ASTNode *update;
    ASTNode **args;
    size_t arg_count;
    size_t arg_cap;
    char *str_value;    /* for literals/strings */
    /* for/while */
    ASTNode *init_stmt;
    ASTNode *cond_expr;
    /* type decl */
    Type *alias_type;
};

/* helpers */
ASTNode *ast_new(ASTNodeType type, int line, int col);
void ast_add_item(ASTNode *parent, ASTNode *item);
Type *type_new(int kind);
Type *type_clone(Type *t);
void type_free(Type *t);
void ast_free(ASTNode *node);
void ast_print(ASTNode *node, int indent);

const char *ast_node_type_str(ASTNodeType type);

#endif
