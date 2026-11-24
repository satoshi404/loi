//! `loi lsp-info --kind semanticTokens` must classify canonical keywords,
//! including the comptime trio `quote`, `unquote` and `splice`.

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

const SOURCE: &str = "comptime flow make\n    => Code\n{\n    => quote {\n        print_int(unquote(1))\n    }\n}\n\nflow main {\n    splice make()\n}\n";

fn tokens() -> Vec<(usize, usize, usize, u32)> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loi-semantic-tokens-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.loi");
    fs::write(&path, SOURCE).expect("write source");
    let output = Command::new(env!("CARGO_BIN_EXE_loi"))
        .arg("lsp-info")
        .arg(&path)
        .arg("--kind")
        .arg("semanticTokens")
        .output()
        .expect("loi lsp-info should run");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "lsp-info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 output");
    let body = text
        .split("\"data\":[")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_else(|| panic!("unexpected semanticTokens output: {text}"));
    let numbers = body
        .split(',')
        .map(|part| part.trim().parse::<usize>().expect("number"))
        .collect::<Vec<_>>();
    assert_eq!(numbers.len() % 5, 0, "incomplete token tuple: {text}");
    let mut line = 0usize;
    let mut column = 0usize;
    numbers
        .chunks(5)
        .map(|chunk| {
            line += chunk[0];
            column = if chunk[0] == 0 {
                column + chunk[1]
            } else {
                chunk[1]
            };
            (line, column, chunk[2], chunk[3] as u32)
        })
        .collect()
}

/// Positions are the zero-based line/character pairs used by LSP.
fn assert_keyword_at(
    line: usize,
    column: usize,
    word: &str,
    tokens: &[(usize, usize, usize, u32)],
) {
    let found = tokens.iter().any(|(token_line, token_column, len, ty)| {
        *ty == 0 && *token_line == line && *token_column <= column && column < *token_column + *len
    });
    assert!(
        found,
        "{word} at {line}:{column} was not classified as a keyword: {tokens:?}"
    );
}

#[test]
fn comptime_keywords_are_semantic_keywords() {
    let tokens = tokens();
    assert_keyword_at(0, 0, "comptime", &tokens);
    assert_keyword_at(3, 7, "quote", &tokens);
    assert_keyword_at(4, 18, "unquote", &tokens);
    assert_keyword_at(9, 4, "splice", &tokens);
}
