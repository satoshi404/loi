use loi::TokenKind;
use loi::{tokenize, tokenize_with_comments};

fn reference_source() -> &'static str {
    include_str!("../examples/particles.loi")
}

#[test]
fn lexes_the_reference_program() {
    let tokens = tokenize(reference_source(), "examples/particles.loi")
        .expect("the reference program must lex");

    assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
    assert_eq!(tokens.len(), 651, "650 source tokens plus EOF");

    let kinds: Vec<TokenKind> = tokens.iter().map(|token| token.kind).collect();
    for expected in [
        TokenKind::Shape,
        TokenKind::Flow,
        TokenKind::Open,
        TokenKind::ArrowIn,
        TokenKind::Arrow,
        TokenKind::ArrowInOut,
        TokenKind::FatArrow,
        TokenKind::Protect,
        TokenKind::Alloc,
        TokenKind::Give,
        TokenKind::Clone,
        TokenKind::For,
        TokenKind::DotDot,
        TokenKind::PlusEq,
        TokenKind::Mut,
    ] {
        assert!(kinds.contains(&expected), "missing {expected:?}");
    }

    assert!(
        tokens[..tokens.len() - 1]
            .iter()
            .all(|token| !token.lexeme.is_empty())
    );
}

#[test]
fn qualified_names_lex_as_single_identifiers() {
    let tokens = tokenize("m::sum geo::Color.Red math::Vec<i32>", "<test>").unwrap();
    let identifiers: Vec<&str> = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Identifier)
        .map(|token| token.lexeme.as_str())
        .collect();

    assert_eq!(
        identifiers,
        vec!["m::sum", "geo::Color", "Red", "math::Vec", "i32"]
    );
    let kinds: Vec<TokenKind> = tokens.iter().map(|token| token.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Dot,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Lt,
            TokenKind::Identifier,
            TokenKind::Gt,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn all_flow_keywords_are_recognized() {
    let source = "import shape flow open sealed mut in give clone protect alloc \
                  comptime if else while for continue break and or not as true false";
    let tokens = tokenize(source, "<test>").unwrap();
    let kinds: Vec<TokenKind> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Import,
            TokenKind::Shape,
            TokenKind::Flow,
            TokenKind::Open,
            TokenKind::Sealed,
            TokenKind::Mut,
            TokenKind::In,
            TokenKind::Give,
            TokenKind::Clone,
            TokenKind::Protect,
            TokenKind::Alloc,
            TokenKind::Comptime,
            TokenKind::If,
            TokenKind::Else,
            TokenKind::While,
            TokenKind::For,
            TokenKind::Continue,
            TokenKind::Break,
            TokenKind::And,
            TokenKind::Or,
            TokenKind::Not,
            TokenKind::As,
            TokenKind::True,
            TokenKind::False,
            TokenKind::Eof,
        ]
    );
}

#[test]
fn flow_syntax_keywords_are_recognized() {
    let source = "shape flow open sealed mut give clone";
    let tokens = tokenize(source, "<test>").unwrap();
    let kinds: Vec<TokenKind> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
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
fn mark_choice_and_is_are_keywords() {
    let source = "mark choice is";
    let tokens = tokenize(source, "<test>").unwrap();
    let kinds: Vec<TokenKind> = tokens.into_iter().map(|token| token.kind).collect();

    assert_eq!(
        kinds,
        vec![
            TokenKind::Mark,
            TokenKind::Choice,
            TokenKind::Is,
            TokenKind::Eof,
        ]
    );
    assert_eq!(TokenKind::Mark.name(), "mark");
    assert_eq!(TokenKind::Choice.name(), "choice");
    assert_eq!(TokenKind::Is.name(), "is");
}

#[test]
fn mark_choice_and_is_can_be_part_of_longer_identifiers() {
    let source = "marker chosen island";
    let tokens = tokenize(source, "<test>").unwrap();
    assert!(
        tokens[..tokens.len() - 1]
            .iter()
            .all(|token| token.kind == TokenKind::Identifier)
    );
}

#[test]
fn decodes_string_literal_escapes() {
    let tokens = tokenize("\"a\\n\\t\\r\\0\\\\\\\"\\x41\"", "<test>").unwrap();
    assert_eq!(tokens[0].kind, TokenKind::String);
    assert_eq!(tokens[0].lexeme, "a\n\t\r\0\\\"A");
}

#[test]
fn rejects_unknown_and_non_ascii_string_escapes() {
    let unknown = tokenize("\"\\q\"", "<test>").unwrap_err();
    assert!(unknown.message.contains("unknown string escape"));

    let non_ascii = tokenize("\"\\xFF\"", "<test>").unwrap_err();
    assert!(non_ascii.message.contains("ASCII byte"));

    let short = tokenize("\"\\x4\"", "<test>").unwrap_err();
    assert!(short.message.contains("two hexadecimal digits"));

    let unterminated = tokenize("\"oops", "<test>").unwrap_err();
    assert!(unterminated.message.contains("unterminated string literal"));
}

#[test]
fn tokenize_with_comments_keeps_comment_positions() {
    let source = "// header\nflow main { // trailing\n    => 0\n}\n";
    let (tokens, comments) = tokenize_with_comments(source, "<test>").expect("source should lex");

    assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].text, "// header");
    assert_eq!((comments[0].line, comments[0].column), (1, 1));
    assert!(!comments[0].trailing);
    assert_eq!(comments[1].text, "// trailing");
    assert_eq!((comments[1].line, comments[1].column), (2, 13));
    assert!(comments[1].trailing);

    // The ordinary tokenizer still discards comments.
    let tokens = tokenize(source, "<test>").expect("source should lex");
    assert!(tokens.iter().all(|token| !token.lexeme.starts_with("//")));
}

#[test]
fn rejects_malformed_radix_literals() {
    assert!(
        tokenize("0x", "<test>").is_err(),
        "a bare 0x prefix must be a lexical error"
    );
    assert!(
        tokenize("0b2", "<test>").is_err(),
        "a digit outside base 2 must be a lexical error"
    );
}

#[test]
fn string_escape_errors_point_at_the_backslash() {
    let error = tokenize("\"a\\q\"", "<test>").unwrap_err();
    assert_eq!(
        (error.line, error.column, error.end_column),
        (1, 3, 4),
        "the diagnostic must point at the backslash: {error:?}"
    );
}
