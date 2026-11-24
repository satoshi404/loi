//! Lightweight editor intelligence used by the LSP bridge.
//!
//! Completion already works on raw tokens so an unfinished buffer still gets
//! useful results. Hover, definition and signature help use the same idea: they
//! read the exact text cached by the language server and return ready-to-use
//! LSP result objects. The compiler therefore never has to parse a half-typed
//! document just to answer an editor question.

use crate::lexer::Comment;
use crate::token::{Token, TokenKind};
use crate::tokenize_with_comments;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoKind {
    Hover,
    Definition,
    Signature,
    References,
    PrepareRename,
    Rename,
    DocumentSymbol,
    SemanticTokens,
    InlayHint,
    FoldingRange,
}

impl InfoKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hover" => Some(Self::Hover),
            "definition" => Some(Self::Definition),
            "signature" => Some(Self::Signature),
            "references" => Some(Self::References),
            "prepareRename" | "prepare-rename" => Some(Self::PrepareRename),
            "rename" => Some(Self::Rename),
            "documentSymbol" | "document-symbol" => Some(Self::DocumentSymbol),
            "semanticTokens" | "semantic-tokens" => Some(Self::SemanticTokens),
            "inlayHint" | "inlay-hint" => Some(Self::InlayHint),
            "foldingRange" | "folding-range" => Some(Self::FoldingRange),
            _ => None,
        }
    }
}

/// Return a complete LSP result value for one editor request.
///
/// `line`, `column` and positions in the result use the compiler's 1-based
/// convention on input and LSP's 0-based convention on output. `uri` is only
/// needed for location-bearing results. `new_name` is used by rename.
pub fn info_json(
    source: &str,
    filename: &str,
    line: usize,
    column: usize,
    uri: &str,
    kind: InfoKind,
    new_name: &str,
) -> String {
    let Ok((tokens, comments)) = tokenize_with_comments(source, filename) else {
        return empty_result(kind);
    };
    match kind {
        InfoKind::Hover => hover_json(&tokens, &comments, line, column),
        InfoKind::Definition => definition_json(&tokens, line, column, uri),
        InfoKind::Signature => signature_json(&tokens, &comments, line, column),
        InfoKind::References => references_json(&tokens, line, column, uri),
        InfoKind::PrepareRename => prepare_rename_json(&tokens, line, column),
        InfoKind::Rename => rename_json(&tokens, line, column, uri, new_name),
        InfoKind::DocumentSymbol => document_symbol_json(&tokens),
        InfoKind::SemanticTokens => semantic_tokens_json(&tokens, &comments),
        InfoKind::InlayHint => inlay_hint_json(&tokens),
        InfoKind::FoldingRange => folding_range_json(&tokens, &comments),
    }
}

fn empty_result(kind: InfoKind) -> String {
    match kind {
        InfoKind::Signature => {
            r#"{"signatures":[],"activeSignature":0,"activeParameter":0}"#.to_string()
        }
        InfoKind::References
        | InfoKind::DocumentSymbol
        | InfoKind::SemanticTokens
        | InfoKind::InlayHint
        | InfoKind::FoldingRange => "[]".to_string(),
        _ => "null".to_string(),
    }
}

fn meaningful(tokens: &[Token], index: usize) -> bool {
    index < tokens.len() && !matches!(tokens[index].kind, TokenKind::Newline | TokenKind::Eof)
}

fn previous(tokens: &[Token], index: usize) -> Option<usize> {
    index.checked_sub(1).and_then(|mut cursor| {
        while cursor > 0 && !meaningful(tokens, cursor) {
            cursor -= 1;
        }
        meaningful(tokens, cursor).then_some(cursor)
    })
}

fn next(tokens: &[Token], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < tokens.len() && !meaningful(tokens, cursor) {
        cursor += 1;
    }
    meaningful(tokens, cursor).then_some(cursor)
}

fn token_at(tokens: &[Token], line: usize, column: usize) -> Option<usize> {
    tokens.iter().enumerate().find_map(|(index, token)| {
        if !meaningful(tokens, index) || token.line != line {
            return None;
        }
        let end = token.column + token.lexeme.chars().count();
        (column >= token.column && column <= end).then_some(index)
    })
}

fn identifier_before(tokens: &[Token], line: usize, column: usize) -> Option<usize> {
    tokens
        .iter()
        .enumerate()
        .filter(|(index, token)| {
            meaningful(tokens, *index)
                && token.kind == TokenKind::Identifier
                && token.line == line
                && token.column + token.lexeme.chars().count() <= column
        })
        .map(|(index, _)| index)
        .next_back()
}

fn target_token(tokens: &[Token], line: usize, column: usize) -> Option<usize> {
    token_at(tokens, line, column).or_else(|| identifier_before(tokens, line, column))
}

fn is_declaration(tokens: &[Token], index: usize) -> bool {
    if tokens[index].kind != TokenKind::Identifier {
        return false;
    }
    let before = previous(tokens, index).map(|position| tokens[position].kind);
    let after = next(tokens, index).map(|position| tokens[position].kind);
    if before == Some(TokenKind::Dot) {
        return false;
    }
    matches!(
        before,
        Some(
            TokenKind::Mut
                | TokenKind::Flow
                | TokenKind::Shape
                | TokenKind::Mark
                | TokenKind::Choice
                | TokenKind::Opaque
                | TokenKind::Extern
                | TokenKind::Open
                | TokenKind::Sealed
                | TokenKind::ArrowIn
                | TokenKind::ArrowInOut
                | TokenKind::As
        )
    ) || matches!(after, Some(TokenKind::In | TokenKind::Colon))
        || (after == Some(TokenKind::ArrowIn) && before != Some(TokenKind::Colon))
}

fn find_declaration(tokens: &[Token], name: &str, before: usize) -> Option<usize> {
    (0..before).rev().find(|&index| {
        tokens[index].kind == TokenKind::Identifier
            && tokens[index].lexeme == name
            && is_declaration(tokens, index)
    })
}

fn declaration_kind(tokens: &[Token], index: usize) -> &'static str {
    let mut cursor = index;
    while let Some(before) = previous(tokens, cursor) {
        let kind = tokens[before].kind;
        if matches!(
            kind,
            TokenKind::Flow
                | TokenKind::Shape
                | TokenKind::Mark
                | TokenKind::Choice
                | TokenKind::Opaque
                | TokenKind::Extern
                | TokenKind::As
                | TokenKind::Import
        ) {
            return keyword_kind(kind);
        }
        if matches!(kind, TokenKind::Open | TokenKind::Sealed) {
            cursor = before;
            continue;
        }
        break;
    }
    if next(tokens, index).is_some_and(|after| tokens[after].kind == TokenKind::In)
        || previous(tokens, index).is_some_and(|before| {
            matches!(
                tokens[before].kind,
                TokenKind::ArrowIn | TokenKind::ArrowInOut | TokenKind::Mut
            )
        })
        || next(tokens, index).is_some_and(|after| tokens[after].kind == TokenKind::Colon)
    {
        "variable"
    } else {
        "identifier"
    }
}

fn keyword_kind(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Flow => "flow",
        TokenKind::Shape => "shape",
        TokenKind::Mark => "mark",
        TokenKind::Choice => "choice",
        TokenKind::Opaque => "opaque",
        TokenKind::Extern => "extern",
        TokenKind::As | TokenKind::Import => "module",
        _ => "identifier",
    }
}

fn doc_comments(comments: &[Comment], line: usize) -> Vec<String> {
    let mut above = Vec::new();
    let mut expected = line.saturating_sub(1);
    for comment in comments.iter().rev() {
        if comment.line == expected
            && !comment.trailing
            && let Some(text) = doc_text(comment)
        {
            above.push(text);
            if expected == 0 {
                break;
            }
            expected -= 1;
            continue;
        }
        if comment.line < expected {
            break;
        }
    }
    above.reverse();
    if let Some(trailing) = comments
        .iter()
        .find(|comment| comment.line == line && comment.trailing)
        .and_then(doc_text)
    {
        above.push(trailing);
    }
    above
}

fn doc_text(comment: &Comment) -> Option<String> {
    let text = comment.text.trim_start();
    if !text.starts_with("///") {
        return None;
    }
    let body = text.trim_start_matches('/').trim();
    (!body.is_empty()).then(|| body.to_string())
}

fn lsp_range(token: &Token) -> String {
    let start_line = token.line.saturating_sub(1);
    let start_character = token.column.saturating_sub(1);
    let end_character = start_character + token.lexeme.chars().count();
    format!(
        "{{\"start\":{{\"line\":{start_line},\"character\":{start_character}}},\"end\":{{\"line\":{start_line},\"character\":{end_character}}}}}"
    )
}

fn hover_json(tokens: &[Token], comments: &[Comment], line: usize, column: usize) -> String {
    let Some(index) = target_token(tokens, line, column) else {
        return "null".to_string();
    };
    let token = &tokens[index];
    let declaration = find_declaration(tokens, &token.lexeme, index);
    let (kind, docs, signature) = match declaration {
        Some(position) => (
            declaration_kind(tokens, position),
            doc_comments(comments, tokens[position].line),
            signature_label(tokens, position),
        ),
        None => ("identifier", Vec::new(), None),
    };

    let mut value = format!("**{kind}** `{}`", token.lexeme);
    if let Some(label) = signature {
        value.push_str("\n\n```loi\n");
        value.push_str(&label);
        value.push_str("\n```");
    }
    for doc in docs {
        value.push_str("\n\n");
        value.push_str(&doc);
    }
    format!(
        "{{\"contents\":{{\"kind\":\"markdown\",\"value\":{}}},\"range\":{}}}",
        quote(&value),
        lsp_range(token)
    )
}

fn definition_json(tokens: &[Token], line: usize, column: usize, uri: &str) -> String {
    let Some(index) = target_token(tokens, line, column) else {
        return "null".to_string();
    };
    let Some(declaration) = find_declaration(tokens, &tokens[index].lexeme, index) else {
        return "null".to_string();
    };
    format!(
        "{{\"uri\":{},\"range\":{}}}",
        quote(uri),
        lsp_range(&tokens[declaration])
    )
}

fn signature_json(tokens: &[Token], comments: &[Comment], line: usize, column: usize) -> String {
    let Some(open) = open_paren_before(tokens, line, column) else {
        return empty_result(InfoKind::Signature);
    };
    let Some(name_index) = previous(tokens, open)
        .filter(|&index| meaningful(tokens, index) && tokens[index].kind == TokenKind::Identifier)
    else {
        return empty_result(InfoKind::Signature);
    };
    let Some(declaration) = find_declaration(tokens, &tokens[name_index].lexeme, name_index) else {
        return empty_result(InfoKind::Signature);
    };
    let label =
        signature_label(tokens, declaration).unwrap_or_else(|| tokens[name_index].lexeme.clone());
    let parameters = parameter_labels(tokens, declaration);
    let active = active_parameter(tokens, open, line, column);
    let docs = doc_comments(comments, tokens[declaration].line).join("\n");

    let mut out = String::from("{\"signatures\":[{\"label\":");
    out.push_str(&quote(&label));
    if !docs.is_empty() {
        out.push_str(",\"documentation\":");
        out.push_str(&quote(&docs));
    }
    if !parameters.is_empty() {
        out.push_str(",\"parameters\":[");
        for (index, parameter) in parameters.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"label\":");
            out.push_str(&quote(parameter));
            out.push('}');
        }
        out.push(']');
    }
    out.push_str("}],\"activeSignature\":0,\"activeParameter\":");
    out.push_str(&active.to_string());
    out.push('}');
    out
}

fn open_paren_before(tokens: &[Token], line: usize, column: usize) -> Option<usize> {
    let limit = token_at(tokens, line, column)
        .or_else(|| {
            tokens
                .iter()
                .enumerate()
                .filter(|(index, token)| {
                    meaningful(tokens, *index)
                        && (token.line < line || (token.line == line && token.column <= column))
                })
                .map(|(index, _)| index)
                .next_back()
        })
        .unwrap_or(0);
    let mut depth = 0usize;
    for index in (0..=limit).rev() {
        let token = &tokens[index];
        match token.kind {
            TokenKind::RParen => depth += 1,
            TokenKind::LParen => {
                if depth == 0 {
                    return Some(index);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn active_parameter(tokens: &[Token], open: usize, line: usize, column: usize) -> u64 {
    let mut depth = 0usize;
    let mut commas = 0u64;
    for token in tokens.iter().skip(open + 1) {
        if token.line > line || (token.line == line && token.column > column) {
            break;
        }
        match token.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            TokenKind::Comma if depth == 0 => commas += 1,
            _ => {}
        }
    }
    commas
}

fn parameter_labels(tokens: &[Token], declaration: usize) -> Vec<String> {
    let mut labels = Vec::new();
    let mut cursor = declaration + 1;
    while cursor < tokens.len() {
        let token = &tokens[cursor];
        if matches!(token.kind, TokenKind::LBrace | TokenKind::Eof) {
            break;
        }
        if token.kind == TokenKind::Identifier
            && previous(tokens, cursor).is_some_and(|index| {
                matches!(
                    tokens[index].kind,
                    TokenKind::ArrowIn | TokenKind::ArrowInOut
                )
            })
        {
            labels.push(token.lexeme.clone());
        }
        cursor += 1;
    }
    labels
}

fn signature_label(tokens: &[Token], declaration: usize) -> Option<String> {
    let mut start = declaration;
    let mut candidates = Vec::new();
    let mut cursor = declaration;
    for _ in 0..3 {
        if let Some(before) = previous(tokens, cursor) {
            candidates.push(before);
            cursor = before;
        } else {
            break;
        }
    }
    if let Some(&keyword) = candidates.iter().rev().find(|&&position| {
        matches!(
            tokens[position].kind,
            TokenKind::Flow
                | TokenKind::Shape
                | TokenKind::Mark
                | TokenKind::Choice
                | TokenKind::Opaque
                | TokenKind::Extern
                | TokenKind::Open
                | TokenKind::Sealed
        )
    }) {
        start = keyword;
    }
    let mut parts = Vec::new();
    let mut cursor = start;
    while cursor < tokens.len() {
        let token = &tokens[cursor];
        if matches!(token.kind, TokenKind::LBrace | TokenKind::Eof) {
            break;
        }
        if !matches!(token.kind, TokenKind::Newline) && !token.lexeme.is_empty() {
            parts.push(token.lexeme.clone());
        }
        cursor += 1;
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(" ");
    let compact = joined
        .replace("( ", "(")
        .replace(" )", ")")
        .replace("[ ", "[")
        .replace(" ]", "]")
        .replace(" :", ":")
        .replace(" ,", ",");
    Some(compact)
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
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
    out.push('"');
    out
}

fn references_json(tokens: &[Token], line: usize, column: usize, uri: &str) -> String {
    let Some(index) = target_token(tokens, line, column) else {
        return "[]".to_string();
    };
    let name = tokens[index].lexeme.as_str();
    let mut locations = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Identifier && token.lexeme == name && meaningful(tokens, i) {
            locations.push(format!(
                "{{\"uri\":{},\"range\":{}}}",
                quote(uri),
                lsp_range(token)
            ));
        }
    }
    format!("[{}]", locations.join(","))
}

fn prepare_rename_json(tokens: &[Token], line: usize, column: usize) -> String {
    let Some(index) = target_token(tokens, line, column) else {
        return "null".to_string();
    };
    let token = &tokens[index];
    if token.kind != TokenKind::Identifier {
        return "null".to_string();
    }
    format!(
        "{{\"range\":{},\"placeholder\":{}}}",
        lsp_range(token),
        quote(&token.lexeme)
    )
}

fn rename_json(tokens: &[Token], line: usize, column: usize, uri: &str, new_name: &str) -> String {
    let Some(index) = target_token(tokens, line, column) else {
        return "null".to_string();
    };
    let token = &tokens[index];
    if token.kind != TokenKind::Identifier || new_name.is_empty() {
        return "null".to_string();
    }
    let name = token.lexeme.as_str();
    let mut edits = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t.kind == TokenKind::Identifier && t.lexeme == name && meaningful(tokens, i) {
            edits.push(format!(
                "{{\"range\":{},\"newText\":{}}}",
                lsp_range(t),
                quote(new_name)
            ));
        }
    }
    format!("{{\"changes\":{{{}:[{}]}}}}", quote(uri), edits.join(","))
}

fn symbol_kind_for_decl(tokens: &[Token], index: usize) -> u32 {
    match declaration_kind(tokens, index) {
        "flow" => 12,     // Function
        "shape" => 23,    // Struct
        "mark" => 10,     // Enum
        "choice" => 10,   // Enum
        "opaque" => 23,   // Struct
        "variable" => 13, // Variable
        "module" => 2,    // Module
        _ => 13,
    }
}

fn document_symbol_json(tokens: &[Token]) -> String {
    let mut symbols = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.kind != TokenKind::Identifier || !is_declaration(tokens, i) {
            continue;
        }
        let kind = symbol_kind_for_decl(tokens, i);
        let range = lsp_range(token);
        let name = quote(&token.lexeme);
        let detail =
            signature_label(tokens, i).unwrap_or_else(|| declaration_kind(tokens, i).to_string());
        symbols.push(format!(
            "{{\"name\":{},\"kind\":{},\"range\":{},\"selectionRange\":{},\"detail\":{}}}",
            name,
            kind,
            range,
            range,
            quote(&detail)
        ));
    }
    format!("[{}]", symbols.join(","))
}

fn semantic_token_type(kind: TokenKind, tokens: &[Token], index: usize) -> Option<u32> {
    match kind {
        TokenKind::Flow
        | TokenKind::Shape
        | TokenKind::Mark
        | TokenKind::Choice
        | TokenKind::Opaque
        | TokenKind::Extern
        | TokenKind::Open
        | TokenKind::Sealed
        | TokenKind::Mut
        | TokenKind::Guard
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::While
        | TokenKind::For
        | TokenKind::In
        | TokenKind::Is
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::Import
        | TokenKind::As
        | TokenKind::Protect
        | TokenKind::Give
        | TokenKind::Clone
        | TokenKind::Alloc
        | TokenKind::Comptime
        | TokenKind::True
        | TokenKind::False
        | TokenKind::And
        | TokenKind::Or
        | TokenKind::Not
        | TokenKind::Quote
        | TokenKind::Unquote
        | TokenKind::Splice
        | TokenKind::ArrowIn
        | TokenKind::ArrowInOut
        | TokenKind::FatArrow => Some(0), // keyword
        TokenKind::Integer | TokenKind::Float => Some(1), // number
        TokenKind::String => Some(2),                     // string
        TokenKind::Identifier => {
            let decl = is_declaration(tokens, index);
            let dk = if decl {
                declaration_kind(tokens, index)
            } else {
                ""
            };
            let looks_type = token_lexeme_starts_upper(tokens, index);
            if decl && matches!(dk, "flow") {
                Some(3) // function
            } else if looks_type {
                Some(4) // type
            } else if decl {
                Some(5) // variable
            } else if next(tokens, index).is_some_and(|n| tokens[n].kind == TokenKind::LParen) {
                Some(3)
            } else {
                Some(5)
            }
        }
        _ => None,
    }
}

fn token_lexeme_starts_upper(tokens: &[Token], index: usize) -> bool {
    tokens[index]
        .lexeme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn semantic_tokens_json(tokens: &[Token], comments: &[Comment]) -> String {
    // Legend: 0 keyword,1 number,2 string,3 function,4 type,5 variable,6 comment
    let mut data: Vec<u32> = Vec::new();
    let mut prev_line: u32 = 0;
    let mut prev_start: u32 = 0;
    let mut entries: Vec<(u32, u32, u32, u32)> = Vec::new(); // line, start, len, type
    for (i, token) in tokens.iter().enumerate() {
        if !meaningful(tokens, i) {
            continue;
        }
        if let Some(ty) = semantic_token_type(token.kind, tokens, i) {
            let line = token.line.saturating_sub(1) as u32;
            let start = token.column.saturating_sub(1) as u32;
            let len = token.lexeme.chars().count() as u32;
            entries.push((line, start, len, ty));
        }
    }
    for comment in comments {
        let line = comment.line.saturating_sub(1) as u32;
        let start = comment.column.saturating_sub(1) as u32;
        let len = comment.text.chars().count() as u32;
        entries.push((line, start, len, 6));
    }
    entries.sort_by_key(|e| (e.0, e.1));
    for (line, start, len, ty) in entries {
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            start - prev_start
        } else {
            start
        };
        data.push(delta_line);
        data.push(delta_start);
        data.push(len);
        data.push(ty);
        data.push(0);
        prev_line = line;
        prev_start = start;
    }
    format!(
        "{{\"data\":[{}]}}",
        data.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn inlay_hint_json(_tokens: &[Token]) -> String {
    // Conservative: no type inference engine here yet.
    "[]".to_string()
}

fn folding_range_json(tokens: &[Token], comments: &[Comment]) -> String {
    let mut ranges: Vec<String> = Vec::new();
    let mut stack: Vec<(TokenKind, usize, usize)> = Vec::new(); // kind, line, col
    for token in tokens {
        match token.kind {
            TokenKind::LBrace | TokenKind::LBracket | TokenKind::LParen => {
                stack.push((
                    token.kind,
                    token.line.saturating_sub(1),
                    token.column.saturating_sub(1),
                ));
            }
            TokenKind::RBrace | TokenKind::RBracket | TokenKind::RParen => {
                let open_kind = match token.kind {
                    TokenKind::RBrace => TokenKind::LBrace,
                    TokenKind::RBracket => TokenKind::LBracket,
                    TokenKind::RParen => TokenKind::LParen,
                    _ => continue,
                };
                if let Some(pos) = stack.iter().rposition(|(k, _, _)| *k == open_kind) {
                    let (_, start_line, _) = stack.remove(pos);
                    let end_line = token.line.saturating_sub(1);
                    if end_line > start_line {
                        ranges.push(format!(
                            "{{\"startLine\":{},\"endLine\":{}}}",
                            start_line, end_line
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    // Comment blocks (consecutive comment lines)
    let mut i = 0usize;
    let mut comment_lines: Vec<usize> = comments
        .iter()
        .filter(|c| !c.trailing)
        .map(|c| c.line.saturating_sub(1))
        .collect();
    comment_lines.sort();
    comment_lines.dedup();
    while i < comment_lines.len() {
        let start = comment_lines[i];
        let mut end = start;
        i += 1;
        while i < comment_lines.len() && comment_lines[i] == end + 1 {
            end = comment_lines[i];
            i += 1;
        }
        if end > start {
            ranges.push(format!(
                "{{\"startLine\":{},\"endLine\":{},\"kind\":\"comment\"}}",
                start, end
            ));
        }
    }
    format!("[{}]", ranges.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"// Loi editor fixture
/// Add two numbers.
open flow add
    <- left: i32
    <- right: i32
    => i32
{
    mut result: i32 <- left + right
    => result
}

flow main => i32 {
    => add(1, 2)
}
"#;

    #[test]
    fn hover_uses_doc_comments_and_declaration_kind() {
        let json = info_json(
            SOURCE,
            "main.loi",
            13,
            8,
            "file:///main.loi",
            InfoKind::Hover,
            "",
        );
        assert!(json.contains("**flow** `add`"), "{json}");
        assert!(json.contains("Add two numbers."), "{json}");
        assert!(json.contains("open flow add <- left: i32"), "{json}");
    }

    #[test]
    fn definition_points_at_the_declaration() {
        let json = info_json(
            SOURCE,
            "main.loi",
            13,
            8,
            "file:///main.loi",
            InfoKind::Definition,
            "",
        );
        assert!(json.contains("\"uri\":\"file:///main.loi\""), "{json}");
        assert!(json.contains("\"line\":2,\"character\":10"), "{json}");
    }

    #[test]
    fn signature_help_reports_the_active_parameter() {
        let json = info_json(
            SOURCE,
            "main.loi",
            13,
            15,
            "file:///main.loi",
            InfoKind::Signature,
            "",
        );
        assert!(json.contains("\"label\":\"open flow add"), "{json}");
        assert!(json.contains("\"label\":\"left\""), "{json}");
        assert!(json.contains("\"activeParameter\":1"), "{json}");
    }

    #[test]
    fn unknown_kinds_return_valid_empty_results() {
        let json = info_json(SOURCE, "main.loi", 99, 1, "", InfoKind::Signature, "");
        assert_eq!(json, empty_result(InfoKind::Signature));
        assert_eq!(empty_result(InfoKind::Hover), "null");
    }

    #[test]
    fn references_and_rename_return_locations_and_edits() {
        let refs = info_json(
            SOURCE,
            "main.loi",
            13,
            8,
            "file:///main.loi",
            InfoKind::References,
            "",
        );
        assert!(refs.contains("\"uri\":\"file:///main.loi\""), "{refs}");
        assert!(refs.contains("\"line\":2"), "{refs}");
        let prep = info_json(
            SOURCE,
            "main.loi",
            13,
            8,
            "file:///main.loi",
            InfoKind::PrepareRename,
            "",
        );
        assert!(prep.contains("\"placeholder\":\"add\""), "{prep}");
        let ren = info_json(
            SOURCE,
            "main.loi",
            13,
            8,
            "file:///main.loi",
            InfoKind::Rename,
            "sum",
        );
        assert!(ren.contains("\"newText\":\"sum\""), "{ren}");
        assert!(ren.contains("\"changes\""), "{ren}");
    }

    #[test]
    fn document_symbol_semantic_tokens_and_folding_ranges() {
        let syms = info_json(
            SOURCE,
            "main.loi",
            1,
            1,
            "file:///main.loi",
            InfoKind::DocumentSymbol,
            "",
        );
        assert!(syms.contains("\"name\":\"add\""), "{syms}");
        assert!(syms.contains("\"name\":\"main\""), "{syms}");
        let toks = info_json(
            SOURCE,
            "main.loi",
            1,
            1,
            "file:///main.loi",
            InfoKind::SemanticTokens,
            "",
        );
        assert!(toks.contains("\"data\":["), "{toks}");
        let folds = info_json(
            SOURCE,
            "main.loi",
            1,
            1,
            "file:///main.loi",
            InfoKind::FoldingRange,
            "",
        );
        assert!(folds.contains("\"startLine\""), "{folds}");
    }
}
