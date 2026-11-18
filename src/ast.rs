//! Abstract syntax tree for Loi.
//!
//! The AST mirrors the declarations, statements and expressions described in
//! `LOI_SPEC.md`. Every statement and expression carries a start [`Span`] so
//! later passes can report precise diagnostics.

/// A 1-based source position.
///
/// `end_column` is the 1-based column just past the token when the parser
/// knows it. Diagnostics use it to underline exactly the token instead of
/// guessing the extent from the source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub end_line: Option<usize>,
    pub end_column: Option<usize>,
}

impl Span {
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            end_line: None,
            end_column: None,
        }
    }

    pub fn with_end(mut self, end_line: usize, end_column: usize) -> Self {
        self.end_line = Some(end_line);
        self.end_column = Some(end_column);
        self
    }

    pub fn with_end_column(mut self, end_column: usize) -> Self {
        self.end_line.get_or_insert(self.line);
        self.end_column = Some(end_column);
        self
    }

    /// Attach the length of `lexeme` as this span's end position.
    pub fn with_lexeme(mut self, lexeme: &str) -> Self {
        self.end_line = Some(self.line);
        self.end_column = Some(self.column + lexeme.chars().count());
        self
    }

    pub fn end_line(&self) -> usize {
        self.end_line.unwrap_or(self.line)
    }

    pub fn end_column(&self) -> usize {
        self.end_column.unwrap_or(self.column)
    }
}

/// Visibility of a declaration. `Hidden` is the default (`LOI_SPEC.md` 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Hidden,
    Visible,
}

/// Parameter and call-argument direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    In,
    Out,
    Inout,
}

/// A Loi type. Slices are written `[T]` and generic instantiations `Name<T>`.
///
/// Every variant carries the span of the complete type text (including the
/// brackets of a slice and the `<...>` of a generic). Diagnostics use that span
/// instead of falling back to the enclosing statement.
#[derive(Debug, Clone)]
pub enum Type {
    Named(String, Span),
    Slice(Box<Type>, Span),
    /// A generic instantiation such as `Vec<i32>`. It is resolved to a
    /// `Named` type by monomorphization before type checking.
    Generic(String, Vec<Type>, Span),
}

impl Type {
    /// A named type without a source span, for synthesized types and tests.
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into(), Span::default())
    }

    /// A slice type without an explicit source span.
    pub fn slice(element: Type) -> Self {
        let span = element.span();
        Self::Slice(Box::new(element), span)
    }

    /// A generic type without a source span, for synthesized types and tests.
    pub fn generic(name: impl Into<String>, args: Vec<Type>) -> Self {
        Self::Generic(name.into(), args, Span::default())
    }

    /// The source extent of this type.
    pub fn span(&self) -> Span {
        match self {
            Self::Named(_, span) | Self::Slice(_, span) | Self::Generic(_, _, span) => *span,
        }
    }
}

/// Spans are diagnostic metadata, not part of a type's meaning: two types are
/// equal when their names and structure match, regardless of where they were
/// written. This keeps AST assertions focused on syntax.
impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Named(left, _), Self::Named(right, _)) => left == right,
            (Self::Slice(left, _), Self::Slice(right, _)) => left == right,
            (Self::Generic(left_name, left_args, _), Self::Generic(right_name, right_args, _)) => {
                left_name == right_name && left_args == right_args
            }
            _ => false,
        }
    }
}

impl Eq for Type {}

/// A complete source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Import(ImportDecl),
    Interface(InterfaceDecl),
    /// `mark Name [ A, B ]`: a finite set of payload-free tags.
    Mark(MarkDecl),
    /// `choice Name [ A: T, B: U ]`: a tagged union with one payload per
    /// variant.
    Choice(ChoiceDecl),
    Opaque(OpaqueDecl),
    Global(GlobalDecl),
    Function(FunctionDecl),
}

impl Item {
    /// The name a declaration contributes to the program namespace.
    ///
    /// `import` declarations carry no name; they are resolved by the driver
    /// before semantic analysis.
    pub fn declared_name(&self) -> Option<&str> {
        match self {
            Item::Import(_) => None,
            Item::Interface(decl) => Some(&decl.name),
            Item::Mark(decl) => Some(&decl.name),
            Item::Choice(decl) => Some(&decl.name),
            Item::Opaque(decl) => Some(&decl.name),
            Item::Global(decl) => Some(&decl.name),
            Item::Function(decl) => Some(&decl.name),
        }
    }
}

/// `import "path.loi"` pulls another source file into the compilation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub span: Span,
    pub path: String,
}

/// A top-level `let`/`var` declaration. Globals are initialized once, before
/// any function runs, and their initializers must be constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalDecl {
    pub span: Span,
    pub visibility: Visibility,
    pub mutable: bool,
    pub name: String,
    pub ty: Option<Type>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceDecl {
    pub span: Span,
    /// Source line of the closing `]`, used to place trailing comments.
    pub close_line: usize,
    pub visibility: Visibility,
    pub name: String,
    /// Type parameters declared as `shape Name<T, U>`.
    pub type_params: Vec<String>,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub span: Span,
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
}

/// `mark Name [ A, B ]` declares a finite set of payload-free tags. A mark is
/// a scalar: it can be bound, copied, passed to flows and compared with
/// `==`/`!=`. Variants are referenced as `Name.Variant`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkDecl {
    pub span: Span,
    /// Source line of the closing `]`, used to place trailing comments.
    pub close_line: usize,
    pub visibility: Visibility,
    pub name: String,
    pub variants: Vec<MarkVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkVariant {
    pub span: Span,
    pub name: String,
}

/// `choice Name [ A: T, B: U ]` declares a tagged union. Each variant carries
/// exactly one payload. A value is built with `Name.Variant(value)`, tested with
/// `value is Name.Variant`, and read with `value.Variant` (checked at runtime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceDecl {
    pub span: Span,
    /// Source line of the closing `]`, used to place trailing comments.
    pub close_line: usize,
    pub visibility: Visibility,
    pub name: String,
    pub variants: Vec<ChoiceVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceVariant {
    pub span: Span,
    pub name: String,
    pub ty: Type,
}

/// A foreign handle. `opaque Name` is a distinct, copyable scalar that the
/// backend represents as an opaque pointer (`void *`). It cannot be
/// dereferenced, indexed or inspected; it only moves across the FFI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueDecl {
    pub span: Span,
    pub visibility: Visibility,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDecl {
    pub span: Span,
    pub visibility: Visibility,
    pub name: String,
    /// Type parameters declared as `flow name<T, U>`.
    pub type_params: Vec<String>,
    /// True for `comptime flow`: the body must be pure and is evaluated at
    /// compile time when called from a constant context.
    pub comptime: bool,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Block,
    /// True for `extern flow`: the declaration has no Loi body and resolves to
    /// a C symbol at link time.
    pub external: bool,
    /// Optional C symbol for an `extern flow ... as <symbol>` declaration.
    /// `None` means the Loi name is the C symbol.
    pub link_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub span: Span,
    pub direction: Direction,
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    /// Source line of the opening `{`, used to anchor comments.
    pub open_line: usize,
    /// Source line of the closing `}`, used to anchor comments.
    pub close_line: usize,
}

/// A statement together with its starting source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StmtKind {
    Let {
        mutable: bool,
        name: String,
        ty: Option<Type>,
        value: Option<Expr>,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
    },
    Expr(Expr),
    If {
        condition: Expr,
        then_block: Block,
        else_branch: Option<ElseBranch>,
    },
    While {
        condition: Expr,
        body: Block,
    },
    ForRange {
        variable: String,
        start: Expr,
        end: Expr,
        body: Block,
    },
    ForEach {
        direction: Direction,
        variable: String,
        iterable: Expr,
        body: Block,
    },
    Protect(Block),
    Return(Option<Expr>),
    Continue,
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElseBranch {
    Block(Block),
    If(Box<Stmt>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

/// An expression together with its starting source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    Int(String),
    Float(String),
    Bool(bool),
    /// A string literal. The stored text is already unescaped; its runtime
    /// type is `[u8]` and its length is the number of UTF-8 bytes.
    String(String),
    Ident(String),
    /// `[a, b, c]` (inferred), `[T: a, b, c]` (explicit) or `[T:]` (empty).
    SliceLiteral {
        element_type: Option<Type>,
        elements: Vec<Expr>,
    },
    InterfaceLiteral {
        name: String,
        type_args: Vec<Type>,
        fields: Vec<FieldInit>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        ty: Type,
    },
    Call {
        callee: Box<Expr>,
        type_args: Vec<Type>,
        args: Vec<CallArg>,
    },
    Alloc {
        target: Box<Expr>,
        element_type: Type,
        count: Box<Expr>,
    },
    Move(Box<Expr>),
    Copy(Box<Expr>),
    Member {
        base: Box<Expr>,
        field: String,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `comptime expr`: force compile-time evaluation.
    Comptime(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldInit {
    pub name: String,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallArg {
    pub direction: Direction,
    pub value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    /// `is`: tests which variant a `choice` value holds.
    Is,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}
