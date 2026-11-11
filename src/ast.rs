//! Abstract syntax tree for Loi.
//!
//! The AST mirrors the declarations, statements and expressions described in
//! `LOI_SPEC.md`. Every statement and expression carries a start [`Span`] so
//! later passes can report precise diagnostics.

/// A 1-based source position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
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

/// A Loi type. Slices are written `[T]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Named(String),
    Slice(Box<Type>),
}

/// A complete source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Import(ImportDecl),
    Interface(InterfaceDecl),
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
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub span: Span,
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
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
