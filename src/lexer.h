#ifndef LOI_LEXER_H
#define LOI_LEXER_H

#include "token.h"

typedef struct {
    const char *src;
    size_t length;
    size_t pos;
    int line;
    int column;
    int at_eof;
} Lexer;

void lexer_init(Lexer *lex, const char *src);
void lexer_init_n(Lexer *lex, const char *src, size_t length);

/* Returns next token. Caller owns token.lexeme and must call token_free(). */
Token lexer_next(Lexer *lex);

/* Peek without consuming? Returns copy (lexeme strdup'd). */
Token lexer_peek(Lexer *lex);

#endif /* LOI_LEXER_H */
