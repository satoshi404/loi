#ifndef LOI_TOKEN_H
#define LOI_TOKEN_H

#include <stddef.h>

typedef enum {
    TOK_EOF = 0,
    TOK_ERROR,

    /* Literals */
    TOK_NUMBER,
    TOK_STRING,
    TOK_CHAR,
    TOK_IDENTIFIER,

    /* Keywords */
    TOK_KW_IMPORT,
    TOK_KW_PACKAGE,
    TOK_KW_MAIN,
    TOK_KW_RETURN,
    TOK_KW_IF,
    TOK_KW_ELSE,
    TOK_KW_WHILE,
    TOK_KW_FOR,
    TOK_KW_FN,
    TOK_KW_LET,
    TOK_KW_CONST,
    TOK_KW_TRUE,
    TOK_KW_FALSE,
    TOK_KW_NULL,
    TOK_KW_BREAK,
    TOK_KW_CONTINUE,
    TOK_KW_SWITCH,
    TOK_KW_CASE,
    TOK_KW_DEFAULT,
    TOK_KW_REQUIRES,
    TOK_KW_ENSURES,
    TOK_KW_REGION,
    TOK_KW_EFFECT,
    TOK_KW_STRUCT,
    TOK_KW_UNION,
    TOK_KW_ENUM,
    TOK_KW_TYPE,
    TOK_KW_EXTERN,
    TOK_KW_ASM,
    TOK_KW_INT,
    TOK_KW_FLOAT,
    TOK_KW_DOUBLE,
    TOK_KW_DO,
    TOK_KW_CHAR,
    TOK_KW_BOOL,
    TOK_KW_VOID,
    TOK_KW_BYTE,
    TOK_KW_LONG,
    TOK_KW_SHORT,
    TOK_KW_UNSIGNED,
    TOK_KW_SIGNED,
    TOK_KW_STATIC,
    TOK_KW_VOLATILE,
    TOK_KW_INLINE,
    TOK_KW_SIZEOF,
    TOK_KW_ALIGNOF,

    /* Arithmetic operators */
    TOK_PLUS,
    TOK_MINUS,
    TOK_STAR,
    TOK_STARSTAR,
    TOK_SLASH,
    TOK_PERCENT,

    /* Increment / decrement */
    TOK_PLUSPLUS,
    TOK_MINUSMINUS,

    /* Assignment */
    TOK_ASSIGN,
    TOK_PLUSEQ,
    TOK_MINUSEQ,
    TOK_STAREQ,
    TOK_SLASHEQ,
    TOK_PERCENTEQ,

    /* Comparison */
    TOK_EQEQ,
    TOK_BANGEQ,
    TOK_LT,
    TOK_LTEQ,
    TOK_GT,
    TOK_GTEQ,

    /* Logical */
    TOK_AMPAMP,
    TOK_PIPEPIPE,
    TOK_BANG,

    /* Bitwise */
    TOK_AMP,
    TOK_PIPE,
    TOK_CARET,
    TOK_TILDE,
    TOK_LSHIFT,
    TOK_RSHIFT,
    /* Compound bitwise/shift assignment */
    TOK_AMPEQ,
    TOK_PIPEEQ,
    TOK_CARETEQ,
    TOK_LSHIFTEQ,
    TOK_RSHIFTEQ,

    /* Delimiters / punctuation */
    TOK_LPAREN,
    TOK_RPAREN,
    TOK_LBRACE,
    TOK_RBRACE,
    TOK_LBRACKET,
    TOK_RBRACKET,
    TOK_COMMA,
    TOK_SEMICOLON,
    TOK_COLON,
    TOK_DOT,
    TOK_ARROW,
    TOK_FAT_ARROW,
    TOK_QUESTION,
    TOK_AT,

    TOK_COUNT
} TokenType;

typedef struct {
    TokenType type;
    char *lexeme;      /* owned, NUL-terminated */
    size_t length;
    int line;          /* 1-based */
    int column;        /* 1-based */
} Token;

const char *token_type_str(TokenType type);
void token_free(Token *tok);

#endif /* LOI_TOKEN_H */
