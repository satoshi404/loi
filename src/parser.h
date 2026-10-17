#ifndef LOI_PARSER_H
#define LOI_PARSER_H

#include "lexer.h"
#include "ast.h"

typedef struct {
    Lexer *lex;
    Token current;
    int had_error;
    char current_package[256];
} Parser;

void parser_init(Parser *p, Lexer *lex);
ASTNode *parse_program(Parser *p);

#endif
