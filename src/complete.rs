//! Editor completion candidates derived from the source the user is typing.
//!
//! Completion works on tokens rather than a checked program: an editor asks
//! while the buffer is incomplete, so a parse error must not hide keywords and
//! names that are already valid. The result is deliberately simple: a label,
//! an LSP `CompletionItemKind` number and a short detail string. The language
//! server converts these objects into `textDocument/completion` responses.

use crate::token::TokenKind;

/// LSP `CompletionItemKind` values used by the compiler.
const KIND_FUNCTION: u8 = 3;
const KIND_VARIABLE: u8 = 5;
const KIND_CLASS: u8 = 7;
const KIND_MODULE: u8 = 8;
const KIND_ENUM: u8 = 12;
const KIND_KEYWORD: u8 = 14;

/// Every canonical keyword, in the same set as `SINTAXE.md` §1.
const KEYWORDS: &[&str] = &[
    "alloc", "and", "as", "break", "choice", "clone", "comptime", "continue", "else", "extern",
    "false", "flow", "for", "give", "guard", "if", "import", "in", "is", "mark", "mut", "not",
    "opaque", "open", "or", "protect", "quote", "sealed", "shape", "splice", "true", "unquote",
    "while",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: u8,
    pub detail: &'static str,
}

struct Candidate {
    item: CompletionItem,
    line: Option<usize>,
    column: Option<usize>,
}

/// Return up to 50 completion items for the identifier at `line`/`column`.
/// Positions are 1-based, matching compiler diagnostics; the LSP bridge
/// subtracts one from each before calling this function.
pub fn completions(
    source: &str,
    filename: &str,
    line: usize,
    column: usize,
) -> Vec<CompletionItem> {
    let tokens = crate::tokenize(source, filename).unwrap_or_default();
    let prefix = word_prefix(source, line, column);
    let mut candidates = Vec::new();

    for keyword in KEYWORDS {
        push(
            &mut candidates,
            keyword,
            KIND_KEYWORD,
            "keyword",
            None,
            None,
        );
    }
    for intrinsic in crate::checker::intrinsic_names() {
        push(
            &mut candidates,
            intrinsic,
            KIND_FUNCTION,
            "intrinsic",
            None,
            None,
        );
    }

    for (index, token) in tokens.iter().enumerate() {
        let next = tokens.get(index + 1);
        let previous = index.checked_sub(1).and_then(|offset| tokens.get(offset));
        let position = (token.line, token.column);
        match token.kind {
            TokenKind::Flow => {
                if let Some(name) = next.filter(|token| token.kind == TokenKind::Identifier) {
                    push(
                        &mut candidates,
                        &name.lexeme,
                        KIND_FUNCTION,
                        "flow",
                        Some(position.0),
                        Some(position.1),
                    );
                }
            }
            TokenKind::Shape | TokenKind::Mark | TokenKind::Choice => {
                if let Some(name) = next.filter(|token| token.kind == TokenKind::Identifier) {
                    let (kind, detail) = match token.kind {
                        TokenKind::Shape => (KIND_CLASS, "shape"),
                        TokenKind::Mark => (KIND_ENUM, "mark"),
                        _ => (KIND_ENUM, "choice"),
                    };
                    push(
                        &mut candidates,
                        &name.lexeme,
                        kind,
                        detail,
                        Some(position.0),
                        Some(position.1),
                    );
                }
            }
            TokenKind::Opaque => {
                if let Some(name) = next.filter(|token| token.kind == TokenKind::Identifier) {
                    push(
                        &mut candidates,
                        &name.lexeme,
                        KIND_CLASS,
                        "opaque",
                        Some(position.0),
                        Some(position.1),
                    );
                }
            }
            TokenKind::Import => {
                if let Some(path) = next.filter(|token| token.kind == TokenKind::String) {
                    let alias = tokens
                        .get(index + 2)
                        .filter(|token| token.kind == TokenKind::As)
                        .and_then(|_| tokens.get(index + 3))
                        .filter(|token| token.kind == TokenKind::Identifier)
                        .map(|token| token.lexeme.clone())
                        .unwrap_or_else(|| module_stem(&path.lexeme));
                    push(
                        &mut candidates,
                        &alias,
                        KIND_MODULE,
                        "module",
                        Some(position.0),
                        Some(position.1),
                    );
                }
            }
            TokenKind::Mut => {
                if let Some(name) = next.filter(|token| token.kind == TokenKind::Identifier) {
                    push(
                        &mut candidates,
                        &name.lexeme,
                        KIND_VARIABLE,
                        "variable",
                        Some(name.line),
                        Some(name.column),
                    );
                }
            }
            TokenKind::Identifier => {
                // `<- x` / `-> x` / `<-> x` put the name after the marker;
                // `x <- value`, `x: T <- value` and `for x in` put it before.
                // A type name followed by `<-` is not a binding, so skip it
                // when the previous token is the annotation's colon.
                let follows_binding_marker = next.is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Colon | TokenKind::In | TokenKind::ArrowInOut
                    )
                }) || (next
                    .is_some_and(|token| token.kind == TokenKind::ArrowIn)
                    && !previous.is_some_and(|token| token.kind == TokenKind::Colon));
                if follows_binding_marker {
                    push(
                        &mut candidates,
                        &token.lexeme,
                        KIND_VARIABLE,
                        "variable",
                        Some(token.line),
                        Some(token.column),
                    );
                }
            }
            _ => {}
        }
    }

    candidates.retain(|candidate| {
        let declared_before = match (candidate.line, candidate.column) {
            (Some(declared_line), Some(declared_column)) => {
                declared_line < line || (declared_line == line && declared_column <= column)
            }
            _ => true,
        };
        declared_before && candidate.item.label.starts_with(&prefix)
    });

    // Keywords and intrinsics are inserted first; a local declaration with the
    // same spelling never hides the canonical word.
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.item.label.clone()));
    candidates.sort_by(|left, right| {
        (left.item.kind, &left.item.label).cmp(&(right.item.kind, &right.item.label))
    });
    candidates
        .into_iter()
        .take(50)
        .map(|candidate| candidate.item)
        .collect()
}

fn push(
    candidates: &mut Vec<Candidate>,
    label: &str,
    kind: u8,
    detail: &'static str,
    line: Option<usize>,
    column: Option<usize>,
) {
    candidates.push(Candidate {
        item: CompletionItem {
            label: label.to_string(),
            kind,
            detail,
        },
        line,
        column,
    });
}

fn module_stem(path: &str) -> String {
    let unquoted = path.trim_matches('"');
    let name = unquoted.rsplit('/').next().unwrap_or(unquoted);
    name.strip_suffix(".loi").unwrap_or(name).to_string()
}

fn word_prefix(source: &str, line: usize, column: usize) -> String {
    let Some(text) = source.lines().nth(line.saturating_sub(1)) else {
        return String::new();
    };
    let before = text
        .chars()
        .take(column.saturating_sub(1))
        .collect::<String>();
    before
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Render the compact JSON array consumed by the LSP bridge.
pub fn to_json(items: &[CompletionItem]) -> String {
    let mut out = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"label\":\"");
        out.push_str(&escape(&item.label));
        out.push_str("\",\"kind\":");
        out.push_str(&item.kind.to_string());
        out.push_str(",\"detail\":\"");
        out.push_str(item.detail);
        out.push_str("\"}");
    }
    out.push(']');
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_and_position_filter_candidates() {
        let source =
            "flow main => i32 {\n    mut alpha: i32 <- 1\n    mut beta: i32 <- 2\n    => alp\n}\n";
        let items = completions(source, "main.loi", 4, 11);
        assert!(items.iter().any(|item| item.label == "alpha"));
        assert!(!items.iter().any(|item| item.label == "beta"));
    }
}
