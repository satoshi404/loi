//! `loi complete` supplies editor candidates from a raw source buffer.

use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn check(source: &str, line: usize, column: usize) -> Output {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loi-completion-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.loi");
    fs::write(&path, source).expect("write source");
    Command::new(loi())
        .arg("complete")
        .arg(&path)
        .arg("--line")
        .arg(line.to_string())
        .arg("--column")
        .arg(column.to_string())
        .output()
        .expect("loi complete should run")
}

fn labels(output: &Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "complete should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "complete must not write diagnostics: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = String::from_utf8(output.stdout.clone()).expect("UTF-8 JSON");
    let trimmed = json.trim_end();
    assert!(
        trimmed.starts_with('[') && trimmed.ends_with(']'),
        "bad JSON: {json}"
    );
    json.split("\"label\":\"")
        .skip(1)
        .filter_map(|entry| entry.split('"').next())
        .map(str::to_string)
        .collect()
}

#[test]
fn completion_offers_keywords_intrinsics_and_declarations() {
    let source = "flow main => i32 {\n    mut alpha: i32 <- 1\n    \n}\n";
    let items = labels(&check(source, 3, 5));
    for label in ["flow", "mut", "protect", "print_int", "alpha"] {
        assert!(
            items.iter().any(|item| item == label),
            "missing {label}: {items:?}"
        );
    }
}

#[test]
fn completion_filters_by_prefix_and_declaration_position() {
    let source =
        "flow main => i32 {\n    mut alpha: i32 <- 1\n    mut beta: i32 <- 2\n    => alp\n}\n";
    let items = labels(&check(source, 4, 11));
    assert!(
        items.iter().any(|item| item == "alpha"),
        "expected alpha: {items:?}"
    );
    assert!(
        !items.iter().any(|item| item == "beta"),
        "unexpected beta: {items:?}"
    );
}

#[test]
fn completion_uses_an_import_alias_as_a_module_name() {
    let source = "import \"lib/std.loi\" as standard\n\nflow main => i32 {\n    => sta\n}\n";
    let output = check(source, 4, 10);
    let json = String::from_utf8(output.stdout.clone()).expect("UTF-8 JSON");
    let items = labels(&output);
    assert!(
        items.iter().any(|item| item == "standard"),
        "expected alias: {items:?}"
    );
    assert!(
        json.contains("\"kind\":8"),
        "module kind should be 8: {json}"
    );
}

#[test]
fn completion_works_on_an_unfinished_expression() {
    let source = "flow main => i32 {\n    => pr\n}\n";
    let items = labels(&check(source, 2, 10));
    for label in ["print", "print_int", "print_float", "protect"] {
        assert!(
            items.iter().any(|item| item == label),
            "missing {label}: {items:?}"
        );
    }
}

#[test]
fn completion_requires_a_valid_one_based_position() {
    let source = "flow main => i32 {\n    => 0\n}\n";
    let dir = std::env::temp_dir().join(format!("loi-completion-args-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.loi");
    fs::write(&path, source).expect("write source");

    let missing = Command::new(loi())
        .arg("complete")
        .arg(&path)
        .output()
        .expect("loi complete should run");
    assert_eq!(missing.status.code(), Some(2));

    for value in ["0", "nope"] {
        let invalid = Command::new(loi())
            .arg("complete")
            .arg(&path)
            .arg("--line")
            .arg(value)
            .arg("--column")
            .arg("1")
            .output()
            .expect("loi complete should run");
        assert_eq!(
            invalid.status.code(),
            Some(2),
            "line {value} should be rejected"
        );
    }
}

#[test]
fn line_and_column_are_only_valid_for_completion() {
    let dir = std::env::temp_dir().join(format!("loi-completion-mode-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.loi");
    fs::write(&path, "flow main => i32 {\n    => 0\n}\n").expect("write source");
    let output = Command::new(loi())
        .arg("check")
        .arg(&path)
        .arg("--line")
        .arg("1")
        .output()
        .expect("loi check should run");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("only supported by 'complete'"),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn completion_json_is_deterministic() {
    let source = "flow main => i32 {\n    mut alpha: i32 <- 1\n    => al\n}\n";
    let first = check(source, 3, 9);
    let second = check(source, 3, 9);
    assert_eq!(
        first.stdout, second.stdout,
        "completion output should be stable"
    );
}

#[test]
fn completion_offers_every_canonical_keyword() {
    // Use a distinct prefix per keyword: the empty prefix is capped at 50
    // items, so it cannot prove that the tail of the canonical set is present.
    for label in ["clone", "quote", "splice", "unquote"] {
        let prefix = &label[..3];
        let source = format!("flow main => i32 {{\n    => {prefix}\n}}\n");
        let items = labels(&check(&source, 2, 8 + prefix.len()));
        assert!(
            items.iter().any(|item| item == label),
            "missing canonical keyword {label}: {items:?}"
        );
    }
}
