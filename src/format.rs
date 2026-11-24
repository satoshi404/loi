//! `printf`-style format strings for Loi's `printf` intrinsic.
//!
//! The parser is deliberately small and strict: it recognises the conversion
//! specifiers Loi can type-check, rejects C features that would consume extra
//! varargs without Loi knowing about them (`*` width/precision), and reports a
//! clear error for malformed strings. Both the checker (argument validation)
//! and the C backend (cast selection) use the same parse so they cannot drift.

use crate::types::Ty;

/// The Loi-side category a conversion expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatKind {
    SignedInt,
    UnsignedInt,
    Float,
    Str,
    Char,
}

/// One `%...` conversion in a format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatSpec {
    /// The full specifier as written, including the leading `%`.
    pub raw: String,
    /// The conversion character (`d`, `u`, `f`, `s`, ...).
    pub conversion: char,
    /// C length modifier: `""`, `h`, `hh`, `l`, `ll`, `L`, `z`, `j` or `t`.
    pub length: String,
}

impl FormatSpec {
    /// The Loi value category this conversion accepts.
    pub fn kind(&self) -> Option<FormatKind> {
        match self.conversion {
            'd' | 'i' => Some(FormatKind::SignedInt),
            'u' | 'x' | 'X' | 'o' => Some(FormatKind::UnsignedInt),
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'a' | 'A' => Some(FormatKind::Float),
            's' => Some(FormatKind::Str),
            'c' => Some(FormatKind::Char),
            _ => None,
        }
    }

    /// The C type to cast the corresponding vararg to. Casting to exactly what
    /// the specifier reads keeps every conversion well-defined even when the
    /// Loi value has a different width (for example `i64` with `%d`).
    pub fn c_cast(&self) -> &'static str {
        match self.kind() {
            Some(FormatKind::SignedInt) => match self.length.as_str() {
                "ll" | "j" => "long long",
                "l" | "z" | "t" => "long",
                _ => "int",
            },
            Some(FormatKind::UnsignedInt) => match self.length.as_str() {
                "ll" | "j" => "unsigned long long",
                "l" | "z" | "t" => "unsigned long",
                _ => "unsigned int",
            },
            Some(FormatKind::Float) => {
                if self.length == "L" {
                    "long double"
                } else {
                    "double"
                }
            }
            Some(FormatKind::Char) => "int",
            Some(FormatKind::Str) => "const char *",
            None => "int",
        }
    }
}

/// A literal chunk or one conversion of a format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatPiece {
    Text(String),
    Spec(FormatSpec),
}

/// Split a format string into literal text and conversions.
///
/// Returns a human-readable error for a dangling `%`, an unknown conversion or
/// a dynamic `*` width/precision (which would need an extra Loi argument that
/// the checker cannot describe).
pub fn parse_format(format: &str) -> Result<Vec<FormatPiece>, String> {
    let mut pieces: Vec<FormatPiece> = Vec::new();
    let mut text = String::new();
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '%' {
            text.push(ch);
            continue;
        }

        if chars.peek() == Some(&'%') {
            chars.next();
            text.push('%');
            continue;
        }

        let mut raw = String::from("%");
        // Flags, then width and precision. `*` is rejected: C would pull an
        // extra vararg that Loi has no way to type-check or supply.
        while matches!(chars.peek(), Some('-' | '+' | ' ' | '#' | '0')) {
            let flag = chars.next().unwrap();
            raw.push(flag);
        }
        while let Some('0'..='9' | '*') = chars.peek() {
            let ch = chars.next().unwrap();
            if ch == '*' {
                return Err("dynamic '*' width/precision is not supported".to_string());
            }
            raw.push(ch);
        }
        if chars.peek() == Some(&'.') {
            raw.push(chars.next().unwrap());
            while let Some('0'..='9' | '*') = chars.peek() {
                let ch = chars.next().unwrap();
                if ch == '*' {
                    return Err("dynamic '*' width/precision is not supported".to_string());
                }
                raw.push(ch);
            }
        }

        let length = parse_length(&mut chars, &mut raw)?;

        let conversion = chars
            .next()
            .ok_or_else(|| "format string ends with '%'".to_string())?;
        raw.push(conversion);

        let spec = FormatSpec {
            raw,
            conversion,
            length,
        };
        let Some(kind) = spec.kind() else {
            return Err(format!("unsupported conversion '%{}'", conversion));
        };
        if !length_is_valid(kind, &spec.length) {
            return Err(format!(
                "length modifier '{}' is not valid for '%{}'",
                spec.length, conversion
            ));
        }

        if !text.is_empty() {
            pieces.push(FormatPiece::Text(std::mem::take(&mut text)));
        }
        pieces.push(FormatPiece::Spec(spec));
    }

    if !text.is_empty() {
        pieces.push(FormatPiece::Text(text));
    }
    Ok(pieces)
}

/// Parse exactly one C length modifier. Repeating letters (`%lll`) or mixing
/// them (`%hl`) would let C read a vararg of a different type than `c_cast`
/// supplies, so an unrecognised sequence is a format error.
fn parse_length(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    raw: &mut String,
) -> Result<String, String> {
    let first = match chars.peek() {
        Some('h' | 'l' | 'L' | 'z' | 'j' | 't') => chars.next().unwrap(),
        _ => return Ok(String::new()),
    };
    let mut length = first.to_string();
    raw.push(first);
    if matches!(first, 'h' | 'l') && chars.peek() == Some(&first) {
        length.push(chars.next().unwrap());
        raw.push(first);
    }
    Ok(length)
}

/// Length modifiers that stay well-defined for each Loi conversion category.
fn length_is_valid(kind: FormatKind, length: &str) -> bool {
    match kind {
        FormatKind::SignedInt | FormatKind::UnsignedInt => {
            matches!(length, "" | "h" | "hh" | "l" | "ll" | "z" | "j" | "t")
        }
        // `%lf` is accepted by C as an alias of `%f`; `%Lf` reads a long double.
        FormatKind::Float => matches!(length, "" | "l" | "L"),
        // Wide `l` variants of `%c`/`%s` are not part of Loi's string model.
        FormatKind::Char | FormatKind::Str => length.is_empty(),
    }
}

/// Return `true` when a Loi type may be passed to the given conversion.
///
/// `Str` is handled by the caller because a string literal has type `[u8]`
/// but is still a valid `%s` argument thanks to its trailing NUL byte.
pub fn type_matches(kind: FormatKind, ty: &Ty) -> bool {
    match kind {
        FormatKind::SignedInt | FormatKind::Char | FormatKind::UnsignedInt => ty.is_integer(),
        FormatKind::Float => ty.is_float(),
        FormatKind::Str => matches!(ty, Ty::CStr),
    }
}

/// A piece of a `format(...)` template string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BracedPiece {
    Text(String),
    Placeholder,
}

/// Parse a `format(...)` template. `{}` is a placeholder, `{{`/`}}` escape
/// literal braces.
pub fn parse_braced_format(format: &str) -> Result<Vec<BracedPiece>, String> {
    let mut pieces: Vec<BracedPiece> = Vec::new();
    let mut text = String::new();
    let mut chars = format.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    text.push('{');
                    continue;
                }
                if chars.peek() == Some(&'}') {
                    chars.next();
                    if !text.is_empty() {
                        pieces.push(BracedPiece::Text(std::mem::take(&mut text)));
                    }
                    pieces.push(BracedPiece::Placeholder);
                } else {
                    return Err("unclosed '{' in format string; use '{}' for a placeholder or '{{' for a literal '{'".to_string());
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    text.push('}');
                } else {
                    return Err(
                        "unmatched '}' in format string; use '}}' for a literal '}'".to_string()
                    );
                }
            }
            _ => text.push(ch),
        }
    }
    if !text.is_empty() {
        pieces.push(BracedPiece::Text(text));
    }
    Ok(pieces)
}

/// Whether a value can be printed by a `{}` placeholder.
pub fn is_braced_printable(ty: &Ty, _is_string_literal: bool) -> bool {
    ty.is_integer()
        || ty.is_float()
        || matches!(ty, Ty::Bool)
        || matches!(ty, Ty::CStr)
        || matches!(ty, Ty::Slice(inner) if matches!(inner.as_ref(), Ty::U8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(format: &str) -> Vec<FormatSpec> {
        parse_format(format)
            .unwrap()
            .into_iter()
            .filter_map(|piece| match piece {
                FormatPiece::Spec(spec) => Some(spec),
                FormatPiece::Text(_) => None,
            })
            .collect()
    }

    #[test]
    fn parses_common_conversions() {
        let parsed = specs("n=%d f=%.2f s=%s pct=%%");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].conversion, 'd');
        assert_eq!(parsed[1].raw, "%.2f");
        assert_eq!(parsed[2].conversion, 's');
    }

    #[test]
    fn keeps_length_modifiers() {
        let parsed = specs("%lld %llu %hhx %Lf");
        assert_eq!(parsed[0].length, "ll");
        assert_eq!(parsed[0].c_cast(), "long long");
        assert_eq!(parsed[1].c_cast(), "unsigned long long");
        assert_eq!(parsed[2].c_cast(), "unsigned int");
        assert_eq!(parsed[3].c_cast(), "long double");
    }

    #[test]
    fn rejects_bad_formats() {
        assert!(parse_format("ends with %").is_err());
        assert!(parse_format("%q").is_err());
        assert!(parse_format("%*d").is_err());
    }
}
