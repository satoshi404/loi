#include "lexer.h"

#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char peek_char(Lexer *lex, size_t off) {
    size_t i = lex->pos + off;
    if (i >= lex->length) return '\0';
    return lex->src[i];
}

static char advance(Lexer *lex) {
    char c;
    if (lex->pos >= lex->length) {
        lex->at_eof = 1;
        return '\0';
    }
    c = lex->src[lex->pos++];
    if (c == '\n') {
        lex->line++;
        lex->column = 1;
    } else {
        lex->column++;
    }
    return c;
}

static Token make_token(Lexer *lex, TokenType type, const char *start, size_t length, int line, int col) {
    (void)lex;
    Token t;
    t.type = type;
    t.lexeme = (char *)malloc(length + 1);
    if (t.lexeme) {
        memcpy(t.lexeme, start, length);
        t.lexeme[length] = '\0';
    } else {
        t.lexeme = NULL;
    }
    t.length = length;
    t.line = line;
    t.column = col;
    return t;
}

static Token make_error(Lexer *lex, const char *msg, int line, int col) {
    (void)lex;
    Token t;
    size_t len = strlen(msg);
    t.type = TOK_ERROR;
    t.lexeme = (char *)malloc(len + 1);
    if (t.lexeme) {
        memcpy(t.lexeme, msg, len + 1);
    }
    t.length = len;
    t.line = line;
    t.column = col;
    return t;
}

static void skip_whitespace_and_comments(Lexer *lex) {
    for (;;) {
        char c = (lex->pos < lex->length) ? lex->src[lex->pos] : '\0';
        if (c == ' ' || c == '\t' || c == '\r' || c == '\n') {
            advance(lex);
        } else if (c == '/' && peek_char(lex, 1) == '/') {
            /* line comment */
            advance(lex);
            advance(lex);
            while (lex->pos < lex->length && lex->src[lex->pos] != '\n') {
                advance(lex);
            }
        } else if (c == '/' && peek_char(lex, 1) == '*') {
            /* block comment */
            advance(lex);
            advance(lex);
            int closed = 0;
            while (lex->pos < lex->length) {
                if (lex->src[lex->pos] == '*' && peek_char(lex, 1) == '/') {
                    advance(lex);
                    advance(lex);
                    closed = 1;
                    break;
                }
                advance(lex);
            }
            if (!closed) {
                /* unterminated block comment: stop scanning, the parser reports it */
            }
        } else {
            break;
        }
    }
}

static int is_ident_start(char c) {
    return isalpha((unsigned char)c) || c == '_';
}
static int is_ident_char(char c) {
    return isalnum((unsigned char)c) || c == '_';
}

static TokenType keyword_type(const char *s, size_t len) {
    /* simple lookup */
    if (len == 6 && strncmp(s, "import", 6) == 0) return TOK_KW_IMPORT;
    if (len == 4 && strncmp(s, "main", 4) == 0) return TOK_KW_MAIN;
    if (len == 6 && strncmp(s, "return", 6) == 0) return TOK_KW_RETURN;
    if (len == 2 && strncmp(s, "if", 2) == 0) return TOK_KW_IF;
    if (len == 4 && strncmp(s, "else", 4) == 0) return TOK_KW_ELSE;
    if (len == 5 && strncmp(s, "while", 5) == 0) return TOK_KW_WHILE;
    if (len == 3 && strncmp(s, "for", 3) == 0) return TOK_KW_FOR;
    if (len == 2 && strncmp(s, "fn", 2) == 0) return TOK_KW_FN;
    if (len == 3 && strncmp(s, "let", 3) == 0) return TOK_KW_LET;
    if (len == 5 && strncmp(s, "const", 5) == 0) return TOK_KW_CONST;
    if (len == 4 && strncmp(s, "true", 4) == 0) return TOK_KW_TRUE;
    if (len == 5 && strncmp(s, "false", 5) == 0) return TOK_KW_FALSE;
    if (len == 4 && strncmp(s, "null", 4) == 0) return TOK_KW_NULL;
    if (len == 5 && strncmp(s, "break", 5) == 0) return TOK_KW_BREAK;
    if (len == 8 && strncmp(s, "continue", 8) == 0) return TOK_KW_CONTINUE;
    if (len == 8 && strncmp(s, "requires", 8) == 0) return TOK_KW_REQUIRES;
    if (len == 7 && strncmp(s, "ensures", 7) == 0) return TOK_KW_ENSURES;
    if (len == 6 && strncmp(s, "region", 6) == 0) return TOK_KW_REGION;
    if (len == 6 && strncmp(s, "effect", 6) == 0) return TOK_KW_EFFECT;
    if (len == 4 && strncmp(s, "pure", 4) == 0) return TOK_KW_PURE;
    if (len == 6 && strncmp(s, "struct", 6) == 0) return TOK_KW_STRUCT;
    if (len == 5 && strncmp(s, "union", 5) == 0) return TOK_KW_UNION;
    if (len == 4 && strncmp(s, "enum", 4) == 0) return TOK_KW_ENUM;
    if (len == 4 && strncmp(s, "type", 4) == 0) return TOK_KW_TYPE;
    if (len == 6 && strncmp(s, "extern", 6) == 0) return TOK_KW_EXTERN;
    if (len == 3 && strncmp(s, "asm", 3) == 0) return TOK_KW_ASM;
    if (len == 3 && strncmp(s, "int", 3) == 0) return TOK_KW_INT;
    if (len == 5 && strncmp(s, "float", 5) == 0) return TOK_KW_FLOAT;
    if (len == 6 && strncmp(s, "double", 6) == 0) return TOK_KW_DOUBLE;
    if (len == 4 && strncmp(s, "char", 4) == 0) return TOK_KW_CHAR;
    if (len == 4 && strncmp(s, "bool", 4) == 0) return TOK_KW_BOOL;
    if (len == 4 && strncmp(s, "void", 4) == 0) return TOK_KW_VOID;
    if (len == 4 && strncmp(s, "byte", 4) == 0) return TOK_KW_BYTE;
    if (len == 4 && strncmp(s, "long", 4) == 0) return TOK_KW_LONG;
    if (len == 5 && strncmp(s, "short", 5) == 0) return TOK_KW_SHORT;
    if (len == 8 && strncmp(s, "unsigned", 8) == 0) return TOK_KW_UNSIGNED;
    if (len == 6 && strncmp(s, "signed", 6) == 0) return TOK_KW_SIGNED;
    if (len == 6 && strncmp(s, "static", 6) == 0) return TOK_KW_STATIC;
    if (len == 8 && strncmp(s, "volatile", 8) == 0) return TOK_KW_VOLATILE;
    if (len == 6 && strncmp(s, "inline", 6) == 0) return TOK_KW_INLINE;
    if (len == 6 && strncmp(s, "sizeof", 6) == 0) return TOK_KW_SIZEOF;
    if (len == 7 && strncmp(s, "alignof", 7) == 0) return TOK_KW_ALIGNOF;
    return TOK_IDENTIFIER;
}

static Token lex_number(Lexer *lex) {
    int line = lex->line;
    int col = lex->column;
    const char *start = lex->src + lex->pos;
    /* handle prefixes */
    if (peek_char(lex, 0) == '0') {
        char p = peek_char(lex, 1);
        if (p == 'x' || p == 'X') {
            advance(lex); advance(lex);
            while (isxdigit((unsigned char)peek_char(lex, 0))) advance(lex);
            size_t len = (lex->src + lex->pos) - start;
            return make_token(lex, TOK_NUMBER, start, len, line, col);
        }
        if (p == 'b' || p == 'B') {
            advance(lex); advance(lex);
            while (peek_char(lex, 0) == '0' || peek_char(lex, 0) == '1') advance(lex);
            size_t len = (lex->src + lex->pos) - start;
            return make_token(lex, TOK_NUMBER, start, len, line, col);
        }
    }
    /* integer / float */
    while (isdigit((unsigned char)peek_char(lex, 0))) advance(lex);
    if (peek_char(lex, 0) == '.' && isdigit((unsigned char)peek_char(lex, 1))) {
        advance(lex);
        while (isdigit((unsigned char)peek_char(lex, 0))) advance(lex);
    }
    /* exponent */
    if (peek_char(lex, 0) == 'e' || peek_char(lex, 0) == 'E') {
        advance(lex);
        if (peek_char(lex, 0) == '+' || peek_char(lex, 0) == '-') advance(lex);
        while (isdigit((unsigned char)peek_char(lex, 0))) advance(lex);
    }
    size_t len = (lex->src + lex->pos) - start;
    return make_token(lex, TOK_NUMBER, start, len, line, col);
}

static Token lex_string(Lexer *lex) {
    int line = lex->line;
    int col = lex->column;
    /* skip opening quote */
    advance(lex); /* " */
    size_t cap = 64;
    char *buf = (char *)malloc(cap);
    size_t len = 0;
    while (lex->pos < lex->length) {
        char c = lex->src[lex->pos];
        if (c == '"') {
            advance(lex);
            buf[len] = '\0';
            /* the lexeme holds the decoded string contents (no surrounding quotes) */
            Token t;
            t.type = TOK_STRING;
            t.lexeme = buf;
            t.length = len;
            t.line = line;
            t.column = col;
            return t;
        }
        if (c == '\n') {
            free(buf);
            return make_error(lex, "unterminated string", line, col);
        }
        if (c == '\\') {
            advance(lex);
            char e = (lex->pos < lex->length) ? lex->src[lex->pos] : '\0';
            char out = e;
            switch (e) {
                case 'n': out = '\n'; break;
                case 't': out = '\t'; break;
                case 'r': out = '\r'; break;
                case '\\': out = '\\'; break;
                case '"': out = '"'; break;
                case '0': out = '\0'; break;
                default: out = e; break;
            }
            if (len + 1 >= cap) { cap *= 2; buf = realloc(buf, cap); }
            buf[len++] = out;
            advance(lex);
        } else {
            if (len + 1 >= cap) { cap *= 2; buf = realloc(buf, cap); }
            buf[len++] = c;
            advance(lex);
        }
    }
    free(buf);
    return make_error(lex, "unterminated string", line, col);
}

static Token lex_char(Lexer *lex) {
    int line = lex->line;
    int col = lex->column;
    advance(lex); /* ' */
    char out = '\0';
    if (lex->pos < lex->length && lex->src[lex->pos] == '\\') {
        advance(lex);
        char e = (lex->pos < lex->length) ? lex->src[lex->pos] : '\0';
        switch (e) {
            case 'n': out = '\n'; break;
            case 't': out = '\t'; break;
            case 'r': out = '\r'; break;
            case '\\': out = '\\'; break;
            case '\'': out = '\''; break;
            default: out = e; break;
        }
        advance(lex);
    } else if (lex->pos < lex->length) {
        out = lex->src[lex->pos];
        advance(lex);
    }
    /* expect closing */
    if (lex->pos < lex->length && lex->src[lex->pos] == '\'') {
        advance(lex);
    } else {
        return make_error(lex, "unterminated char literal", line, col);
    }
    char *buf = (char *)malloc(2);
    buf[0] = out;
    buf[1] = '\0';
    Token t;
    t.type = TOK_CHAR;
    t.lexeme = buf;
    t.length = 1;
    t.line = line;
    t.column = col;
    return t;
}

static Token lex_identifier(Lexer *lex) {
    int line = lex->line;
    int col = lex->column;
    const char *start = lex->src + lex->pos;
    while (lex->pos < lex->length && is_ident_char(lex->src[lex->pos])) {
        advance(lex);
    }
    size_t len = (lex->src + lex->pos) - start;
    char *tmp = (char *)malloc(len + 1);
    memcpy(tmp, start, len);
    tmp[len] = '\0';
    TokenType kw = keyword_type(tmp, len);
    free(tmp);
    return make_token(lex, kw, start, len, line, col);
}

void lexer_init(Lexer *lex, const char *src) {
    if (!src) src = "";
    lexer_init_n(lex, src, strlen(src));
}

void lexer_init_n(Lexer *lex, const char *src, size_t length) {
    lex->src = src ? src : "";
    lex->length = length;
    lex->pos = 0;
    lex->line = 1;
    lex->column = 1;
    lex->at_eof = (length == 0);
}

Token lexer_next(Lexer *lex) {
    skip_whitespace_and_comments(lex);
    if (lex->pos >= lex->length) {
        lex->at_eof = 1;
        return make_token(lex, TOK_EOF, "", 0, lex->line, lex->column);
    }
    int line = lex->line;
    int col = lex->column;
    char c = lex->src[lex->pos];

    if (is_ident_start(c)) {
        return lex_identifier(lex);
    }
    if (isdigit((unsigned char)c)) {
        return lex_number(lex);
    }
    if (c == '"') {
        return lex_string(lex);
    }
    if (c == '\'') {
        return lex_char(lex);
    }

    /* Multi-char operators */
    char n1 = peek_char(lex, 1);
    /* three? none currently */
    char n2 = peek_char(lex, 2);
    /* three-char */
    if (c == '<' && n1 == '<' && n2 == '=') { advance(lex); advance(lex); advance(lex); return make_token(lex, TOK_LSHIFTEQ, "<<=", 3, line, col); }
    if (c == '>' && n1 == '>' && n2 == '=') { advance(lex); advance(lex); advance(lex); return make_token(lex, TOK_RSHIFTEQ, ">>=", 3, line, col); }
    /* two-char */
    if (c == '=' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_EQEQ, "==", 2, line, col); }
    if (c == '!' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_BANGEQ, "!=", 2, line, col); }
    if (c == '<' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_LTEQ, "<=", 2, line, col); }
    if (c == '>' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_GTEQ, ">=", 2, line, col); }
    if (c == '&' && n1 == '&') { advance(lex); advance(lex); return make_token(lex, TOK_AMPAMP, "&&", 2, line, col); }
    if (c == '|' && n1 == '|') { advance(lex); advance(lex); return make_token(lex, TOK_PIPEPIPE, "||", 2, line, col); }
    if (c == '+' && n1 == '+') { advance(lex); advance(lex); return make_token(lex, TOK_PLUSPLUS, "++", 2, line, col); }
    if (c == '-' && n1 == '-') { advance(lex); advance(lex); return make_token(lex, TOK_MINUSMINUS, "--", 2, line, col); }
    if (c == '+' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_PLUSEQ, "+=", 2, line, col); }
    if (c == '-' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_MINUSEQ, "-=", 2, line, col); }
    if (c == '*' && n1 == '*') { advance(lex); advance(lex); return make_token(lex, TOK_STARSTAR, "**", 2, line, col); }
    if (c == '*' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_STAREQ, "*=", 2, line, col); }
    if (c == '/' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_SLASHEQ, "/=", 2, line, col); }
    if (c == '%' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_PERCENTEQ, "%=", 2, line, col); }
    if (c == '-' && n1 == '>') { advance(lex); advance(lex); return make_token(lex, TOK_ARROW, "->", 2, line, col); }
    if (c == '<' && n1 == '<') { advance(lex); advance(lex); return make_token(lex, TOK_LSHIFT, "<<", 2, line, col); }
    if (c == '>' && n1 == '>') { advance(lex); advance(lex); return make_token(lex, TOK_RSHIFT, ">>", 2, line, col); }
    if (c == '&' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_AMPEQ, "&=", 2, line, col); }
    if (c == '|' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_PIPEEQ, "|=", 2, line, col); }
    if (c == '^' && n1 == '=') { advance(lex); advance(lex); return make_token(lex, TOK_CARETEQ, "^=", 2, line, col); }

    /* single char */
    advance(lex);
    switch (c) {
        case '+': return make_token(lex, TOK_PLUS, "+", 1, line, col);
        case '-': return make_token(lex, TOK_MINUS, "-", 1, line, col);
        case '*': return make_token(lex, TOK_STAR, "*", 1, line, col);
        case '/': return make_token(lex, TOK_SLASH, "/", 1, line, col);
        case '%': return make_token(lex, TOK_PERCENT, "%", 1, line, col);
        case '=': return make_token(lex, TOK_ASSIGN, "=", 1, line, col);
        case '<': return make_token(lex, TOK_LT, "<", 1, line, col);
        case '>': return make_token(lex, TOK_GT, ">", 1, line, col);
        case '!': return make_token(lex, TOK_BANG, "!", 1, line, col);
        case '&': return make_token(lex, TOK_AMP, "&", 1, line, col);
        case '|': return make_token(lex, TOK_PIPE, "|", 1, line, col);
        case '^': return make_token(lex, TOK_CARET, "^", 1, line, col);
        case '~': return make_token(lex, TOK_TILDE, "~", 1, line, col);
        case '(': return make_token(lex, TOK_LPAREN, "(", 1, line, col);
        case ')': return make_token(lex, TOK_RPAREN, ")", 1, line, col);
        case '{': return make_token(lex, TOK_LBRACE, "{", 1, line, col);
        case '}': return make_token(lex, TOK_RBRACE, "}", 1, line, col);
        case '[': return make_token(lex, TOK_LBRACKET, "[", 1, line, col);
        case ']': return make_token(lex, TOK_RBRACKET, "]", 1, line, col);
        case ',': return make_token(lex, TOK_COMMA, ",", 1, line, col);
        case ';': return make_token(lex, TOK_SEMICOLON, ";", 1, line, col);
        case ':': return make_token(lex, TOK_COLON, ":", 1, line, col);
        case '.': return make_token(lex, TOK_DOT, ".", 1, line, col);
        case '?': return make_token(lex, TOK_QUESTION, "?", 1, line, col);
        case '@': return make_token(lex, TOK_AT, "@", 1, line, col);
        default: {
            char buf[256];
            snprintf(buf, sizeof(buf), "unexpected character '%c'", c);
            return make_error(lex, buf, line, col);
        }
    }
}

Token lexer_peek(Lexer *lex) {
    /* Save state */
    Lexer saved = *lex;
    Token t = lexer_next(lex);
    /* Restore state; but token lexeme ownership: we cloned? Need clone */
    /* We can't simply restore because t owns lexeme. Return token and restore lexer. */
    *lex = saved;
    /* However lexeme of t still valid; caller frees. OK. */
    return t;
}
