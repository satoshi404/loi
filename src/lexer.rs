//! Lexer for the Loi language.
//!
//! The lexer turns source text into a flat token stream. It recognizes the
//! keywords and operators listed in `LOI_SPEC.md` section 7, reports the first
//! lexical error with its source position, and always ends with `TokenKind::Eof`.

use std::fmt;

use crate::token::{Token, TokenKind};

/// Raised when the source contains a character or literal Loi cannot lex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub filename: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.filename, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for LexError {}

type LexResult<T> = Result<T, LexError>;

/// A single-pass lexer over a string of Loi source code.
pub struct Lexer {
    source: Vec<char>,
    filename: String,
    offset: usize,
    line: usize,
    column: usize,
    tokens: Vec<Token>,
}

impl Lexer {
    pub fn new(source: &str, filename: impl Into<String>) -> Self {
        Self {
            source: source.chars().collect(),
            filename: filename.into(),
            offset: 0,
            line: 1,
            column: 1,
            tokens: Vec::new(),
        }
    }

    /// Return every token in the source, ending with an `Eof` token.
    pub fn tokenize(mut self) -> LexResult<Vec<Token>> {
        while let Some(ch) = self.peek() {
            let (line, column) = (self.line, self.column);

            if matches!(ch, ' ' | '\t' | '\r') {
                self.advance();
            } else if ch == '\n' {
                self.advance();
                self.emit_newline(line, column);
            } else if ch == '/' && self.peek_at(1) == Some('/') {
                self.skip_comment();
            } else if ch.is_alphabetic() || ch == '_' {
                self.scan_identifier(line, column);
            } else if ch.is_ascii_digit() {
                self.scan_number(line, column)?;
            } else if ch == '"' {
                self.scan_string(line, column)?;
            } else {
                self.scan_operator_or_punctuation(line, column)?;
            }
        }

        self.emit(TokenKind::Eof, String::new(), self.line, self.column);
        Ok(self.tokens)
    }

    // ------------------------------------------------------------------
    // Character helpers
    // ------------------------------------------------------------------
    fn peek(&self) -> Option<char> {
        self.source.get(self.offset).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.source.get(self.offset + ahead).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.offset += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    // ------------------------------------------------------------------
    // Token emitters
    // ------------------------------------------------------------------
    fn emit(&mut self, kind: TokenKind, lexeme: String, line: usize, column: usize) {
        self.tokens.push(Token::new(kind, lexeme, line, column));
    }

    fn emit_newline(&mut self, line: usize, column: usize) {
        // Consecutive blank lines carry no extra meaning for the parser.
        if self.tokens.last().map(|token| token.kind) != Some(TokenKind::Newline) {
            self.emit(TokenKind::Newline, "\n".to_string(), line, column);
        }
    }

    fn error(&self, message: impl Into<String>, line: usize, column: usize) -> LexError {
        LexError {
            message: message.into(),
            filename: self.filename.clone(),
            line,
            column,
        }
    }

    // ------------------------------------------------------------------
    // Scanners
    // ------------------------------------------------------------------
    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                break;
            }
            self.advance();
        }
    }

    fn scan_identifier(&mut self, line: usize, column: usize) {
        let start = self.offset;
        while let Some(ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                self.advance();
            } else {
                break;
            }
        }

        let lexeme: String = self.source[start..self.offset].iter().collect();
        let kind = TokenKind::from_keyword(&lexeme).unwrap_or(TokenKind::Identifier);
        self.emit(kind, lexeme, line, column);
    }

    fn scan_number(&mut self, line: usize, column: usize) -> LexResult<()> {
        let start = self.offset;

        // Detect radix prefix: 0x (hex) or 0b (binary). A lone '0' is decimal.
        let radix = if self.peek() == Some('0')
            && matches!(
                self.peek_at(1),
                Some('x') | Some('X') | Some('b') | Some('B')
            ) {
            self.advance(); // consume '0'
            self.advance(); // consume prefix letter
            match self.source[self.offset - 1].to_ascii_lowercase() {
                'x' => 16,
                'b' => 2,
                _ => unreachable!(),
            }
        } else {
            10
        };

        self.scan_digits_radix(radix);

        let mut is_float = false;

        // A single '.' followed by a digit is a decimal point. Two dots are
        // the range operator, so "0..count" must not consume the first dot.
        if radix == 10
            && self.peek() == Some('.')
            && self.peek_at(1).is_some_and(|ch| ch.is_ascii_digit())
        {
            is_float = true;
            self.advance();
            self.scan_digits_radix(10);
        }

        // Accept an optional exponent (1e3, 2.5e-4). The specification does
        // not define numeric syntax yet; this is a small conventional subset.
        if radix == 10 && matches!(self.peek(), Some('e') | Some('E')) {
            let mut lookahead = 1;
            if matches!(self.peek_at(1), Some('+') | Some('-')) {
                lookahead = 2;
            }
            if self
                .peek_at(lookahead)
                .is_some_and(|ch| ch.is_ascii_digit())
            {
                is_float = true;
                for _ in 0..lookahead {
                    self.advance();
                }
                self.scan_digits_radix(10);
            }
        }

        if let Some(ch) = self.peek()
            && (ch.is_alphabetic() || ch == '_')
        {
            let mut bad: String = self.source[start..self.offset].iter().collect();
            bad.push(ch);
            return Err(self.error(format!("invalid number literal '{bad}'"), line, column));
        }

        let lexeme: String = self.source[start..self.offset].iter().collect();
        let kind = if is_float {
            TokenKind::Float
        } else {
            TokenKind::Integer
        };
        self.emit(kind, lexeme, line, column);
        Ok(())
    }

    fn scan_digits_radix(&mut self, radix: u32) {
        while let Some(ch) = self.peek() {
            match radix {
                16 => {
                    if ch.is_ascii_digit() || matches!(ch, 'a'..='f' | 'A'..='F') || ch == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
                2 => {
                    if ch == '0' || ch == '1' || ch == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
                _ => {
                    if ch.is_ascii_digit() || ch == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Scan a double-quoted string literal and decode the common escapes.
    ///
    /// The token lexeme holds the decoded UTF-8 bytes (without quotes), so the
    /// parser and backend never need to interpret escapes again.
    fn scan_string(&mut self, line: usize, column: usize) -> LexResult<()> {
        self.advance(); // opening quote
        let mut value = String::new();

        loop {
            let Some(ch) = self.peek() else {
                return Err(self.error("unterminated string literal", line, column));
            };
            if ch == '"' {
                self.advance();
                self.emit(TokenKind::String, value, line, column);
                return Ok(());
            }
            if ch == '\n' {
                return Err(self.error("unterminated string literal", line, column));
            }
            if ch == '\\' {
                self.advance();
                let Some(escaped) = self.advance() else {
                    return Err(self.error("unterminated string escape", line, column));
                };
                match escaped {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '0' => value.push('\0'),
                    '\\' => value.push('\\'),
                    '"' => value.push('"'),
                    'x' => {
                        let high = self.peek().and_then(|c| c.to_digit(16));
                        let low = self.peek_at(1).and_then(|c| c.to_digit(16));
                        match (high, low) {
                            (Some(high), Some(low)) => {
                                self.advance();
                                self.advance();
                                let byte = (high * 16 + low) as u8;
                                if !byte.is_ascii() {
                                    return Err(self.error(
                                        "'\\x' escape must be an ASCII byte (0x00-0x7f)",
                                        line,
                                        column,
                                    ));
                                }
                                value.push(byte as char);
                            }
                            _ => {
                                return Err(self.error(
                                    "'\\x' escape requires two hexadecimal digits",
                                    line,
                                    column,
                                ));
                            }
                        }
                    }
                    other => {
                        return Err(self.error(
                            format!("unknown string escape '\\{other}'"),
                            line,
                            column,
                        ));
                    }
                }
                continue;
            }
            value.push(ch);
            self.advance();
        }
    }

    fn scan_operator_or_punctuation(&mut self, line: usize, column: usize) -> LexResult<()> {
        if let (Some(first), Some(second), Some(third)) =
            (self.peek(), self.peek_at(1), self.peek_at(2))
        {
            let triple = [first, second, third].iter().collect::<String>();
            if triple == "<->" {
                self.advance();
                self.advance();
                self.advance();
                self.emit(TokenKind::ArrowInOut, triple, line, column);
                return Ok(());
            }
        }

        if let (Some(first), Some(second)) = (self.peek(), self.peek_at(1)) {
            let pair = [first, second].iter().collect::<String>();
            let kind = match pair.as_str() {
                ".." => Some(TokenKind::DotDot),
                "->" => Some(TokenKind::Arrow),
                "<-" => Some(TokenKind::ArrowIn),
                "=>" => Some(TokenKind::FatArrow),
                "+=" => Some(TokenKind::PlusEq),
                "-=" => Some(TokenKind::MinusEq),
                "*=" => Some(TokenKind::StarEq),
                "/=" => Some(TokenKind::SlashEq),
                "==" => Some(TokenKind::EqEq),
                "!=" => Some(TokenKind::Neq),
                "<=" => Some(TokenKind::Lte),
                ">=" => Some(TokenKind::Gte),
                _ => None,
            };

            if let Some(kind) = kind {
                self.advance();
                self.advance();
                self.emit(kind, pair, line, column);
                return Ok(());
            }
        }

        let ch = self.peek().expect("called only when not at end of input");
        let kind = match ch {
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,
            '.' => TokenKind::Dot,
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '=' => TokenKind::Assign,
            '<' => TokenKind::Lt,
            '>' => TokenKind::Gt,
            _ => {
                return Err(self.error(format!("unexpected character '{ch}'"), line, column));
            }
        };

        self.advance();
        self.emit(kind, ch.to_string(), line, column);
        Ok(())
    }
}

/// Convenience wrapper around [`Lexer`].
pub fn tokenize(source: &str, filename: &str) -> LexResult<Vec<Token>> {
    Lexer::new(source, filename).tokenize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source, "<test>")
            .expect("source should lex")
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn longest_match_operators() {
        assert_eq!(
            kinds("a..b -> c += 1 -= 2 *= 3 /= 4 == 5 != 6 <= 7 >= 8 < 9 > 10 = 11"),
            vec![
                TokenKind::Identifier,
                TokenKind::DotDot,
                TokenKind::Identifier,
                TokenKind::Arrow,
                TokenKind::Identifier,
                TokenKind::PlusEq,
                TokenKind::Integer,
                TokenKind::MinusEq,
                TokenKind::Integer,
                TokenKind::StarEq,
                TokenKind::Integer,
                TokenKind::SlashEq,
                TokenKind::Integer,
                TokenKind::EqEq,
                TokenKind::Integer,
                TokenKind::Neq,
                TokenKind::Integer,
                TokenKind::Lte,
                TokenKind::Integer,
                TokenKind::Gte,
                TokenKind::Integer,
                TokenKind::Lt,
                TokenKind::Integer,
                TokenKind::Gt,
                TokenKind::Integer,
                TokenKind::Assign,
                TokenKind::Integer,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn flow_syntax_tokens() {
        assert_eq!(
            kinds("<- <-> -> => shape flow open sealed mut give clone"),
            vec![
                TokenKind::ArrowIn,
                TokenKind::ArrowInOut,
                TokenKind::Arrow,
                TokenKind::FatArrow,
                TokenKind::Shape,
                TokenKind::Flow,
                TokenKind::Open,
                TokenKind::Sealed,
                TokenKind::Mut,
                TokenKind::Give,
                TokenKind::Clone,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn numbers_and_ranges() {
        let tokens = tokenize("0..3600 0.1 -9.81 1_000 2.5e-4", "<test>").unwrap();
        let lexemes: Vec<&str> = tokens.iter().map(|token| token.lexeme.as_str()).collect();
        assert_eq!(
            lexemes,
            vec!["0", "..", "3600", "0.1", "-", "9.81", "1_000", "2.5e-4", ""]
        );
        assert_eq!(tokens[3].kind, TokenKind::Float);
        assert_eq!(tokens[7].kind, TokenKind::Float);
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            kinds("mut x: i32 // trailing comment\nx = 1"),
            vec![
                TokenKind::Mut,
                TokenKind::Identifier,
                TokenKind::Colon,
                TokenKind::Identifier,
                TokenKind::Newline,
                TokenKind::Identifier,
                TokenKind::Assign,
                TokenKind::Integer,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn consecutive_newlines_are_collapsed() {
        assert_eq!(
            kinds("a\n\n\nb"),
            vec![
                TokenKind::Identifier,
                TokenKind::Newline,
                TokenKind::Identifier,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn reports_source_positions() {
        let tokens = tokenize("mut x\n  x = 1", "<test>").unwrap();
        assert_eq!((tokens[0].line, tokens[0].column), (1, 1));
        assert_eq!((tokens[3].line, tokens[3].column), (2, 3));
    }

    #[test]
    fn rejects_unexpected_character() {
        let error = tokenize("mut x: i32\n@", "sample.loi").unwrap_err();
        assert_eq!((error.line, error.column), (2, 1));
        assert!(error.to_string().contains("sample.loi:2:1"));
    }

    #[test]
    fn rejects_invalid_number_literal() {
        assert!(tokenize("123abc", "<test>").is_err());
    }
}
