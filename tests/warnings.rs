//! Warnings are diagnostics with severity `warning` and must not fail `loi check`.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn check(source: &str) -> std::process::Output {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loi-warning-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.loi");
    std::fs::write(&path, source).expect("write source");
    Command::new(loi())
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg("--color")
        .arg("never")
        .arg(&path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("loi should run")
}

#[test]
fn unused_local_is_a_warning_and_does_not_fail_check() {
    let output = check("flow main => i32 {\n    mut unused: i32 <- 1\n    => 0\n}\n");
    assert!(
        output.status.success(),
        "warnings must not fail the check: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "check must not write to stdout");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 diagnostics");
    assert!(
        stderr.contains("\"severity\":\"warning\""),
        "expected warning severity, got {stderr}"
    );
    assert!(
        stderr.contains("unused variable 'unused'"),
        "expected unused-variable warning, got {stderr}"
    );
}

#[test]
fn used_local_has_no_warning() {
    let output = check("flow main => i32 {\n    mut used: i32 <- 1\n    => used\n}\n");
    assert!(
        output.status.success(),
        "clean source should pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "[]\n",
        "clean JSON check should emit an empty array"
    );
}

#[test]
fn discard_binding_has_no_warning() {
    let output = check("flow main => i32 {\n    _ <- 1\n    => 0\n}\n");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "[]\n",
        "`_` is a deliberate discard and JSON still reports an empty array"
    );
}

#[test]
fn imported_modules_do_not_produce_warnings_in_the_checked_file() {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loi-warning-import-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(
        dir.join("helper.loi"),
        "open flow helper => i32 {\n    for i in 0..3 { }\n    => 0\n}\n",
    )
    .expect("write helper");
    let main = dir.join("main.loi");
    std::fs::write(
        &main,
        "import \"helper.loi\"\n\nflow main => i32 {\n    => helper()\n}\n",
    )
    .expect("write source");

    let output = Command::new(loi())
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg("--color")
        .arg("never")
        .arg(&main)
        .output()
        .expect("loi should run");

    assert!(
        output.status.success(),
        "clean imports should pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "[]\n",
        "imported modules must not add warnings to the checked JSON result"
    );
}

#[test]
fn warnings_in_the_checked_file_with_imports_are_still_reported() {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loi-warning-import-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(
        dir.join("helper.loi"),
        "open flow helper => i32 {\n    => 0\n}\n",
    )
    .expect("write helper");
    let main = dir.join("main.loi");
    std::fs::write(
        &main,
        "import \"helper.loi\"\n\nflow main => i32 {\n    mut unused: i32 <- helper()\n    => 0\n}\n",
    )
    .expect("write source");

    let output = Command::new(loi())
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg("--color")
        .arg("never")
        .arg(&main)
        .output()
        .expect("loi should run");

    let stderr = String::from_utf8(output.stderr).expect("UTF-8 diagnostics");
    assert!(
        output.status.success(),
        "warnings must not fail the check: {stderr}"
    );
    assert!(
        stderr.contains("unused variable 'unused'"),
        "the checked file's own warning should survive: {stderr}"
    );
    assert!(
        stderr.contains(&format!("\"file\":\"{}\"", main.display())),
        "warning should carry the checked file: {stderr}"
    );
}
