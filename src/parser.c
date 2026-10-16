#include "parser.h"
#include "ast.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void advance_parser(Parser *p) {
    token_free(&p->current);
    p->current = lexer_next(p->lex);
}

static int check(Parser *p, TokenType t) {
    return p->current.type == t;
}

static int match_tok(Parser *p, TokenType t) {
    if (p->current.type == t) {
        advance_parser(p);
        return 1;
    }
    return 0;
}

static void parser_error(Parser *p, const char *msg) {
    fprintf(stderr, "Parse error at %d:%d: %s (got %s)\n",
            p->current.line, p->current.column, msg, token_type_str(p->current.type));
    p->had_error = 1;
}

static int expect(Parser *p, TokenType t, const char *msg) {
    if (p->current.type == t) {
        advance_parser(p);
        return 1;
    }
    parser_error(p, msg);
    return 0;
}

static int is_type_keyword(Parser *p) {
    TokenType t = p->current.type;
    return t == TOK_KW_INT || t == TOK_KW_FLOAT || t == TOK_KW_DOUBLE ||
           t == TOK_KW_CHAR || t == TOK_KW_BOOL || t == TOK_KW_VOID ||
           t == TOK_KW_BYTE || t == TOK_KW_SHORT || t == TOK_KW_LONG ||
           t == TOK_KW_UNSIGNED || t == TOK_KW_SIGNED ||
           t == TOK_KW_STRUCT || t == TOK_KW_UNION || t == TOK_KW_ENUM ||
           t == TOK_KW_CONST || t == TOK_KW_STATIC || t == TOK_KW_VOLATILE ||
           t == TOK_KW_INLINE || t == TOK_KW_REGION || t == TOK_KW_EFFECT;
}

static int is_type_start(Parser *p) {
    TokenType t = p->current.type;
    if (t == TOK_KW_INT || t == TOK_KW_FLOAT || t == TOK_KW_DOUBLE ||
        t == TOK_KW_CHAR || t == TOK_KW_BOOL || t == TOK_KW_VOID ||
        t == TOK_KW_BYTE || t == TOK_KW_SHORT || t == TOK_KW_LONG ||
        t == TOK_KW_UNSIGNED || t == TOK_KW_SIGNED ||
        t == TOK_KW_STRUCT || t == TOK_KW_UNION || t == TOK_KW_ENUM ||
        t == TOK_KW_CONST || t == TOK_KW_STATIC || t == TOK_KW_VOLATILE ||
        t == TOK_KW_INLINE || t == TOK_IDENTIFIER || t == TOK_KW_FN ||
        t == TOK_KW_REGION || t == TOK_KW_EFFECT ||
        t == TOK_AT || t == TOK_LPAREN)
        return 1;
    return 0;
}

static Type *parse_type(Parser *p);
static ASTNode *parse_expr(Parser *p);

/* lookahead: at '(' — is the matching ')' followed by '->'?
   Distinguishes a parenthesized function type `(a: int) -> int` from a
   grouped type `(int)`. Uses a lexer copy; the parser is untouched. */
static int at_fn_type(Parser *p) {
    if (!check(p, TOK_LPAREN)) return 0;
    Lexer l = *p->lex; /* struct copy; lexing it does not disturb the parser */
    int depth = 1;
    for (;;) {
        Token t = lexer_next(&l);
        if (t.type == TOK_EOF) { token_free(&t); return 0; }
        if (t.type == TOK_LPAREN) depth++;
        else if (t.type == TOK_RPAREN) {
            depth--;
            if (depth == 0) {
                token_free(&t);
                Token n = lexer_next(&l);
                int arrow = (n.type == TOK_ARROW);
                token_free(&n);
                return arrow;
            }
        }
        token_free(&t);
    }
}

/* `!IO,Alloc` effect list (shared by function declarations and function types) */
static char *parse_effects(Parser *p) {
    if (!match_tok(p, TOK_BANG)) return NULL;
    size_t cap = 64, len = 0;
    char *buf = malloc(cap); buf[0] = 0;
    while (check(p, TOK_IDENTIFIER)) {
        const char *s = p->current.lexeme;
        size_t sl = strlen(s);
        if (len + sl + 2 >= cap) { while (len + sl + 2 >= cap) cap *= 2; buf = realloc(buf, cap); }
        if (len) buf[len++] = ',';
        memcpy(buf + len, s, sl + 1);
        len += sl;
        advance_parser(p);
        if (check(p, TOK_COMMA)) advance_parser(p);
        else break;
    }
    return buf;
}

/* --- parser symbol table for @typeof inference --- */
typedef struct { char name[64]; Type *ty; } SymEnt;
static SymEnt g_symtab[1024];
static int g_nsym = 0;
static void sym_set(const char *name, Type *ty) {
    if (!name) return;
    for (int i = 0; i < g_nsym; i++) {
        if (strcmp(g_symtab[i].name, name) == 0) { g_symtab[i].ty = ty; return; }
    }
    if (g_nsym < 1024) { snprintf(g_symtab[g_nsym].name, sizeof(g_symtab[0].name), "%s", name); g_symtab[g_nsym].ty = ty; g_nsym++; }
}
static Type *sym_get(const char *name) {
    for (int i = 0; i < g_nsym; i++) if (strcmp(g_symtab[i].name, name) == 0) return g_symtab[i].ty;
    return NULL;
}
static Type *infer_type_expr(ASTNode *e) {
    Type *t = type_new(TY_PRIMITIVE);
    if (!e) { t->name = strdup("int"); return t; }
    if (e->type == AST_EXPR_IDENT) {
        Type *st = sym_get(e->name);
        if (st) return type_clone(st);
        t->name = strdup("int"); return t;
    }
    if (e->type == AST_EXPR_STRING) { t->name = strdup("ptr"); return t; }
    if (e->type == AST_EXPR_CHAR) { t->name = strdup("char"); return t; }
    if (e->type == AST_EXPR_NUMBER) {
        const char *v = e->str_value ? e->str_value : "";
        t->name = strdup((strchr(v, '.') || strchr(v, 'e') || strchr(v, 'E')) ? "double" : "int");
        return t;
    }
    t->name = strdup("int"); return t;
}

static Type *parse_type(Parser *p) {
    Type *t = NULL;
    /* Arrays use the canonical postfix form `T[n]` / `T[]` (handled in the
       suffix loop below). The old prefix form `[n]T` is intentionally not
       parsed: it was unreachable as a declaration start and only caused
       ambiguity. */
    while (check(p, TOK_KW_CONST) || check(p, TOK_KW_STATIC) ||
           check(p, TOK_KW_VOLATILE) || check(p, TOK_KW_INLINE)) {
        advance_parser(p);
    }
    if (check(p, TOK_AT)) {
        advance_parser(p);
        if (check(p, TOK_IDENTIFIER) && strcmp(p->current.lexeme, "typeof") == 0) {
            advance_parser(p);
            expect(p, TOK_LPAREN, "expected '('");
            ASTNode *ex = parse_expr(p);
            expect(p, TOK_RPAREN, "expected ')'");
            t = infer_type_expr(ex);
            ast_free(ex);
            goto type_suffix;
        }
    }
    if (check(p, TOK_LPAREN)) {
        if (at_fn_type(p)) {
            /* `(a: int, b: int) -> int !Effects` — function type */
            t = type_new(TY_FUNCTION);
            advance_parser(p);
            if (!check(p, TOK_RPAREN)) {
                size_t cap = 4, cnt = 0;
                Type **arr = malloc(cap * sizeof(Type *));
                do {
                    if (cnt == cap) { cap *= 2; arr = realloc(arr, cap * sizeof(Type *)); }
                    if (check(p, TOK_IDENTIFIER)) {
                        Token nx = lexer_peek(p->lex);
                        int named = (nx.type == TOK_COLON);
                        token_free(&nx);
                        if (named) {
                            advance_parser(p);
                            expect(p, TOK_COLON, "expected ':'");
                        }
                    }
                    arr[cnt++] = parse_type(p);
                } while (match_tok(p, TOK_COMMA));
                t->param_types = arr;
                t->param_count = cnt;
            }
            expect(p, TOK_RPAREN, "expected ')'");
            expect(p, TOK_ARROW, "expected '->'");
            t->return_type = parse_type(p);
            t->effects = parse_effects(p);
        } else {
            advance_parser(p);
            t = parse_type(p);
            expect(p, TOK_RPAREN, "expected ')'");
        }
    } else if (check(p, TOK_KW_STRUCT) || check(p, TOK_KW_UNION) || check(p, TOK_KW_ENUM)) {
        TokenType kw = p->current.type;
        advance_parser(p);
        char *name = NULL;
        if (check(p, TOK_IDENTIFIER)) {
            name = p->current.lexeme ? strdup(p->current.lexeme) : NULL;
            advance_parser(p);
        }
        t = type_new(TY_NAMED);
        t->name = name ? name : (kw == TOK_KW_STRUCT ? strdup("struct") : (kw == TOK_KW_UNION ? strdup("union") : strdup("enum")));
    } else if (check(p, TOK_KW_FN)) {
        advance_parser(p);
        t = type_new(TY_FUNCTION);
        expect(p, TOK_LPAREN, "expected '('");
        if (!check(p, TOK_RPAREN)) {
            size_t cap = 4, cnt = 0;
            Type **arr = malloc(cap * sizeof(Type *));
            do {
                if (cnt == cap) { cap *= 2; arr = realloc(arr, cap * sizeof(Type *)); }
                arr[cnt++] = parse_type(p);
            } while (match_tok(p, TOK_COMMA));
            t->param_types = arr;
            t->param_count = cnt;
        }
        expect(p, TOK_RPAREN, "expected ')'");
        expect(p, TOK_ARROW, "expected '->'");
        t->return_type = parse_type(p);
        t->effects = parse_effects(p);
    } else {
        t = type_new(TY_PRIMITIVE);
        char buf[128];
        buf[0] = '\0';
        if (check(p, TOK_KW_REGION)) {
            snprintf(buf, sizeof(buf), "%s", "ptr");
            advance_parser(p);
            /* mark consumed; disable generic advance */
            t->name = strdup(buf);
            /* skip suffix advance already handled below via flag stored in buf? use a static var */
            goto type_primitive_done;
        } else if (check(p, TOK_KW_UNSIGNED) || check(p, TOK_KW_SIGNED)) {
            snprintf(buf, sizeof(buf), "%s", p->current.lexeme);
            advance_parser(p);
            if (is_type_start(p) && p->current.lexeme) {
                strncat(buf, " ", sizeof(buf)-strlen(buf)-1);
                strncat(buf, p->current.lexeme, sizeof(buf)-strlen(buf)-1);
            }
        } else if (p->current.lexeme) {
            snprintf(buf, sizeof(buf), "%s", p->current.lexeme);
        } else {
            snprintf(buf, sizeof(buf), "?");
        }
        t->name = strdup(buf);
        if (is_type_start(p)) advance_parser(p);
        else if (strcmp(buf, "?") == 0) parser_error(p, "expected type");
type_primitive_done: ;
    }
type_suffix:
    /* suffixes */
    while (1) {
        if (check(p, TOK_STAR)) {
            advance_parser(p);
            Type *ptr = type_new(TY_POINTER);
            ptr->base = t;
            ptr->name = t && t->name ? strdup(t->name) : strdup("?");
            t = ptr;
        } else if (check(p, TOK_LBRACKET)) {
            advance_parser(p);
            ASTNode *sz = NULL;
            if (!check(p, TOK_RBRACKET)) {
                sz = parse_expr(p);
            }
            expect(p, TOK_RBRACKET, "expected ']'");
            Type *arr = type_new(TY_ARRAY);
            arr->base = t;
            arr->array_size = sz;
            arr->name = t && t->name ? strdup(t->name) : strdup("?");
            t = arr;
        } else {
            break;
        }
    }
    return t;
}

static ASTNode *parse_expr(Parser *p);
static ASTNode *parse_assign(Parser *p);
static ASTNode *parse_ternary(Parser *p);
static ASTNode *parse_log_or(Parser *p);
static ASTNode *parse_log_and(Parser *p);
static ASTNode *parse_bit_or(Parser *p);
static ASTNode *parse_bit_xor(Parser *p);
static ASTNode *parse_bit_and(Parser *p);
static ASTNode *parse_eq(Parser *p);
static ASTNode *parse_rel(Parser *p);
static ASTNode *parse_shift(Parser *p);
static ASTNode *parse_add(Parser *p);
static ASTNode *parse_mul(Parser *p);
static ASTNode *parse_pow(Parser *p);
static ASTNode *parse_unary(Parser *p);
static ASTNode *parse_postfix(Parser *p);
static ASTNode *parse_primary(Parser *p);
static ASTNode *parse_stmt(Parser *p);
static ASTNode *parse_block(Parser *p);
static ASTNode *parse_var_decl(Parser *p, int require_semi);

static int is_vardecl_start(Parser *p) {
    if (is_type_keyword(p)) return 1;
    if (check(p, TOK_AT)) {
        Token t = lexer_peek(p->lex);
        int r = (t.type == TOK_IDENTIFIER && t.lexeme && strcmp(t.lexeme, "typeof") == 0);
        token_free(&t);
        return r;
    }
    if (check(p, TOK_IDENTIFIER)) {
        Token t = lexer_peek(p->lex);
        int r = (t.type == TOK_COLON || t.type == TOK_IDENTIFIER);
        token_free(&t);
        return r;
    }
    return 0;
}

static char *strdup_lex(const char *s) { return s ? strdup(s) : NULL; }

static ASTNode *parse_var_decl(Parser *p, int require_semi) {
    int line = p->current.line, col = p->current.column;
    ASTNode *decl = ast_new(AST_VAR_DECL, line, col);
    if (check(p, TOK_IDENTIFIER)) {
        Token t = lexer_peek(p->lex);
        if (t.type == TOK_COLON) {
            decl->name = strdup_lex(p->current.lexeme);
            advance_parser(p);
            expect(p, TOK_COLON, "expected ':'");
            decl->ty = parse_type(p);
            token_free(&t);
        } else if (t.type == TOK_IDENTIFIER) {
            Type *ty = type_new(TY_NAMED);
            ty->name = strdup_lex(p->current.lexeme);
            decl->ty = ty;
            advance_parser(p);
            decl->name = strdup_lex(p->current.lexeme);
            advance_parser(p);
            token_free(&t);
        } else {
            token_free(&t);
            if (is_type_start(p)) {
                decl->ty = parse_type(p);
                if (!check(p, TOK_IDENTIFIER)) { parser_error(p, "expected variable name"); }
                else { decl->name = strdup_lex(p->current.lexeme); advance_parser(p); }
            } else {
                parser_error(p, "expected variable declaration");
            }
        }
    } else if (is_type_start(p)) {
        decl->ty = parse_type(p);
        if (!check(p, TOK_IDENTIFIER)) { parser_error(p, "expected variable name"); }
        else { decl->name = strdup_lex(p->current.lexeme); advance_parser(p); }
    } else {
        parser_error(p, "expected type");
    }
    if (match_tok(p, TOK_ASSIGN)) {
        decl->init = parse_assign(p);
    }
    if (decl->name) sym_set(decl->name, decl->ty);
    if (require_semi) { expect(p, TOK_SEMICOLON, "expected ';'"); }
    return decl;
}

static ASTNode *parse_param(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *param = ast_new(AST_VAR_DECL, line, col);
    if (check(p, TOK_IDENTIFIER)) {
        Token t = lexer_peek(p->lex);
        if (t.type == TOK_COLON) {
            param->name = strdup_lex(p->current.lexeme);
            advance_parser(p);
            expect(p, TOK_COLON, "expected ':'");
            param->ty = parse_type(p);
            token_free(&t);
            return param;
        }
        token_free(&t);
    }
    if (is_type_start(p)) {
        param->ty = parse_type(p);
        if (check(p, TOK_IDENTIFIER)) { param->name = strdup_lex(p->current.lexeme); advance_parser(p); }
    } else if (check(p, TOK_IDENTIFIER)) {
        Token t = lexer_peek(p->lex);
        if (t.type == TOK_IDENTIFIER) {
            Type *ty = type_new(TY_NAMED);
            ty->name = strdup_lex(p->current.lexeme);
            param->ty = ty;
            advance_parser(p);
            param->name = strdup_lex(p->current.lexeme);
            advance_parser(p);
        }
        token_free(&t);
    }
    return param;
}

static ASTNode *parse_block(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *blk = ast_new(AST_BLOCK, line, col);
    expect(p, TOK_LBRACE, "expected '{'");
    while (!check(p, TOK_RBRACE) && !check(p, TOK_EOF)) {
        ASTNode *s = parse_stmt(p);
        if (s) ast_add_item(blk, s);
        if (p->had_error) break;
    }
    expect(p, TOK_RBRACE, "expected '}'");
    return blk;
}

static ASTNode *parse_if(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_IF, line, col);
    expect(p, TOK_KW_IF, "expected 'if'");
    expect(p, TOK_LPAREN, "expected '('");
    n->cond = parse_expr(p);
    expect(p, TOK_RPAREN, "expected ')'");
    n->then_branch = parse_block(p);
    if (match_tok(p, TOK_KW_ELSE)) {
        n->else_branch = parse_block(p);
    }
    return n;
}

static ASTNode *parse_while(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_WHILE, line, col);
    expect(p, TOK_KW_WHILE, "expected 'while'");
    expect(p, TOK_LPAREN, "expected '('");
    n->cond = parse_expr(p);
    expect(p, TOK_RPAREN, "expected ')'");
    n->body = parse_block(p);
    return n;
}

static ASTNode *parse_do_while(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_DO_WHILE, line, col);
    expect(p, TOK_KW_DO, "expected 'do'");
    n->body = parse_block(p);
    expect(p, TOK_KW_WHILE, "expected 'while'");
    expect(p, TOK_LPAREN, "expected '('");
    n->cond = parse_expr(p);
    expect(p, TOK_RPAREN, "expected ')'");
    expect(p, TOK_SEMICOLON, "expected ';'");
    return n;
}

static ASTNode *parse_for(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_FOR, line, col);
    expect(p, TOK_KW_FOR, "expected 'for'");
    expect(p, TOK_LPAREN, "expected '('");
    if (!check(p, TOK_SEMICOLON)) {
        if (is_vardecl_start(p)) {
            n->init_stmt = parse_var_decl(p, 0);
        } else {
            ASTNode *e = parse_expr(p);
            ASTNode *es = ast_new(AST_EXPR_STMT, e->line, e->column);
            es->left = e;
            n->init_stmt = es;
        }
    }
    expect(p, TOK_SEMICOLON, "expected ';'");
    if (!check(p, TOK_SEMICOLON)) {
        n->cond_expr = parse_expr(p);
    }
    expect(p, TOK_SEMICOLON, "expected ';'");
    if (!check(p, TOK_RPAREN)) {
        ASTNode *e = parse_expr(p);
        ASTNode *es = ast_new(AST_EXPR_STMT, e->line, e->column);
        es->left = e;
        n->update = es;
    }
    expect(p, TOK_RPAREN, "expected ')'");
    n->body = parse_block(p);
    return n;
}

static ASTNode *parse_switch(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_SWITCH, line, col);
    expect(p, TOK_KW_SWITCH, "expected 'switch'");
    expect(p, TOK_LPAREN, "expected '('");
    n->cond = parse_expr(p);
    expect(p, TOK_RPAREN, "expected ')'");
    expect(p, TOK_LBRACE, "expected '{'");
    /* parse cases */
    while (!check(p, TOK_RBRACE) && !check(p, TOK_EOF)) {
        if (check(p, TOK_KW_CASE)) {
            advance_parser(p);
            ASTNode *case_node = ast_new(AST_CASE, p->current.line, p->current.column);
            case_node->cond = parse_expr(p);
            expect(p, TOK_COLON, "expected ':'");
            case_node->body = parse_block(p);
            ast_add_item(n, case_node);
        } else if (check(p, TOK_KW_DEFAULT)) {
            advance_parser(p);
            ASTNode *def_node = ast_new(AST_DEFAULT, p->current.line, p->current.column);
            expect(p, TOK_COLON, "expected ':'");
            def_node->body = parse_block(p);
            ast_add_item(n, def_node);
        } else {
            parser_error(p, "expected 'case' or 'default'");
            break;
        }
    }
    expect(p, TOK_RBRACE, "expected '}'");
    return n;
}

static ASTNode *parse_return(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_RETURN, line, col);
    expect(p, TOK_KW_RETURN, "expected 'return'");
    if (!check(p, TOK_SEMICOLON) && !check(p, TOK_RBRACE)) {
        n->left = parse_expr(p);
    }
    expect(p, TOK_SEMICOLON, "expected ';'");
    return n;
}

static ASTNode *parse_asm(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_ASM, line, col);
    expect(p, TOK_KW_ASM, "expected 'asm'");
    if (check(p, TOK_LPAREN)) {
        advance_parser(p);
        if (check(p, TOK_STRING)) {
            n->str_value = strdup_lex(p->current.lexeme);
            advance_parser(p);
        }
        expect(p, TOK_RPAREN, "expected ')'");
    } else if (check(p, TOK_STRING)) {
        n->str_value = strdup_lex(p->current.lexeme);
        advance_parser(p);
    }
    expect(p, TOK_SEMICOLON, "expected ';'");
    return n;
}

static ASTNode *parse_stmt(Parser *p) {
    if (check(p, TOK_LBRACE)) return parse_block(p);
    if (check(p, TOK_SEMICOLON)) {
        /* empty statement: also lets brace-expanding macros be called with `;` */
        advance_parser(p);
        return NULL;
    }
    if (check(p, TOK_KW_IF)) return parse_if(p);
    if (check(p, TOK_KW_REGION)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        ASTNode *n = ast_new(AST_REGION, line, col);
        if (check(p, TOK_IDENTIFIER)) {
            n->name = strdup_lex(p->current.lexeme);
            advance_parser(p);
        } else {
            parser_error(p, "expected region name");
        }
        if (check(p, TOK_LPAREN)) {
            advance_parser(p);
            if (!check(p, TOK_RPAREN)) n->cond = parse_expr(p);
            expect(p, TOK_RPAREN, "expected ')'");
        }
        n->body = parse_block(p);
        return n;
    }
    if (check(p, TOK_KW_REQUIRES) || check(p, TOK_KW_ENSURES)) {
        int line = p->current.line, col = p->current.column;
        TokenType kw = p->current.type;
        advance_parser(p);
        ASTNode *n = ast_new(AST_CONTRACT, line, col);
        n->cond = parse_expr(p);
        n->op_token = kw;
        match_tok(p, TOK_SEMICOLON);
        return n;
    }
    if (check(p, TOK_KW_WHILE)) return parse_while(p);
    if (check(p, TOK_KW_FOR)) return parse_for(p);
    if (check(p, TOK_KW_DO)) return parse_do_while(p);
    if (check(p, TOK_KW_SWITCH)) return parse_switch(p);
    if (check(p, TOK_KW_RETURN)) return parse_return(p);
    if (check(p, TOK_KW_BREAK)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        expect(p, TOK_SEMICOLON, "expected ';'");
        return ast_new(AST_BREAK, line, col);
    }
    if (check(p, TOK_KW_CONTINUE)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        expect(p, TOK_SEMICOLON, "expected ';'");
        return ast_new(AST_CONTINUE, line, col);
    }
    if (check(p, TOK_KW_ASM)) return parse_asm(p);
    if (is_vardecl_start(p)) return parse_var_decl(p, 1);
    ASTNode *e = parse_expr(p);
    int line = e ? e->line : p->current.line;
    int col = e ? e->column : p->current.column;
    ASTNode *es = ast_new(AST_EXPR_STMT, line, col);
    es->left = e;
    expect(p, TOK_SEMICOLON, "expected ';'");
    return es;
}

/* Expressions */
static ASTNode *parse_expr(Parser *p) { return parse_assign(p); }

static int is_assign_op(TokenType t) {
    return t == TOK_ASSIGN || t == TOK_PLUSEQ || t == TOK_MINUSEQ ||
           t == TOK_STAREQ || t == TOK_SLASHEQ || t == TOK_PERCENTEQ ||
           t == TOK_AMPEQ || t == TOK_PIPEEQ || t == TOK_CARETEQ ||
           t == TOK_LSHIFTEQ || t == TOK_RSHIFTEQ;
}

static ASTNode *parse_assign(Parser *p) {
    ASTNode *left = parse_ternary(p);
    if (is_assign_op(p->current.type)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *right = parse_assign(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = left;
        b->op_token = op;
        b->right = right;
        return b;
    }
    return left;
}

static ASTNode *parse_ternary(Parser *p) {
    ASTNode *cond = parse_log_or(p);
    if (match_tok(p, TOK_QUESTION)) {
        ASTNode *n = ast_new(AST_EXPR_TERNARY, cond->line, cond->column);
        n->cond = cond;
        n->then_branch = parse_expr(p);
        expect(p, TOK_COLON, "expected ':'");
        n->else_branch = parse_ternary(p);
        return n;
    }
    return cond;
}

static ASTNode *parse_log_or(Parser *p) {
    ASTNode *node = parse_log_and(p);
    while (check(p, TOK_PIPEPIPE)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_log_and(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_log_and(Parser *p) {
    ASTNode *node = parse_bit_or(p);
    while (check(p, TOK_AMPAMP)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_bit_or(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_bit_or(Parser *p) {
    ASTNode *node = parse_bit_xor(p);
    while (check(p, TOK_PIPE) && !check(p, TOK_PIPEPIPE)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_bit_xor(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_bit_xor(Parser *p) {
    ASTNode *node = parse_bit_and(p);
    while (check(p, TOK_CARET)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_bit_and(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_bit_and(Parser *p) {
    ASTNode *node = parse_eq(p);
    while (check(p, TOK_AMP) && !check(p, TOK_AMPAMP)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_eq(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_eq(Parser *p) {
    ASTNode *node = parse_rel(p);
    while (check(p, TOK_EQEQ) || check(p, TOK_BANGEQ)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_rel(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_rel(Parser *p) {
    ASTNode *node = parse_shift(p);
    while (check(p, TOK_LT) || check(p, TOK_LTEQ) || check(p, TOK_GT) || check(p, TOK_GTEQ)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_shift(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_shift(Parser *p) {
    ASTNode *node = parse_add(p);
    while (check(p, TOK_LSHIFT) || check(p, TOK_RSHIFT)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_add(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_add(Parser *p) {
    ASTNode *node = parse_mul(p);
    while (check(p, TOK_PLUS) || check(p, TOK_MINUS)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_mul(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_mul(Parser *p) {
    ASTNode *node = parse_pow(p);
    while (check(p, TOK_STAR) || check(p, TOK_SLASH) || check(p, TOK_PERCENT)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *r = parse_pow(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = op; b->right = r;
        node = b;
    }
    return node;
}

static ASTNode *parse_pow(Parser *p) {
    ASTNode *node = parse_unary(p);
    if (check(p, TOK_STARSTAR)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        ASTNode *r = parse_pow(p);
        ASTNode *b = ast_new(AST_EXPR_BINARY, line, col);
        b->left = node; b->op_token = TOK_STARSTAR;
        b->right = r;
        return b;
    }
    return node;
}

static ASTNode *parse_unary(Parser *p) {
    if (check(p, TOK_BANG) || check(p, TOK_TILDE) ||
        check(p, TOK_PLUS) || check(p, TOK_MINUS) ||
        check(p, TOK_STAR) || check(p, TOK_AMP)) {
        int line = p->current.line, col = p->current.column;
        TokenType op = p->current.type;
        advance_parser(p);
        ASTNode *operand = parse_unary(p);
        ASTNode *u = ast_new(AST_EXPR_UNARY, line, col);
        u->op_token = op;
        u->left = operand;
        return u;
    }
    return parse_postfix(p);
}

static ASTNode *parse_postfix(Parser *p) {
    ASTNode *node = parse_primary(p);
    if (!node) return NULL;
    for (;;) {
        if (check(p, TOK_LPAREN)) {
            int line = p->current.line, col = p->current.column;
            advance_parser(p);
            ASTNode *call = ast_new(AST_EXPR_CALL, line, col);
            call->left = node;
            if (!check(p, TOK_RPAREN)) {
                do {
                    ASTNode *a = parse_assign(p);
                    if (!call->args) { call->args = malloc(sizeof(ASTNode*)); call->arg_cap = 1; }
                    else if (call->arg_count == call->arg_cap) {
                        call->arg_cap *= 2;
                        call->args = realloc(call->args, call->arg_cap * sizeof(ASTNode*));
                    }
                    call->args[call->arg_count++] = a;
                } while (match_tok(p, TOK_COMMA));
            }
            expect(p, TOK_RPAREN, "expected ')'");
            node = call;
        } else if (check(p, TOK_LBRACKET)) {
            int line = p->current.line, col = p->current.column;
            advance_parser(p);
            ASTNode *idx = parse_expr(p);
            expect(p, TOK_RBRACKET, "expected ']'");
            ASTNode *ix = ast_new(AST_EXPR_INDEX, line, col);
            ix->left = node;
            ix->right = idx;
            node = ix;
        } else if (check(p, TOK_DOT) || check(p, TOK_ARROW)) {
            int is_arrow = check(p, TOK_ARROW);
            (void)is_arrow;
            advance_parser(p);
            int line = p->current.line, col = p->current.column;
            ASTNode *f = ast_new(AST_EXPR_FIELD, line, col);
            f->left = node;
            if (check(p, TOK_IDENTIFIER)) {
                f->name = strdup_lex(p->current.lexeme);
                advance_parser(p);
            } else {
                parser_error(p, "expected field name");
            }
            node = f;
        } else if (check(p, TOK_PLUSPLUS) || check(p, TOK_MINUSMINUS)) {
            int line = p->current.line, col = p->current.column;
            TokenType op = p->current.type;
            advance_parser(p);
            ASTNode *u = ast_new(AST_EXPR_UNARY, line, col);
            u->op_token = op;
            u->left = node;
            node = u;
        } else {
            break;
        }
    }
    return node;
}

static ASTNode *parse_primary(Parser *p) {
    if (check(p, TOK_NUMBER)) {
        ASTNode *n = ast_new(AST_EXPR_NUMBER, p->current.line, p->current.column);
        n->str_value = strdup_lex(p->current.lexeme);
        advance_parser(p);
        return n;
    }
    if (check(p, TOK_STRING)) {
        ASTNode *n = ast_new(AST_EXPR_STRING, p->current.line, p->current.column);
        n->str_value = strdup_lex(p->current.lexeme);
        advance_parser(p);
        return n;
    }
    if (check(p, TOK_CHAR)) {
        ASTNode *n = ast_new(AST_EXPR_CHAR, p->current.line, p->current.column);
        n->str_value = strdup_lex(p->current.lexeme);
        advance_parser(p);
        return n;
    }
    if (check(p, TOK_KW_NULL)) {
        ASTNode *n = ast_new(AST_EXPR_NULL, p->current.line, p->current.column);
        advance_parser(p);
        return n;
    }
    if (check(p, TOK_IDENTIFIER) || check(p, TOK_KW_MAIN)) {
        ASTNode *n = ast_new(AST_EXPR_IDENT, p->current.line, p->current.column);
        n->name = strdup_lex(p->current.lexeme);
        advance_parser(p);
        return n;
    }
    if (check(p, TOK_KW_SIZEOF)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        expect(p, TOK_LPAREN, "expected '('");
        Type *t = parse_type(p);
        ASTNode *n = ast_new(AST_EXPR_SIZEOF, line, col);
        n->ty = type_clone(t);
        type_free(t);
        expect(p, TOK_RPAREN, "expected ')'");
        return n;
    }
    if (check(p, TOK_KW_ALIGNOF)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        expect(p, TOK_LPAREN, "expected '('");
        Type *t = parse_type(p);
        ASTNode *n = ast_new(AST_EXPR_ALIGNOF, line, col);
        n->ty = type_clone(t);
        type_free(t);
        return n;
    }
    if (check(p, TOK_LPAREN)) {
        advance_parser(p);
        ASTNode *e = parse_expr(p);
        expect(p, TOK_RPAREN, "expected ')'");
        return e;
    }
    if (check(p, TOK_AT)) {
        int line = p->current.line, col = p->current.column;
        advance_parser(p);
        if (check(p, TOK_KW_SIZEOF)) {
            advance_parser(p);
            expect(p, TOK_LPAREN, "expected '('");
            Type *t = parse_type(p);
            ASTNode *n = ast_new(AST_EXPR_SIZEOF, line, col);
            n->ty = type_clone(t);
            type_free(t);
            expect(p, TOK_RPAREN, "expected ')'");
            return n;
        }
        if (check(p, TOK_KW_ALIGNOF)) {
            advance_parser(p);
            expect(p, TOK_LPAREN, "expected '('");
            Type *t = parse_type(p);
            ASTNode *n = ast_new(AST_EXPR_ALIGNOF, line, col);
            n->ty = type_clone(t);
            type_free(t);
            expect(p, TOK_RPAREN, "expected ')'");
            return n;
        }
        if (check(p, TOK_IDENTIFIER)) {
            const char *id = p->current.lexeme;
            if (strcmp(id, "sizeof") == 0) {
                advance_parser(p); expect(p, TOK_LPAREN, "expected '('");
                Type *t = parse_type(p); ASTNode *n = ast_new(AST_EXPR_SIZEOF, line, col); n->ty = type_clone(t); type_free(t); expect(p, TOK_RPAREN, "expected ')'");
                return n;
            }
            if (strcmp(id, "alignof") == 0) {
                advance_parser(p); expect(p, TOK_LPAREN, "expected '('");
                Type *t = parse_type(p); ASTNode *n = ast_new(AST_EXPR_ALIGNOF, line, col); n->ty = type_clone(t); type_free(t); expect(p, TOK_RPAREN, "expected ')'");
                return n;
            }
            if (strcmp(id, "offsetof") == 0) {
                advance_parser(p); expect(p, TOK_LPAREN, "expected '('");
                Type *t = parse_type(p);
                match_tok(p, TOK_COMMA);
                ASTNode *n = ast_new(AST_EXPR_OFFSETOF, line, col);
                n->ty = type_clone(t);
                type_free(t);
                if (check(p, TOK_IDENTIFIER)) { n->name = strdup_lex(p->current.lexeme); advance_parser(p); }
                expect(p, TOK_RPAREN, "expected ')'");
                return n;
            }
            if (strcmp(id, "typeof") == 0) {
                advance_parser(p); expect(p, TOK_LPAREN, "expected '('");
                ASTNode *e = parse_expr(p);
                expect(p, TOK_RPAREN, "expected ')'");
                /* identity: reuse the expression (type inferred) */
                return e;
            }
            /* skip unknown */
            advance_parser(p);
            if (check(p, TOK_LPAREN)) { advance_parser(p); while (!check(p,TOK_RPAREN)&&!check(p,TOK_EOF)) advance_parser(p); match_tok(p,TOK_RPAREN); }
            return ast_new(AST_EXPR_NUMBER, line, col);
        }
    }
    parser_error(p, "expected expression");
    if (!check(p, TOK_EOF)) advance_parser(p);
    return NULL;
}

static ASTNode *parse_package(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_PACKAGE, line, col);
    expect(p, TOK_KW_PACKAGE, "expected 'package'");
    if (check(p, TOK_IDENTIFIER)) {
        n->name = strdup_lex(p->current.lexeme);
        advance_parser(p);
    }
    match_tok(p, TOK_SEMICOLON);
    return n;
}

static ASTNode *parse_import(Parser *p) {
    fprintf(stderr, "error: line %d: 'import' is no longer supported; use '@inc' for loi files/packages\n",
            p->current.line);
    exit(1);
}

static ASTNode *parse_struct(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_STRUCT_DECL, line, col);
    TokenType kw = p->current.type;
    advance_parser(p);
    if (check(p, TOK_IDENTIFIER)) {
        n->name = strdup_lex(p->current.lexeme);
        advance_parser(p);
    }
    /* struct-level attributes */
    while (check(p, TOK_AT)) {
        advance_parser(p);
        if (check(p, TOK_IDENTIFIER) && strcmp(p->current.lexeme, "align") == 0) {
            advance_parser(p);
            if (check(p, TOK_LPAREN)) {
                advance_parser(p);
                if (check(p, TOK_NUMBER)) { n->align = atoi(p->current.lexeme); advance_parser(p); }
                match_tok(p, TOK_RPAREN);
            }
        } else if (check(p, TOK_IDENTIFIER) && strcmp(p->current.lexeme, "pack") == 0) {
            advance_parser(p);
            match_tok(p, TOK_LPAREN); /* optional */
            match_tok(p, TOK_RPAREN);
            n->packed = 1;
        } else if (check(p, TOK_IDENTIFIER)) {
            advance_parser(p);
            if (check(p, TOK_LPAREN)) {
                advance_parser(p);
                while (!check(p, TOK_RPAREN) && !check(p, TOK_EOF)) advance_parser(p);
                match_tok(p, TOK_RPAREN);
            }
        }
    }
    expect(p, TOK_LBRACE, "expected '{'");
    while (!check(p, TOK_RBRACE) && !check(p, TOK_EOF)) {
        if (kw == TOK_KW_ENUM) {
            if (check(p, TOK_IDENTIFIER)) {
                ASTNode *id = ast_new(AST_EXPR_IDENT, p->current.line, p->current.column);
                id->name = strdup_lex(p->current.lexeme);
                advance_parser(p);
                ast_add_item(n, id);
                match_tok(p, TOK_COMMA);
            } else break;
        } else {
            int falign = 0;
            int fpacked = 0;
            while (check(p, TOK_AT)) {
                advance_parser(p);
                if (check(p, TOK_IDENTIFIER) && strcmp(p->current.lexeme, "align") == 0) {
                    advance_parser(p);
                    if (check(p, TOK_LPAREN)) {
                        advance_parser(p);
                        if (check(p, TOK_NUMBER)) { falign = atoi(p->current.lexeme); advance_parser(p); }
                        match_tok(p, TOK_RPAREN);
                    }
                } else if (check(p, TOK_IDENTIFIER) && strcmp(p->current.lexeme, "pack") == 0) {
                    advance_parser(p);
                    match_tok(p, TOK_LPAREN);
                    match_tok(p, TOK_RPAREN);
                    fpacked = 1;
                } else {
                    if (check(p, TOK_IDENTIFIER)) advance_parser(p);
                    if (check(p, TOK_LPAREN)) {
                        advance_parser(p);
                        while (!check(p, TOK_RPAREN) && !check(p, TOK_EOF)) advance_parser(p);
                        match_tok(p, TOK_RPAREN);
                    }
                }
            }
            if (!is_type_start(p)) break;
            ASTNode *f = parse_var_decl(p, 1);
            f->align = falign;
            f->packed = fpacked;
            ast_add_item(n, f);
        }
    }
    expect(p, TOK_RBRACE, "expected '}'");
    match_tok(p, TOK_SEMICOLON);
    return n;
}

static ASTNode *parse_type_decl(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_TYPE_DECL, line, col);
    expect(p, TOK_KW_TYPE, "expected 'type'");
    if (check(p, TOK_IDENTIFIER)) {
        n->name = strdup_lex(p->current.lexeme);
        advance_parser(p);
    }
    expect(p, TOK_ASSIGN, "expected '='");
    n->alias_type = parse_type(p);
    match_tok(p, TOK_SEMICOLON);
    return n;
}

static ASTNode *parse_extern(Parser *p) {
    int line = p->current.line, col = p->current.column;
    ASTNode *n = ast_new(AST_EXTERN_DECL, line, col);
    expect(p, TOK_KW_EXTERN, "expected 'extern'");
    if (check(p, TOK_KW_ASM)) {
        advance_parser(p);
        if (check(p, TOK_STRING)) {
            n->str_value = strdup_lex(p->current.lexeme);
            advance_parser(p);
        }
        match_tok(p, TOK_SEMICOLON);
        return n;
    }
    n->ty = parse_type(p);
    if (check(p, TOK_IDENTIFIER)) {
        n->name = strdup_lex(p->current.lexeme);
        advance_parser(p);
    }
    if (check(p, TOK_LPAREN)) {
        advance_parser(p);
        if (!check(p, TOK_RPAREN)) {
            do {
                ASTNode *param = parse_param(p);
                if (!n->args) { n->args = malloc(sizeof(ASTNode*)); n->arg_cap = 1; }
                else if (n->arg_count == n->arg_cap) {
                    n->arg_cap *= 2;
                    n->args = realloc(n->args, n->arg_cap * sizeof(ASTNode*));
                }
                n->args[n->arg_count++] = param;
            } while (match_tok(p, TOK_COMMA));
        }
        expect(p, TOK_RPAREN, "expected ')'");
    }
    match_tok(p, TOK_SEMICOLON);
    return n;
}

static ASTNode *parse_func(Parser *p) {
    int line = p->current.line, col = p->current.column;
    if (!(check(p, TOK_IDENTIFIER) || check(p, TOK_KW_MAIN))) {
        parser_error(p, "expected function name");
        return NULL;
    }
    ASTNode *n = ast_new(AST_FUNC_DECL, line, col);
    n->name = strdup_lex(p->current.lexeme);
    advance_parser(p);
    /* signature may appear before or after '=' */
    if (check(p, TOK_ASSIGN)) {
        advance_parser(p);
        if (check(p, TOK_LPAREN)) {
            advance_parser(p);
            if (!check(p, TOK_RPAREN)) {
                do {
                    ASTNode *param = parse_param(p);
                    if (!n->args) { n->args = malloc(sizeof(ASTNode*)); n->arg_cap = 1; }
                    else if (n->arg_count == n->arg_cap) {
                        n->arg_cap *= 2;
                        n->args = realloc(n->args, n->arg_cap * sizeof(ASTNode*));
                    }
                    n->args[n->arg_count++] = param;
                } while (match_tok(p, TOK_COMMA));
            }
            expect(p, TOK_RPAREN, "expected ')'");
            if (match_tok(p, TOK_COLON)) {
                n->ty = parse_type(p);
            } else if (match_tok(p, TOK_ARROW)) {
                n->ty = parse_type(p);
            }
            n->effects = parse_effects(p);
        }
    } else if (check(p, TOK_LPAREN)) {
        advance_parser(p);
        if (!check(p, TOK_RPAREN)) {
            do {
                ASTNode *param = parse_param(p);
                if (!n->args) { n->args = malloc(sizeof(ASTNode*)); n->arg_cap = 1; }
                else if (n->arg_count == n->arg_cap) {
                    n->arg_cap *= 2;
                    n->args = realloc(n->args, n->arg_cap * sizeof(ASTNode*));
                }
                n->args[n->arg_count++] = param;
            } while (match_tok(p, TOK_COMMA));
        }
        expect(p, TOK_RPAREN, "expected ')'");
        if (match_tok(p, TOK_COLON)) {
            n->ty = parse_type(p);
        } else if (match_tok(p, TOK_ARROW)) {
            n->ty = parse_type(p);
        }
        n->effects = parse_effects(p);
        expect(p, TOK_ASSIGN, "expected '='");
    } else {
        parser_error(p, "expected '=' or '('");
    }
    if (check(p, TOK_FAT_ARROW)) {
        advance_parser(p);
        ASTNode *expr = parse_expr(p);
        match_tok(p, TOK_SEMICOLON);
        ASTNode *block = ast_new(AST_BLOCK, n->line, n->column);
        ASTNode *ret = ast_new(AST_RETURN, expr ? expr->line : n->line, expr ? expr->column : n->column);
        ret->left = expr;
        ast_add_item(block, ret);
        n->body = block;
        return n;
    }
    ASTNode *b = parse_block(p);
    n->body = b;
    return n;
}

static ASTNode *parse_decl_core(Parser *p) {
    if (check(p, TOK_KW_PACKAGE)) return parse_package(p);
    /* `import` is not a keyword any more; catch the leftover identifier and
       point at the `@inc` replacement. */
    if (check(p, TOK_IDENTIFIER) && strcmp(p->current.lexeme, "import") == 0) return parse_import(p);
    if (check(p, TOK_KW_STRUCT) || check(p, TOK_KW_UNION) || check(p, TOK_KW_ENUM)) return parse_struct(p);
    if (check(p, TOK_KW_TYPE)) return parse_type_decl(p);
    if (check(p, TOK_KW_EXTERN)) return parse_extern(p);
    if (check(p, TOK_IDENTIFIER) || check(p, TOK_KW_MAIN)) {
        if (check(p, TOK_KW_MAIN)) {
            return parse_func(p);
        }
        Token t = lexer_peek(p->lex);
        if (t.type == TOK_ASSIGN || t.type == TOK_LPAREN) {
            token_free(&t);
            return parse_func(p);
        }
        token_free(&t);
    }
    if (is_vardecl_start(p)) return parse_var_decl(p, 1);
    parser_error(p, "expected top-level declaration");
    if (!check(p, TOK_EOF)) advance_parser(p);
    return NULL;
}

/* leading @section("name") attribute applied to the next declaration */
static ASTNode *parse_top_level(Parser *p) {
    char *sec = NULL;
    while (check(p, TOK_AT)) {
        Token t = lexer_peek(p->lex);
        int is_section = (t.type == TOK_IDENTIFIER && t.lexeme && strcmp(t.lexeme, "section") == 0);
        token_free(&t);
        if (!is_section) break;
        advance_parser(p); /* '@' */
        advance_parser(p); /* 'section' */
        expect(p, TOK_LPAREN, "expected '('");
        char buf[256]; buf[0] = 0;
        if (check(p, TOK_STRING)) {
            const char *lex = p->current.lexeme;
            size_t n = strlen(lex);
            /* string lexeme is stored without surrounding quotes, but strip if present */
            if (n >= 2 && lex[0] == '"' && lex[n - 1] == '"') { lex++; n -= 2; }
            if (n >= sizeof(buf)) n = sizeof(buf) - 1;
            memcpy(buf, lex, n); buf[n] = 0;
            advance_parser(p);
        }
        expect(p, TOK_RPAREN, "expected ')'");
        free(sec);
        sec = strdup(buf);
    }
    ASTNode *d = parse_decl_core(p);
    if (d && sec) { d->section = sec; sec = NULL; }
    free(sec);
    return d;
}

ASTNode *parse_program(Parser *p) {
    ASTNode *prog = ast_new(AST_PROGRAM, 1, 1);
    while (!check(p, TOK_EOF)) {
        ASTNode *d = parse_top_level(p);
        if (d) ast_add_item(prog, d);
        if (p->had_error) break;
    }
    return prog;
}

void parser_init(Parser *p, Lexer *lex) {
    p->lex = lex;
    p->had_error = 0;
    p->current = lexer_next(lex);
}
