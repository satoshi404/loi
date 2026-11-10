//! Semantic types for Loi.
//!
//! The parser represents types syntactically as [`crate::ast::Type`]. This
//! module provides the resolved semantic type used by the type checker.
//!
//! Unsuffixed literals get provisional types ([`Ty::IntLiteral`] and
//! [`Ty::FloatLiteral`]) so they can coerce to a surrounding numeric type.
//! They are concretized (to `i32` / `f64`) when stored without a declared type.

use std::fmt;

/// A resolved Loi type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    /// A user-declared `shape`.
    Named(String),
    /// A foreign handle declared with `opaque Name`.
    Opaque(String),
    /// A NUL-terminated C string handle. Constructed from a string literal or
    /// returned by an `extern flow`; never inspected in Loi.
    CStr,
    /// A slice type, written `[T]`.
    Slice(Box<Ty>),
    /// The type of an unsuffixed integer literal before coercion.
    IntLiteral,
    /// The type of an unsuffixed float literal before coercion.
    FloatLiteral,
    /// The result type of a function with no `-> T`.
    Void,
    /// Placeholder used after a diagnostic to avoid cascading errors.
    Error,
}

impl Ty {
    /// Map a primitive type name to its semantic type.
    pub fn from_name(name: &str) -> Option<Ty> {
        let ty = match name {
            "i8" => Ty::I8,
            "i16" => Ty::I16,
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "u8" => Ty::U8,
            "u16" => Ty::U16,
            "u32" => Ty::U32,
            "u64" => Ty::U64,
            "f32" => Ty::F32,
            "f64" => Ty::F64,
            "bool" => Ty::Bool,
            _ => return None,
        };
        Some(ty)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Ty::Error)
    }

    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Ty::I8
                | Ty::I16
                | Ty::I32
                | Ty::I64
                | Ty::U8
                | Ty::U16
                | Ty::U32
                | Ty::U64
                | Ty::F32
                | Ty::F64
                | Ty::IntLiteral
                | Ty::FloatLiteral
        )
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Ty::I8
                | Ty::I16
                | Ty::I32
                | Ty::I64
                | Ty::U8
                | Ty::U16
                | Ty::U32
                | Ty::U64
                | Ty::IntLiteral
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::F32 | Ty::F64 | Ty::FloatLiteral)
    }

    /// Give a literal its default type.
    pub fn concretize(&self) -> Ty {
        match self {
            Ty::IntLiteral => Ty::I32,
            Ty::FloatLiteral => Ty::F64,
            _ => self.clone(),
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::I8 => write!(f, "i8"),
            Ty::I16 => write!(f, "i16"),
            Ty::I32 => write!(f, "i32"),
            Ty::I64 => write!(f, "i64"),
            Ty::U8 => write!(f, "u8"),
            Ty::U16 => write!(f, "u16"),
            Ty::U32 => write!(f, "u32"),
            Ty::U64 => write!(f, "u64"),
            Ty::F32 => write!(f, "f32"),
            Ty::F64 => write!(f, "f64"),
            Ty::Bool => write!(f, "bool"),
            Ty::Named(name) => write!(f, "{name}"),
            Ty::Opaque(name) => write!(f, "{name}"),
            Ty::CStr => write!(f, "cstr"),
            Ty::Slice(element) => write!(f, "[{element}]"),
            Ty::IntLiteral => write!(f, "integer literal"),
            Ty::FloatLiteral => write!(f, "float literal"),
            Ty::Void => write!(f, "void"),
            Ty::Error => write!(f, "<error>"),
        }
    }
}

/// Whether a value of `actual` can be used where `expected` is required.
///
/// Unsuffixed literals coerce to any compatible numeric type. Other types must
/// match exactly; there are no implicit numeric conversions between named
/// integer widths.
pub fn assignable(expected: &Ty, actual: &Ty) -> bool {
    if expected.is_error() || actual.is_error() {
        return true;
    }
    if expected == actual {
        return true;
    }

    match actual {
        Ty::IntLiteral => expected.is_numeric(),
        Ty::FloatLiteral => expected.is_float(),
        _ => false,
    }
}

/// The common numeric type of two operands, if they can be used together.
///
/// Provisionally, only unsuffixed literals coerce. Mixing two named numeric
/// types requires an explicit `as` cast.
pub fn common_numeric(left: &Ty, right: &Ty) -> Option<Ty> {
    if left.is_error() || right.is_error() {
        return Some(Ty::Error);
    }
    if !left.is_numeric() || !right.is_numeric() {
        return None;
    }
    if left == right {
        return Some(left.clone());
    }

    match (left, right) {
        (Ty::IntLiteral, other) if other.is_integer() => Some(other.clone()),
        (other, Ty::IntLiteral) if other.is_integer() => Some(other.clone()),
        (Ty::IntLiteral, other) if other.is_float() => Some(other.clone()),
        (other, Ty::IntLiteral) if other.is_float() => Some(other.clone()),
        (Ty::FloatLiteral, other) if other.is_float() => Some(other.clone()),
        (other, Ty::FloatLiteral) if other.is_float() => Some(other.clone()),
        (Ty::IntLiteral, Ty::FloatLiteral) | (Ty::FloatLiteral, Ty::IntLiteral) => {
            Some(Ty::FloatLiteral)
        }
        _ => None,
    }
}
