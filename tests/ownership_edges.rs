//! Edge cases for contracts that were already documented but had no dedicated
//! regression: definite initialization, conservative joins, early loop exits
//! through `continue`, and view escape from a loop body.

use loi::{check, parse, tokenize};

fn diagnostics(source: &str) -> Vec<loi::Diagnostic> {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    check(&program)
}

fn assert_message(source: &str, expected: &str) {
    let messages = diagnostics(source)
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "expected a diagnostic containing {expected:?}, got {messages:#?}"
    );
}

#[test]
fn give_from_uninitialized_composite_field_is_rejected() {
    assert_message(
        "shape Pair [\n    left: u64\n    right: u64\n]\n\nflow f {\n    mut p: Pair\n    _ <- give(p.left)\n}\n",
        "cannot give from uninitialized variable 'p'",
    );
}

#[test]
fn give_before_continue_inside_an_if_poisons_the_value_after_the_loop() {
    assert_message(
        "shape Pair [\n    left: [u64]\n    right: u64\n]\n\nflow f <- n: u64 => u64 {\n    mut p <- Pair[ left: [u64: 1], right: 2 ]\n    for i in 0..n {\n        if i == 1 {\n            _ <- give(p.left)\n            continue\n        }\n    }\n    => p.left.len\n}\n",
        "use of moved value",
    );
}

#[test]
fn branch_join_keeps_a_move_from_a_branch_without_else() {
    assert_message(
        "shape Pair [\n    left: [u64]\n    right: u64\n]\n\nflow f <- c: bool => u64 {\n    mut p <- Pair[ left: [u64: 1], right: 2 ]\n    if c { _ <- give(p.left) }\n    => p.left.len\n}\n",
        "use of moved value",
    );
}

#[test]
fn view_cannot_escape_a_loop_body() {
    assert_message(
        "flow main => i32 {\n    mut escaped: [u8]\n    for i in 0..1 {\n        data: [u8] <- [u8: 1, 2]\n        escaped = view(data, 0, 1)\n    }\n    => escaped.len as i32\n}\n",
        "escapes the block",
    );
}
