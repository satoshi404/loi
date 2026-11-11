//! Recursive-descent parser for Loi.
//!
//! The parser consumes the token stream produced by [`crate::lexer`] and
//! builds the AST from [`crate::ast`]. `Newline` tokens are separators between
//! statements, shape fields and call arguments.

use std::fmt;

use crate::ast::*;
use crate::token::{Token, TokenKind};

/// A syntax error with its source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl ParseError {
    fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

type ParseResult<T> = Result<T, ParseError>;

/// Parse a complete token stream into a [`Program`].
pub fn parse(tokens: Vec<Token>) -> ParseResult<Program> {
    Parser::new(tokens).parse_program()
}

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    pub fn new(mut tokens: Vec<Token>) -> Self {
        if tokens.last().map(|token| token.kind) != Some(TokenKind::Eof) {
            let (line, column) = tokens
                .last()
                .map(|token| (token.line, token.column + 1))
                .unwrap_or((1, 1));
            tokens.push(Token::new(TokenKind::Eof, String::new(), line, column));
        }

        Self {
            tokens,
            position: 0,
        }
    }

    pub fn parse_program(&mut self) -> ParseResult<Program> {
        let mut items = Vec::new();
        self.skip_newlines();

        while !self.at(TokenKind::Eof) {
            items.push(self.parse_item()?);
            self.skip_newlines();
        }

        Ok(Program { items })
    }

    // ------------------------------------------------------------------
    // Token helpers
    // ------------------------------------------------------------------
    fn current(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    fn peek_kind(&self, offset: usize) -> TokenKind {
        self.tokens
            .get(self.position + offset)
            .map(|token| token.kind)
            .unwrap_or(TokenKind::Eof)
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        if self.position + 1 < self.tokens.len() {
            self.position += 1;
        }
        token
    }

    fn matches(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, expected: &str) -> ParseResult<Token> {
        if self.at(kind) {
            Ok(self.advance())
        } else {
            Err(self.error_current(format!(
                "expected {expected}, found {}",
                self.describe_current()
            )))
        }
    }

    fn describe_current(&self) -> String {
        let token = self.current();
        if token.kind == TokenKind::Eof {
            "end of file".to_string()
        } else {
            format!("'{}'", token.lexeme)
        }
    }

    fn error_current(&self, message: impl Into<String>) -> ParseError {
        let token = self.current();
        ParseError::new(message, token.line, token.column)
    }

    fn skip_newlines(&mut self) {
        while self.matches(TokenKind::Newline) {}
    }

    fn span_here(&self) -> Span {
        Span::new(self.current().line, self.current().column)
    }

    // ------------------------------------------------------------------
    // Items and types
    // ------------------------------------------------------------------
    fn parse_item(&mut self) -> ParseResult<Item> {
        let span = self.span_here();

        if self.at(TokenKind::Import) {
            return self.parse_import(span);
        }

        let visibility = self.parse_visibility();

        if self.at(TokenKind::Shape) {
            Ok(Item::Interface(self.parse_interface(visibility, span)?))
        } else if self.at(TokenKind::Opaque) {
            Ok(Item::Opaque(self.parse_opaque(visibility, span)?))
        } else if self.at(TokenKind::Extern) {
            Ok(Item::Function(
                self.parse_extern_function(visibility, span)?,
            ))
        } else if self.at(TokenKind::Flow) {
            Ok(Item::Function(self.parse_function(visibility, span)?))
        } else if self.at(TokenKind::Mut)
            || (self.at(TokenKind::Identifier)
                && matches!(self.peek_kind(1), TokenKind::ArrowIn | TokenKind::Colon))
        {
            Ok(Item::Global(self.parse_flow_global(visibility, span)?))
        } else {
            Err(self.error_current(format!(
                "expected 'import', 'shape', 'opaque', 'extern flow', 'flow' or a global binding, found {}",
                self.describe_current()
            )))
        }
    }

    fn parse_import(&mut self, span: Span) -> ParseResult<Item> {
        self.expect(TokenKind::Import, "'import'")?;
        let path = self.expect(TokenKind::String, "an import path string")?;
        Ok(Item::Import(ImportDecl {
            span,
            path: path.lexeme,
        }))
    }

    /// Parse the Flow-Syntax global form: `name <- value`, `mut name: T <- value`.
    fn parse_flow_global(&mut self, visibility: Visibility, span: Span) -> ParseResult<GlobalDecl> {
        let mutable = self.matches(TokenKind::Mut);
        let name = self.parse_identifier("global name")?;

        let ty = if self.matches(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(TokenKind::ArrowIn, "'<-' (globals require an initializer)")?;
        let value = self.parse_expression()?;

        Ok(GlobalDecl {
            span,
            visibility,
            mutable,
            name,
            ty,
            value,
        })
    }

    fn parse_visibility(&mut self) -> Visibility {
        if self.matches(TokenKind::Open) {
            Visibility::Visible
        } else {
            // Sealed is the default (`LOI_SPEC.md` 5.3); `sealed` is accepted
            // explicitly but needs no state beyond the default.
            self.matches(TokenKind::Sealed);
            Visibility::Hidden
        }
    }

    fn parse_interface(
        &mut self,
        visibility: Visibility,
        span: Span,
    ) -> ParseResult<InterfaceDecl> {
        if !self.matches(TokenKind::Shape) {
            self.expect(TokenKind::Shape, "'shape'")?;
        }
        let name = self.parse_identifier("shape name")?;

        // Data bodies use brackets in Flow Syntax.
        let close = if self.matches(TokenKind::LBracket) {
            TokenKind::RBracket
        } else {
            return Err(self.error_current("expected '[' to open a shape body"));
        };
        self.skip_newlines();

        let mut fields = Vec::new();
        while !self.at(close) {
            if self.at(TokenKind::Eof) {
                return Err(self.error_current("unterminated shape body"));
            }

            let field_span = self.span_here();
            let field_name = self.parse_identifier("field name")?;
            self.expect(TokenKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            let default = if self.matches(TokenKind::Assign) {
                Some(self.parse_expression()?)
            } else {
                None
            };

            fields.push(FieldDecl {
                span: field_span,
                name: field_name,
                ty,
                default,
            });

            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
            }
        }

        let close_token = self.expect(close, "the shape body terminator")?;
        Ok(InterfaceDecl {
            span,
            close_line: close_token.line,
            visibility,
            name,
            fields,
        })
    }

    /// `opaque Name` declares a foreign handle type.
    fn parse_opaque(&mut self, visibility: Visibility, span: Span) -> ParseResult<OpaqueDecl> {
        self.expect(TokenKind::Opaque, "'opaque'")?;
        let name = self.parse_identifier("opaque type name")?;
        Ok(OpaqueDecl {
            span,
            visibility,
            name,
        })
    }

    fn parse_function(&mut self, visibility: Visibility, span: Span) -> ParseResult<FunctionDecl> {
        if !self.matches(TokenKind::Flow) {
            self.expect(TokenKind::Flow, "'flow'")?;
        }
        let name = self.parse_identifier("function name")?;
        self.skip_newlines();

        let (params, return_type) = self.parse_signature(false)?;

        self.skip_newlines();
        let body = self.parse_block()?;

        Ok(FunctionDecl {
            span,
            visibility,
            name,
            params,
            return_type,
            body,
            external: false,
            link_name: None,
        })
    }

    /// `extern flow name [as c_symbol]` declares a C function. It has a
    /// signature but no Loi body.
    fn parse_extern_function(
        &mut self,
        visibility: Visibility,
        span: Span,
    ) -> ParseResult<FunctionDecl> {
        self.expect(TokenKind::Extern, "'extern'")?;
        self.expect(TokenKind::Flow, "'flow' after 'extern'")?;
        let name = self.parse_identifier("extern function name")?;
        let link_name = if self.matches(TokenKind::As) {
            Some(self.parse_identifier("C symbol name")?)
        } else {
            None
        };
        self.skip_newlines();

        let (params, return_type) = self.parse_signature(true)?;

        self.skip_newlines();
        if self.at(TokenKind::LBrace) {
            return Err(self.error_current("an extern flow declares no body"));
        }

        Ok(FunctionDecl {
            span,
            visibility,
            name,
            params,
            return_type,
            body: Block {
                statements: Vec::new(),
                open_line: span.line,
                close_line: span.line,
            },
            external: true,
            link_name,
        })
    }

    fn parse_signature(&mut self, external: bool) -> ParseResult<(Vec<Param>, Option<Type>)> {
        if self.at(TokenKind::LParen) {
            self.parse_inline_signature()
        } else {
            self.parse_flow_signature(external)
        }
    }

    /// Inline signature: `flow f(<- x: T) => R`.
    fn parse_inline_signature(&mut self) -> ParseResult<(Vec<Param>, Option<Type>)> {
        self.expect(TokenKind::LParen, "'('")?;
        self.skip_newlines();

        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                params.push(self.parse_param()?);
                self.skip_newlines();

                if self.matches(TokenKind::Comma) {
                    self.skip_newlines();
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    continue;
                }
                if self.at(TokenKind::RParen) {
                    break;
                }

                return Err(self.error_current(format!(
                    "expected ',' or ')' in parameter list, found {}",
                    self.describe_current()
                )));
            }
        }

        self.expect(TokenKind::RParen, "')'")?;
        self.skip_newlines();
        let return_type = if self.matches(TokenKind::FatArrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        Ok((params, return_type))
    }

    /// Flow-Syntax signature clauses:
    ///
    /// ```loi
    /// flow add
    ///     <- a: i32
    ///     <- b: i32
    ///     => i32
    /// { ... }
    /// ```
    fn parse_flow_signature(&mut self, external: bool) -> ParseResult<(Vec<Param>, Option<Type>)> {
        let mut params = Vec::new();
        let mut return_type = None;

        loop {
            self.skip_newlines();
            match self.current().kind {
                TokenKind::LBrace => break,
                TokenKind::ArrowIn | TokenKind::Arrow | TokenKind::ArrowInOut => {
                    if return_type.is_some() {
                        return Err(self.error_current(
                            "parameters must appear before the '=>' result clause",
                        ));
                    }
                    params.push(self.parse_param()?);
                }
                TokenKind::FatArrow => {
                    self.advance();
                    return_type = Some(self.parse_type()?);
                }
                _ if external => break,
                _ => {
                    return Err(self.error_current(format!(
                        "expected a parameter clause, '=> Type' or '{{', found {}",
                        self.describe_current()
                    )));
                }
            }
        }

        Ok((params, return_type))
    }

    fn parse_param(&mut self) -> ParseResult<Param> {
        let span = self.span_here();
        let direction = match self.current().kind {
            TokenKind::ArrowIn => {
                self.advance();
                Direction::In
            }
            TokenKind::Arrow => {
                self.advance();
                Direction::Out
            }
            TokenKind::ArrowInOut => {
                self.advance();
                Direction::Inout
            }
            _ => {
                return Err(self.error_current(format!(
                    "expected parameter direction '<-', '->' or '<->', found {}",
                    self.describe_current()
                )));
            }
        };

        let name = self.parse_identifier("parameter name")?;
        self.expect(TokenKind::Colon, "':'")?;
        let ty = self.parse_type()?;

        Ok(Param {
            span,
            direction,
            name,
            ty,
        })
    }

    fn parse_type(&mut self) -> ParseResult<Type> {
        if self.matches(TokenKind::LBracket) {
            let element = self.parse_type()?;
            self.expect(TokenKind::RBracket, "']'")?;
            Ok(Type::Slice(Box::new(element)))
        } else {
            let token = self.expect(TokenKind::Identifier, "type name")?;
            Ok(Type::Named(token.lexeme))
        }
    }

    fn parse_identifier(&mut self, expected: &str) -> ParseResult<String> {
        Ok(self.expect(TokenKind::Identifier, expected)?.lexeme)
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------
    fn parse_block(&mut self) -> ParseResult<Block> {
        let open = self.expect(TokenKind::LBrace, "'{'")?;
        self.skip_newlines();

        let mut statements = Vec::new();
        while !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Eof) {
                return Err(self.error_current("unterminated block"));
            }

            statements.push(self.parse_statement()?);

            if !self.at(TokenKind::RBrace)
                && !self.at(TokenKind::Newline)
                && !self.at(TokenKind::Eof)
            {
                return Err(self.error_current(format!(
                    "expected newline or '}}' after statement, found {}",
                    self.describe_current()
                )));
            }
            self.skip_newlines();
        }

        let close = self.expect(TokenKind::RBrace, "'}'")?;
        Ok(Block {
            statements,
            open_line: open.line,
            close_line: close.line,
        })
    }

    fn parse_statement(&mut self) -> ParseResult<Stmt> {
        let span = self.span_here();
        match self.current().kind {
            TokenKind::Mut => self.parse_flow_binding(span),
            TokenKind::Identifier
                if matches!(self.peek_kind(1), TokenKind::ArrowIn | TokenKind::Colon) =>
            {
                self.parse_flow_binding(span)
            }
            TokenKind::Protect => {
                self.advance();
                self.skip_newlines();
                Ok(Stmt::new(StmtKind::Protect(self.parse_block()?), span))
            }
            TokenKind::If => self.parse_if(span),
            TokenKind::While => self.parse_while(span),
            TokenKind::For => self.parse_for(span),
            TokenKind::FatArrow => {
                self.advance();
                if self.at(TokenKind::Newline)
                    || self.at(TokenKind::RBrace)
                    || self.at(TokenKind::Eof)
                {
                    Ok(Stmt::new(StmtKind::Return(None), span))
                } else {
                    Ok(Stmt::new(
                        StmtKind::Return(Some(self.parse_expression()?)),
                        span,
                    ))
                }
            }
            TokenKind::Continue => {
                self.advance();
                Ok(Stmt::new(StmtKind::Continue, span))
            }
            TokenKind::Break => {
                self.advance();
                Ok(Stmt::new(StmtKind::Break, span))
            }
            _ => self.parse_expression_or_assignment(span),
        }
    }

    /// Flow-Syntax binding: `x <- v`, `mut x: T <- v`, or `mut x: T`.
    fn parse_flow_binding(&mut self, span: Span) -> ParseResult<Stmt> {
        let mutable = self.matches(TokenKind::Mut);
        let name = self.parse_identifier("variable name")?;

        let ty = if self.matches(TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let value = if self.matches(TokenKind::ArrowIn) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        if ty.is_none() && value.is_none() {
            return Err(
                self.error_current("a binding needs a type or an initializer (`name <- value`)")
            );
        }

        Ok(Stmt::new(
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            },
            span,
        ))
    }

    fn parse_if(&mut self, span: Span) -> ParseResult<Stmt> {
        self.expect(TokenKind::If, "'if'")?;
        let condition = self.parse_expression()?;
        self.skip_newlines();
        let then_block = self.parse_block()?;

        // Only consume the newline before `else`; otherwise leave it as the
        // separator for the next statement.
        let saved = self.position;
        self.skip_newlines();
        let else_branch = if self.matches(TokenKind::Else) {
            self.skip_newlines();
            if self.at(TokenKind::If) {
                let inner_span = self.span_here();
                Some(ElseBranch::If(Box::new(self.parse_if(inner_span)?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            self.position = saved;
            None
        };

        Ok(Stmt::new(
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            },
            span,
        ))
    }

    fn parse_while(&mut self, span: Span) -> ParseResult<Stmt> {
        self.expect(TokenKind::While, "'while'")?;
        let condition = self.parse_expression()?;
        self.skip_newlines();
        let body = self.parse_block()?;
        Ok(Stmt::new(StmtKind::While { condition, body }, span))
    }

    fn parse_for(&mut self, span: Span) -> ParseResult<Stmt> {
        self.expect(TokenKind::For, "'for'")?;

        if matches!(
            self.current().kind,
            TokenKind::ArrowIn | TokenKind::ArrowInOut
        ) {
            let direction = if self.matches(TokenKind::ArrowIn) {
                Direction::In
            } else {
                self.expect(TokenKind::ArrowInOut, "'<->'")?;
                Direction::Inout
            };
            let variable = self.parse_identifier("loop variable")?;
            self.expect(TokenKind::In, "'in'")?;
            let iterable = self.parse_expression()?;
            self.skip_newlines();
            let body = self.parse_block()?;

            Ok(Stmt::new(
                StmtKind::ForEach {
                    direction,
                    variable,
                    iterable,
                    body,
                },
                span,
            ))
        } else {
            let variable = self.parse_identifier("loop variable")?;
            self.expect(TokenKind::In, "'in'")?;
            let start = self.parse_expression()?;
            self.expect(TokenKind::DotDot, "'..'")?;
            let end = self.parse_expression()?;
            self.skip_newlines();
            let body = self.parse_block()?;

            Ok(Stmt::new(
                StmtKind::ForRange {
                    variable,
                    start,
                    end,
                    body,
                },
                span,
            ))
        }
    }

    fn parse_expression_or_assignment(&mut self, span: Span) -> ParseResult<Stmt> {
        let target = self.parse_expression()?;

        let op = match self.current().kind {
            TokenKind::Assign => Some(AssignOp::Assign),
            TokenKind::PlusEq => Some(AssignOp::Add),
            TokenKind::MinusEq => Some(AssignOp::Sub),
            TokenKind::StarEq => Some(AssignOp::Mul),
            TokenKind::SlashEq => Some(AssignOp::Div),
            TokenKind::PercentEq => Some(AssignOp::Mod),
            TokenKind::AmpEq => Some(AssignOp::BitAnd),
            TokenKind::PipeEq => Some(AssignOp::BitOr),
            TokenKind::CaretEq => Some(AssignOp::BitXor),
            TokenKind::ShlEq => Some(AssignOp::Shl),
            TokenKind::ShrEq => Some(AssignOp::Shr),
            _ => None,
        };

        if let Some(op) = op {
            self.advance();
            let value = self.parse_expression()?;
            Ok(Stmt::new(StmtKind::Assign { target, op, value }, span))
        } else {
            Ok(Stmt::new(StmtKind::Expr(target), span))
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------
    fn parse_expression(&mut self) -> ParseResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_and()?;
        while self.matches(TokenKind::Or) {
            let span = expr.span;
            let right = self.parse_and()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op: BinaryOp::Or,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_comparison()?;
        while self.matches(TokenKind::And) {
            let span = expr.span;
            let right = self.parse_comparison()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op: BinaryOp::And,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_bitor()?;
        loop {
            let op = match self.current().kind {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::Neq => BinaryOp::Neq,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Lte => BinaryOp::Lte,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Gte => BinaryOp::Gte,
                _ => break,
            };
            self.advance();
            let span = expr.span;
            let right = self.parse_bitor()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_bitor(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_bitxor()?;
        while self.matches(TokenKind::Pipe) {
            let span = expr.span;
            let right = self.parse_bitxor()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op: BinaryOp::BitOr,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_bitxor(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_bitand()?;
        while self.matches(TokenKind::Caret) {
            let span = expr.span;
            let right = self.parse_bitand()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op: BinaryOp::BitXor,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_bitand(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_shift()?;
        while self.matches(TokenKind::Amp) {
            let span = expr.span;
            let right = self.parse_shift()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op: BinaryOp::BitAnd,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_shift(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_additive()?;
        loop {
            let op = match self.current().kind {
                TokenKind::Shl => BinaryOp::Shl,
                TokenKind::Shr => BinaryOp::Shr,
                _ => break,
            };
            self.advance();
            let span = expr.span;
            let right = self.parse_additive()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_additive(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_multiplicative()?;
        loop {
            let op = match self.current().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let span = expr.span;
            let right = self.parse_multiplicative()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_multiplicative(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_cast()?;
        loop {
            let op = match self.current().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            self.advance();
            let span = expr.span;
            let right = self.parse_cast()?;
            expr = Expr::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(expr),
                    right: Box::new(right),
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_cast(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_unary()?;
        while self.matches(TokenKind::As) {
            let span = expr.span;
            let ty = self.parse_type()?;
            expr = Expr::new(
                ExprKind::Cast {
                    expr: Box::new(expr),
                    ty,
                },
                span,
            );
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> ParseResult<Expr> {
        if self.at(TokenKind::Minus) {
            let span = self.span_here();
            self.advance();
            Ok(Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(self.parse_unary()?),
                },
                span,
            ))
        } else if self.at(TokenKind::Not) {
            let span = self.span_here();
            self.advance();
            Ok(Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(self.parse_unary()?),
                },
                span,
            ))
        } else if self.at(TokenKind::Tilde) {
            let span = self.span_here();
            self.advance();
            Ok(Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::BitNot,
                    operand: Box::new(self.parse_unary()?),
                },
                span,
            ))
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> ParseResult<Expr> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.at(TokenKind::LParen) {
                let span = expr.span;
                let args = self.parse_call_args()?;
                expr = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(expr),
                        args,
                    },
                    span,
                );
            } else if self.matches(TokenKind::Dot) {
                let span = expr.span;
                let field = self.parse_identifier("field name")?;
                expr = Expr::new(
                    ExprKind::Member {
                        base: Box::new(expr),
                        field,
                    },
                    span,
                );
            } else if self.matches(TokenKind::LBracket) {
                let span = expr.span;
                let index = self.parse_expression()?;
                self.expect(TokenKind::RBracket, "']'")?;
                expr = Expr::new(
                    ExprKind::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                    },
                    span,
                );
            } else {
                break;
            }
        }

        Ok(expr)
    }

    fn parse_primary(&mut self) -> ParseResult<Expr> {
        let token = self.current().clone();
        let span = Span::new(token.line, token.column);

        match token.kind {
            TokenKind::Integer => {
                self.advance();
                Ok(Expr::new(ExprKind::Int(token.lexeme), span))
            }
            TokenKind::Float => {
                self.advance();
                Ok(Expr::new(ExprKind::Float(token.lexeme), span))
            }
            TokenKind::String => {
                self.advance();
                Ok(Expr::new(ExprKind::String(token.lexeme), span))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::new(ExprKind::Bool(true), span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::new(ExprKind::Bool(false), span))
            }
            TokenKind::Identifier => {
                self.advance();
                let bracket_literal =
                    self.at(TokenKind::LBracket) && self.bracket_looks_like_shape_literal();
                if bracket_literal {
                    self.parse_interface_literal(token.lexeme, span)
                } else {
                    Ok(Expr::new(ExprKind::Ident(token.lexeme), span))
                }
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(TokenKind::RParen, "')'")?;
                Ok(expr)
            }
            TokenKind::Give => {
                self.advance();
                self.expect(TokenKind::LParen, "'('")?;
                let value = self.parse_expression()?;
                self.expect(TokenKind::RParen, "')'")?;
                Ok(Expr::new(ExprKind::Move(Box::new(value)), span))
            }
            TokenKind::Clone => {
                self.advance();
                self.expect(TokenKind::LParen, "'('")?;
                let value = self.parse_expression()?;
                self.expect(TokenKind::RParen, "')'")?;
                Ok(Expr::new(ExprKind::Copy(Box::new(value)), span))
            }
            TokenKind::Alloc => self.parse_alloc(span),
            TokenKind::LBracket => self.parse_slice_literal(span),
            _ => Err(self.error_current(format!(
                "expected expression, found {}",
                self.describe_current()
            ))),
        }
    }

    /// Does the `[` at the current position start a Flow-Syntax shape literal
    /// (`Name[ field: value ]`) rather than an index expression?
    fn bracket_looks_like_shape_literal(&self) -> bool {
        if !self.at(TokenKind::LBracket) {
            return false;
        }
        if self.peek_kind(1) == TokenKind::RBracket {
            return true; // `Name[]` is an empty shape literal
        }

        let mut depth = 0usize;
        let mut index = self.position;
        while let Some(token) = self.tokens.get(index) {
            match token.kind {
                TokenKind::LBracket | TokenKind::LBrace | TokenKind::LParen => depth += 1,
                TokenKind::RBracket | TokenKind::RBrace | TokenKind::RParen => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                TokenKind::Colon if depth == 1 => return true,
                TokenKind::Comma if depth == 1 => return false,
                TokenKind::Eof => return false,
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// Parse `[a, b]` or the explicit `[T: a, b]` / `[T:]` form.
    fn parse_slice_literal(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::LBracket, "'['")?;

        // Explicit form `[T: a, b]`. Try a type followed by ':' and backtrack
        // if the brackets actually hold an inferred element list.
        let saved = self.position;
        let typed = match self.parse_type() {
            Ok(element_type) if self.matches(TokenKind::Colon) => Some(element_type),
            _ => None,
        };
        if let Some(element_type) = typed {
            let elements = self.parse_element_list(TokenKind::RBracket, "slice literal")?;
            return Ok(Expr::new(
                ExprKind::SliceLiteral {
                    element_type: Some(element_type),
                    elements,
                },
                span,
            ));
        }
        self.position = saved;

        let elements = self.parse_element_list(TokenKind::RBracket, "slice literal")?;
        Ok(Expr::new(
            ExprKind::SliceLiteral {
                element_type: None,
                elements,
            },
            span,
        ))
    }

    /// Parse a comma/newline-separated element list up to `close`.
    fn parse_element_list(&mut self, close: TokenKind, what: &str) -> ParseResult<Vec<Expr>> {
        self.skip_newlines();
        let mut elements = Vec::new();

        while !self.at(close) {
            if self.at(TokenKind::Eof) {
                return Err(self.error_current(format!("unterminated {what}")));
            }

            elements.push(self.parse_expression()?);
            let had_newline = self.at(TokenKind::Newline);
            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
            } else if !self.at(close) && !had_newline {
                return Err(self.error_current(format!(
                    "expected ',' or the closing delimiter in {what}, found {}",
                    self.describe_current()
                )));
            }
        }

        self.expect(close, "the closing delimiter")?;
        Ok(elements)
    }

    fn parse_interface_literal(&mut self, name: String, span: Span) -> ParseResult<Expr> {
        // Data literals use brackets in Flow Syntax.
        let close = if self.matches(TokenKind::LBracket) {
            TokenKind::RBracket
        } else {
            return Err(self.error_current("expected '[' to open a shape literal"));
        };
        self.skip_newlines();

        let mut fields = Vec::new();
        while !self.at(close) {
            if self.at(TokenKind::Eof) {
                return Err(self.error_current("unterminated shape literal"));
            }

            let field_name = self.parse_identifier("field name")?;
            self.expect(TokenKind::Colon, "':'")?;
            let value = self.parse_expression()?;
            fields.push(FieldInit {
                name: field_name,
                value,
            });

            self.skip_newlines();
            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
            }
        }

        self.expect(close, "the shape literal terminator")?;
        Ok(Expr::new(ExprKind::InterfaceLiteral { name, fields }, span))
    }

    fn parse_call_args(&mut self) -> ParseResult<Vec<CallArg>> {
        self.expect(TokenKind::LParen, "'('")?;
        self.parse_call_args_inner()
    }

    fn parse_call_args_inner(&mut self) -> ParseResult<Vec<CallArg>> {
        self.skip_newlines();
        let mut args = Vec::new();

        if self.matches(TokenKind::RParen) {
            return Ok(args);
        }

        loop {
            let direction = match self.current().kind {
                TokenKind::ArrowIn => {
                    self.advance();
                    Direction::In
                }
                TokenKind::Arrow => {
                    self.advance();
                    Direction::Out
                }
                TokenKind::ArrowInOut => {
                    self.advance();
                    Direction::Inout
                }
                _ => Direction::In,
            };

            let value = self.parse_expression()?;
            args.push(CallArg { direction, value });

            let had_newline = self.at(TokenKind::Newline);
            self.skip_newlines();

            if self.matches(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RParen) {
                    break;
                }
                continue;
            }

            if self.at(TokenKind::RParen) {
                break;
            }

            if had_newline {
                continue;
            }

            return Err(self.error_current(format!(
                "expected ',' or ')' in argument list, found {}",
                self.describe_current()
            )));
        }

        self.expect(TokenKind::RParen, "')'")?;
        Ok(args)
    }

    fn parse_alloc(&mut self, span: Span) -> ParseResult<Expr> {
        self.expect(TokenKind::Alloc, "'alloc'")?;
        self.expect(TokenKind::LParen, "'('")?;
        self.expect(TokenKind::Arrow, "'->'")?;
        let target = self.parse_expression()?;
        self.expect(TokenKind::Comma, "','")?;
        let element_type = self.parse_type()?;
        self.expect(TokenKind::Comma, "','")?;
        let count = self.parse_expression()?;
        self.expect(TokenKind::RParen, "')'")?;

        Ok(Expr::new(
            ExprKind::Alloc {
                target: Box::new(target),
                element_type,
                count: Box::new(count),
            },
            span,
        ))
    }
}
