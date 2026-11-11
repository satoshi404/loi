//! Token definitions for the Loi language.

/// Every token kind recognized by the Loi lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Keywords
    Import,
    Shape,
    Flow,
    Extern,
    Opaque,
    Open,
    Sealed,
    Mut,
    In,
    Give,
    Clone,
    Protect,
    Alloc,
    Comptime,
    If,
    Else,
    While,
    For,
    Continue,
    Break,
    And,
    Or,
    Not,
    As,
    True,
    False,

    // Identifiers and literals
    Identifier,
    Integer,
    Float,
    String,

    // Delimiters
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    DotDot,
    Arrow,
    ArrowIn,
    ArrowInOut,
    FatArrow,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Assign,
    EqEq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,

    // Layout and end of input
    Newline,
    Eof,
}

impl TokenKind {
    /// The keyword kind for `text`, if `text` is a Loi keyword.
    pub fn from_keyword(text: &str) -> Option<Self> {
        let kind = match text {
            "import" => Self::Import,
            "shape" => Self::Shape,
            "flow" => Self::Flow,
            "extern" => Self::Extern,
            "opaque" => Self::Opaque,
            "open" => Self::Open,
            "sealed" => Self::Sealed,
            "mut" => Self::Mut,
            "in" => Self::In,
            "give" => Self::Give,
            "clone" => Self::Clone,
            "protect" => Self::Protect,
            "alloc" => Self::Alloc,
            "comptime" => Self::Comptime,
            "if" => Self::If,
            "else" => Self::Else,
            "while" => Self::While,
            "for" => Self::For,
            "continue" => Self::Continue,
            "break" => Self::Break,
            "and" => Self::And,
            "or" => Self::Or,
            "not" => Self::Not,
            "as" => Self::As,
            "true" => Self::True,
            "false" => Self::False,
            _ => return None,
        };
        Some(kind)
    }

    /// A name suitable for diagnostics and CLI output.
    pub fn name(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Shape => "shape",
            Self::Flow => "flow",
            Self::Extern => "extern",
            Self::Opaque => "opaque",
            Self::Open => "open",
            Self::Sealed => "sealed",
            Self::Mut => "mut",
            Self::In => "in",
            Self::Give => "give",
            Self::Clone => "clone",
            Self::Protect => "protect",
            Self::Alloc => "alloc",
            Self::Comptime => "comptime",
            Self::If => "if",
            Self::Else => "else",
            Self::While => "while",
            Self::For => "for",
            Self::Continue => "continue",
            Self::Break => "break",
            Self::And => "and",
            Self::Or => "or",
            Self::Not => "not",
            Self::As => "as",
            Self::True => "true",
            Self::False => "false",
            Self::Identifier => "identifier",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::String => "string",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBracket => "[",
            Self::RBracket => "]",
            Self::Comma => ",",
            Self::Colon => ":",
            Self::Dot => ".",
            Self::DotDot => "..",
            Self::Arrow => "->",
            Self::ArrowIn => "<-",
            Self::ArrowInOut => "<->",
            Self::FatArrow => "=>",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Percent => "%",
            Self::Amp => "&",
            Self::Pipe => "|",
            Self::Caret => "^",
            Self::Tilde => "~",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Assign => "=",
            Self::EqEq => "==",
            Self::Neq => "!=",
            Self::Lt => "<",
            Self::Lte => "<=",
            Self::Gt => ">",
            Self::Gte => ">=",
            Self::PlusEq => "+=",
            Self::MinusEq => "-=",
            Self::StarEq => "*=",
            Self::SlashEq => "/=",
            Self::PercentEq => "%=",
            Self::AmpEq => "&=",
            Self::PipeEq => "|=",
            Self::CaretEq => "^=",
            Self::ShlEq => "<<=",
            Self::ShrEq => ">>=",
            Self::Newline => "newline",
            Self::Eof => "end of file",
        }
    }
}

/// A single lexical token, with 1-based source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub line: usize,
    pub column: usize,
}

impl Token {
    pub fn new(kind: TokenKind, lexeme: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            kind,
            lexeme: lexeme.into(),
            line,
            column,
        }
    }

    pub fn is(&self, kind: TokenKind) -> bool {
        self.kind == kind
    }
}
