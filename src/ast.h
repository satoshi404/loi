#ifndef LOI_AST_H
#define LOI_AST_H

#include "token.h"

typedef enum {
    AST_PROGRAM,
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
    /* lexical capabilities */
    AST_CAP_DECL,   /* @capability(Read, Write, ...) */
    AST_GRANT,      /* @grant(Set) name;  -> introduces a capability binding */
    AST_REFINE,     /* @refine(A -> B) name;  -> weakens an existing binding */
    AST_DROP,       /* @drop(name);       -> consumes a capability binding */
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
        TY_PRIMITIVE, TY_NAMED, TY_POINTER, TY_ARRAY, TY_FUNCTION,
        TY_CAPABILITY   /* @cap(Name1 + Name2), lexical & zero-cost */
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
    /* capability type (TY_CAPABILITY): the permission set it proves, and
       whether it is a `borrow` (non-consuming) proof. */
    char **cap_names;
    size_t cap_count;
    int cap_borrow;
};

struct ASTNode {
    ASTNodeType type;
    int line;
    int column;
    int align;     /* requested alignment (for fields/structs) */
    int packed;   /* packed layout hint */
    char *effects; /* function effects, e.g. "IO,Alloc" */
    char *section; /* target assembly section (metaprogramming @section) */
    /* lexical capabilities (see docs/capabilities.md) */
    char *needs;   /* @needs(A + B)  -> "A,B" */
    char *gives;   /* @gives(A + B)  -> "A,B" */
    char *caps;    /* @grant / @refine(from) / region-with set -> "A,B" */
    char *cap_to;  /* @refine(... -> X) target */
    /* @package / @attr metadata */
    char *package;    /* enclosing @package name (NULL => none) */
    char *visibility; /* "local" / "global" (NULL => global) */
    char *link_name;  /* @attr(link_name("...")) — emitted symbol name */
    char **attr_names;  /* @attr bare flags and named keys */
    char **attr_values; /* matching values (NULL for a bare flag) */
    size_t attr_count;
    size_t attr_cap;
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

/* @attr helpers */
void ast_add_attr(ASTNode *n, const char *name, const char *value);
const char *ast_attr(const ASTNode *n, const char *name);
int ast_has_attr(const ASTNode *n, const char *name);

#endif
