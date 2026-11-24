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
    /// A user-declared `mark`: a finite, payload-free tag set. It is a scalar.
    Mark(String),
    /// A user-declared `choice`: a tagged union with one payload per variant.
    /// It is a composite value and follows the ownership rules of shapes.
    Choice(String),
    /// A foreign handle declared with `opaque Name`.
    Opaque(String),
    /// A C callback function pointer. The plain `callback` type is the fixed
    /// signature `int32_t (*)(int32_t, int32_t)`; `callback(T, ...) => R` names
    /// other signatures. Constructed with `callback(flow)` from a non-extern
    /// Loi flow with the same signature. Values are copyable but cannot be
    /// called or dereferenced from Loi.
    Callback(CallbackSig),
    /// A NUL-terminated C string handle. Constructed from a string literal or
    /// returned by an `extern flow`; never inspected in Loi.
    CStr,
    /// A slice type, written `[T]`.
    Slice(Box<Ty>),
    /// The type of an unsuffixed integer literal before coercion.
    IntLiteral,
    /// The type of an unsuffixed float literal before coercion.
    FloatLiteral,
    /// A comptime-only block of Loi statements produced by `quote`.
    Code,
    /// The result type of a function with no `-> T`.
    Void,
    /// Placeholder used after a diagnostic to avoid cascading errors.
    Error,
}

/// The C signature carried by a [`Ty::Callback`].
///
/// The fixed `(i32, i32) => i32` signature keeps the historical spelling
/// `callback` in diagnostics and the historical C typedef `LoiCallback`. Other
/// signatures get a structural display and a distinct, guarded C typedef.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackSig {
    pub params: Vec<Ty>,
    pub result: Box<Ty>,
}

impl CallbackSig {
    /// The original ABI: `int32_t (*)(int32_t, int32_t)`.
    pub fn fixed() -> Self {
        Self {
            params: vec![Ty::I32, Ty::I32],
            result: Box::new(Ty::I32),
        }
    }

    pub fn is_fixed(&self) -> bool {
        self.params == [Ty::I32, Ty::I32] && *self.result == Ty::I32
    }

    /// A stable structural key used to name the generated C typedef.
    pub fn key(&self) -> String {
        let mut parts: Vec<String> = self.params.iter().map(callback_key_part).collect();
        parts.push(callback_key_part(&self.result));
        format!("{}_{}", self.params.len(), parts.join("_"))
    }

    /// The C typedef name. The fixed signature keeps the existing public name.
    pub fn c_name(&self) -> String {
        if self.is_fixed() {
            "LoiCallback".to_string()
        } else {
            format!("LoiCallback_{}", self.key())
        }
    }

    /// The include guard for the generated typedef.
    pub fn c_guard(&self) -> String {
        format!("{}_DEFINED", self.c_name().to_ascii_uppercase())
    }

    /// Source-like rendering that expands nested callback aliases.
    pub fn display(&self) -> String {
        let params = self
            .params
            .iter()
            .map(callback_key_display)
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "callback({params}) => {}",
            callback_key_display(&self.result)
        )
    }
}

fn callback_key_part(ty: &Ty) -> String {
    let name = match ty {
        Ty::Callback(sig) => sig.display(),
        other => other.to_string(),
    };
    let mut key = String::new();
    let mut pending_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_underscore && !key.is_empty() {
                key.push('_');
            }
            pending_underscore = false;
            key.push(ch);
        } else {
            pending_underscore = true;
        }
    }
    key
}

fn callback_key_display(ty: &Ty) -> String {
    match ty {
        Ty::Callback(sig) => sig.display(),
        other => other.to_string(),
    }
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
            "Code" => Ty::Code,
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
            Ty::Named(name) | Ty::Mark(name) | Ty::Choice(name) => write!(f, "{name}"),
            Ty::Opaque(name) => write!(f, "{name}"),
            Ty::Callback(sig) => {
                if sig.is_fixed() {
                    write!(f, "callback")
                } else {
                    write!(f, "{}", sig.display())
                }
            }
            Ty::CStr => write!(f, "cstr"),
            Ty::Slice(element) => write!(f, "[{element}]"),
            Ty::Code => write!(f, "Code"),
            Ty::IntLiteral => write!(f, "integer literal"),
            Ty::FloatLiteral => write!(f, "float literal"),
            Ty::Void => write!(f, "void"),
            Ty::Error => write!(f, "<error>"),
        }
    }
}

/// Which comparison operators a type supports.
///
/// Both flags are structural: a composite supports an operator only when every
/// part it contains supports it. Mark and choice variant order follows source
/// declaration order, so both are orderable. Opaque handles have identity but
/// no meaningful order, and callbacks are deliberately neither comparable nor
/// orderable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComparisonCaps {
    pub eq: bool,
    pub ord: bool,
}

impl ComparisonCaps {
    pub const NONE: Self = Self {
        eq: false,
        ord: false,
    };
    pub const BOTH: Self = Self {
        eq: true,
        ord: true,
    };
    pub const IDENTITY: Self = Self {
        eq: true,
        ord: false,
    };

    /// Combine the capabilities of all fields or elements of a composite.
    pub fn and(self, other: Self) -> Self {
        Self {
            eq: self.eq && other.eq,
            ord: self.ord && other.ord,
        }
    }
}

/// Access to the fields of shapes and the payloads of choices.
///
/// The checker and the C backend already own those declaration tables; this
/// trait lets both use the same structural comparison rule without a second
/// copy of it. The monomorphizer has only names at bound-validation time and
/// supplies the coarse rule there (see `coarse_caps` in `monomorphize.rs`).
pub trait ComparisonEnv {
    fn shape_fields(&self, name: &str) -> Option<Vec<Ty>>;
    fn choice_payloads(&self, name: &str) -> Option<Vec<Ty>>;
}

/// Compute the comparison capabilities of a resolved type.
pub fn comparison_caps(ty: &Ty, env: &impl ComparisonEnv) -> ComparisonCaps {
    comparison_caps_inner(ty, env, &mut std::collections::HashSet::new())
}

fn comparison_caps_inner(
    ty: &Ty,
    env: &impl ComparisonEnv,
    visited: &mut std::collections::HashSet<String>,
) -> ComparisonCaps {
    match ty {
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
        | Ty::Bool
        | Ty::Mark(_)
        | Ty::CStr
        | Ty::IntLiteral
        | Ty::FloatLiteral => ComparisonCaps::BOTH,
        Ty::Opaque(_) => ComparisonCaps::IDENTITY,
        Ty::Callback(_) | Ty::Void | Ty::Error | Ty::Code => ComparisonCaps::NONE,
        Ty::Slice(element) => comparison_caps_inner(element, env, visited),
        Ty::Named(name) => {
            let key = format!("shape:{name}");
            if !visited.insert(key.clone()) {
                return ComparisonCaps::BOTH;
            }
            let caps = env
                .shape_fields(name)
                .unwrap_or_default()
                .iter()
                .fold(ComparisonCaps::BOTH, |acc, field| {
                    acc.and(comparison_caps_inner(field, env, visited))
                });
            visited.remove(&key);
            caps
        }
        Ty::Choice(name) => {
            let key = format!("choice:{name}");
            if !visited.insert(key.clone()) {
                return ComparisonCaps::BOTH;
            }
            let caps = env
                .choice_payloads(name)
                .unwrap_or_default()
                .iter()
                .fold(ComparisonCaps::BOTH, |acc, payload| {
                    acc.and(comparison_caps_inner(payload, env, visited))
                });
            visited.remove(&key);
            caps
        }
    }
}

/// Whether a value of `actual` can be used where `expected` is required.
///
/// Unsuffixed literals coerce to any compatible numeric type. Other types must
/// match exactly; there are no implicit numeric conversions between named
/// integer widths.
/// Name of the linear-resource release flow for a named shape.
///
/// The convention is `<snake_case_shape>_release`: `File` becomes
///
/// The convention is `<snake_case_shape>_release`: `File` becomes
/// `file_release`, `HttpClient` becomes `http_client_release`. Only named
/// shapes can own a resource; other types have no release flow.
pub fn release_flow_name(ty: &Ty) -> Option<String> {
    let Ty::Named(name) = ty else {
        return None;
    };
    Some(release_flow_name_for(name))
}

/// Spell the release flow name directly from a shape's source name.
///
/// The module resolver uses this before type checking, when only AST names are
/// available. Keeping the spelling here avoids a second snake-case rule.
pub fn release_flow_name_for(shape: &str) -> String {
    if let Some((module, item)) = shape.split_once("::") {
        return format!("{module}::{}_release", snake_case(item));
    }
    format!("{}_release", snake_case(shape))
}

fn snake_case(name: &str) -> String {
    let mut snake = String::new();
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if index > 0 && !snake.ends_with('_') {
                snake.push('_');
            }
            for lower in ch.to_lowercase() {
                snake.push(lower);
            }
        } else {
            snake.push(ch);
        }
    }
    snake
}

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
