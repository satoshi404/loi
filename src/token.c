#include "token.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

const char *token_type_str(TokenType type) {
    switch (type) {
        case TOK_EOF: return "EOF";
        case TOK_ERROR: return "ERROR";
        case TOK_NUMBER: return "NUMBER";
        case TOK_STRING: return "STRING";
        case TOK_CHAR: return "CHAR";
        case TOK_IDENTIFIER: return "IDENTIFIER";
        case TOK_KW_MAIN: return "KW_MAIN";
        case TOK_KW_RETURN: return "KW_RETURN";
        case TOK_KW_IF: return "KW_IF";
        case TOK_KW_ELSE: return "KW_ELSE";
        case TOK_KW_WHILE: return "KW_WHILE";
        case TOK_KW_FOR: return "KW_FOR";
        case TOK_KW_FN: return "KW_FN";
        case TOK_KW_LET: return "KW_LET";
        case TOK_KW_CONST: return "KW_CONST";
        case TOK_KW_TRUE: return "KW_TRUE";
        case TOK_KW_FALSE: return "KW_FALSE";
        case TOK_KW_NULL: return "KW_NULL";
        case TOK_KW_BREAK: return "KW_BREAK";
        case TOK_KW_CONTINUE: return "KW_CONTINUE";
        case TOK_KW_REQUIRES: return "KW_REQUIRES";
        case TOK_KW_ENSURES: return "KW_ENSURES";
        case TOK_KW_REGION: return "KW_REGION";
        case TOK_KW_EFFECT: return "KW_EFFECT";
        case TOK_KW_STRUCT: return "KW_STRUCT";
        case TOK_KW_UNION: return "KW_UNION";
        case TOK_KW_ENUM: return "KW_ENUM";
        case TOK_KW_TYPE: return "KW_TYPE";
        case TOK_KW_EXTERN: return "KW_EXTERN";
        case TOK_KW_ASM: return "KW_ASM";
        case TOK_KW_INT: return "KW_INT";
        case TOK_KW_FLOAT: return "KW_FLOAT";
        case TOK_KW_DOUBLE: return "KW_DOUBLE";
        case TOK_KW_CHAR: return "KW_CHAR";
        case TOK_KW_BOOL: return "KW_BOOL";
        case TOK_KW_VOID: return "KW_VOID";
        case TOK_KW_BYTE: return "KW_BYTE";
        case TOK_KW_LONG: return "KW_LONG";
        case TOK_KW_SHORT: return "KW_SHORT";
        case TOK_KW_UNSIGNED: return "KW_UNSIGNED";
        case TOK_KW_SIGNED: return "KW_SIGNED";
        case TOK_KW_STATIC: return "KW_STATIC";
        case TOK_KW_VOLATILE: return "KW_VOLATILE";
        case TOK_KW_INLINE: return "KW_INLINE";
        case TOK_KW_SIZEOF: return "KW_SIZEOF";
        case TOK_KW_ALIGNOF: return "KW_ALIGNOF";
        case TOK_PLUS: return "PLUS";
        case TOK_MINUS: return "MINUS";
        case TOK_STAR: return "STAR";
        case TOK_STARSTAR: return "STARSTAR";
        case TOK_SLASH: return "SLASH";
        case TOK_PERCENT: return "PERCENT";
        case TOK_PLUSPLUS: return "PLUSPLUS";
        case TOK_MINUSMINUS: return "MINUSMINUS";
        case TOK_ASSIGN: return "ASSIGN";
        case TOK_PLUSEQ: return "PLUSEQ";
        case TOK_MINUSEQ: return "MINUSEQ";
        case TOK_STAREQ: return "STAREQ";
        case TOK_SLASHEQ: return "SLASHEQ";
        case TOK_PERCENTEQ: return "PERCENTEQ";
        case TOK_EQEQ: return "EQEQ";
        case TOK_BANGEQ: return "BANGEQ";
        case TOK_LT: return "LT";
        case TOK_LTEQ: return "LTEQ";
        case TOK_GT: return "GT";
        case TOK_GTEQ: return "GTEQ";
        case TOK_AMPAMP: return "AMPAMP";
        case TOK_PIPEPIPE: return "PIPEPIPE";
        case TOK_BANG: return "BANG";
        case TOK_AMP: return "AMP";
        case TOK_PIPE: return "PIPE";
        case TOK_CARET: return "CARET";
        case TOK_TILDE: return "TILDE";
        case TOK_LSHIFT: return "LSHIFT";
        case TOK_RSHIFT: return "RSHIFT";
        case TOK_AMPEQ: return "AMPEQ";
        case TOK_PIPEEQ: return "PIPEEQ";
        case TOK_CARETEQ: return "CARETEQ";
        case TOK_LSHIFTEQ: return "LSHIFTEQ";
        case TOK_RSHIFTEQ: return "RSHIFTEQ";
    case TOK_KW_SWITCH: return "KW_SWITCH";
    case TOK_KW_CASE: return "KW_CASE";
    case TOK_KW_DEFAULT: return "KW_DEFAULT";
        case TOK_LPAREN: return "LPAREN";
        case TOK_RPAREN: return "RPAREN";
        case TOK_LBRACE: return "LBRACE";
        case TOK_RBRACE: return "RBRACE";
        case TOK_LBRACKET: return "LBRACKET";
        case TOK_RBRACKET: return "RBRACKET";
        case TOK_COMMA: return "COMMA";
        case TOK_SEMICOLON: return "SEMICOLON";
        case TOK_COLON: return "COLON";
        case TOK_DOT: return "DOT";
        case TOK_ARROW: return "ARROW";
        case TOK_FAT_ARROW: return "FAT_ARROW";
        case TOK_QUESTION: return "QUESTION";
        case TOK_AT: return "AT";
        default: return "UNKNOWN";
    }
}

void token_free(Token *tok) {
    if (!tok) return;
    free(tok->lexeme);
    tok->lexeme = NULL;
    tok->length = 0;
}

/* Provide a helper implemented in lexer.c for freeing lexeme to avoid
   double ownership ambiguity; here we do nothing and let lexer handle. */
