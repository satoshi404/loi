use std::fs;
use std::process::Command;

use loi::{check, generate_c, parse, tokenize};

fn diagnostics(source: &str) -> Vec<loi::Diagnostic> {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    check(&program)
}

fn messages(source: &str) -> Vec<String> {
    diagnostics(source)
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn assert_clean(source: &str) {
    let diagnostics = diagnostics(source);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
}

fn assert_message(source: &str, expected: &str) {
    let messages = messages(source);
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "expected a diagnostic containing {expected:?}, got {messages:#?}"
    );
}

fn generated(source: &str) -> String {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    let diagnostics = check(&program);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
    generate_c(&program, None).expect("source should generate C")
}

const RESOURCE: &str = "shape File [\n    fd: i32\n]\n\nmut released: i32 <- 0\n\nsealed flow file_release\n    <-> file: File\n{\n    released = released + 1\n}\n\nopen flow file_open\n    => File\n{\n    => File[ fd: 7 ]\n}\n";

#[test]
fn guard_lexes_as_a_keyword() {
    let tokens = tokenize("guard f <- file_open() {}", "<test>").expect("source should lex");
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == loi::TokenKind::Guard)
    );
}

#[test]
fn guard_releases_on_normal_exit() {
    let source = format!(
        "{RESOURCE}\nopen flow use_guard\n    => i32\n{{\n    guard f <- file_open() {{\n        if f.fd != 7 {{ => 1 }}\n    }}\n    => released\n}}\n"
    );
    assert_clean(&source);
    let c = generated(&source);
    assert!(
        c.contains("loi_fn_file_release(&(loi_v_f))"),
        "missing synthesized release call in:\n{c}"
    );
}

#[test]
fn guard_releases_on_return_with_an_explicit_type() {
    let source = format!(
        "{RESOURCE}\nopen flow use_guard\n    => i32\n{{\n    guard f: File <- file_open() {{\n        => released + 100\n    }}\n    => 999\n}}\n"
    );
    assert_clean(&source);
    let c = generated(&source);
    assert!(
        c.contains("loi_fn_file_release(&(loi_v_f))"),
        "missing synthesized release call in:\n{c}"
    );
}

#[test]
fn guard_with_no_release_flow_is_rejected() {
    let source = "shape File [\n    fd: i32\n]\n\nopen flow file_open\n    => File\n{\n    => File[ fd: 7 ]\n}\n\nopen flow main\n    => i32\n{\n    guard f <- file_open() {\n    }\n    => 0\n}\n";
    assert_message(source, "has no resource release flow");
}

#[test]
fn guard_with_a_wrong_release_signature_is_rejected() {
    let source = "shape File [\n    fd: i32\n]\n\nsealed flow file_release\n    <- file: File\n{\n}\n\nopen flow file_open\n    => File\n{\n    => File[ fd: 7 ]\n}\n\nopen flow main\n    => i32\n{\n    guard f <- file_open() {\n    }\n    => 0\n}\n";
    assert_message(source, "invalid resource release flow");
}

#[test]
fn guard_resource_cannot_be_moved_out() {
    let source = format!(
        "{RESOURCE}\nopen flow main\n    => i32\n{{\n    guard f <- file_open() {{\n        other <- give(f)\n    }}\n    => 0\n}}\n"
    );
    assert_message(&source, "cannot move guarded resource 'f' out of its guard");
}

#[test]
fn guard_allows_returning_a_scalar_field() {
    let source = format!(
        "{RESOURCE}\nopen flow main\n    => i32\n{{\n    guard f <- file_open() {{\n        => f.fd\n    }}\n    => 0\n}}\n"
    );
    // Returning a scalar field copies the value; the resource itself stays in
    // the guard and is still released on this exit path.
    assert_clean(&source);
}

#[test]
fn guard_rejects_returning_the_resource_itself() {
    let source = "shape File [\n    fd: i32\n]\n\nsealed flow file_release\n    <-> file: File\n{\n}\n\nopen flow file_open\n    => File\n{\n    => File[ fd: 7 ]\n}\n\nopen flow borrow\n    => File\n{\n    guard f <- file_open() {\n        => f\n    }\n    => File[ fd: 0 ]\n}\n\nopen flow main\n    => i32\n{\n    => 0\n}\n";
    assert_message(source, "cannot return guarded resource 'f'");
}

#[test]
fn explicit_release_inside_a_guard_is_rejected() {
    let source = format!(
        "{RESOURCE}\nopen flow main\n    => i32\n{{\n    guard f <- file_open() {{\n        file_release(<-> f)\n    }}\n    => 0\n}}\n"
    );
    assert_message(&source, "released automatically by 'guard'");
}

#[test]
fn guard_resource_cannot_be_a_non_resource_type() {
    let source = "open flow main\n    => i32\n{\n    guard n <- 42 {\n    }\n    => 0\n}\n";
    assert_message(source, "guard requires a named shape with a release flow");
}

/// Compile the generated C together with a harness and return its exit code.
/// `None` means no C compiler was available.
fn run_with_harness(name: &str, source: &str, harness: &str) -> Option<i32> {
    if Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| !output.status.success())
        .unwrap_or(true)
    {
        eprintln!("skipping {name}: no C compiler found");
        return None;
    }

    let dir = std::env::temp_dir().join(format!("loi-guard-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    let generated_path = dir.join("generated.c");
    let harness_path = dir.join("harness.c");
    let executable = dir.join("program");
    fs::write(&generated_path, source).expect("generated C should be writable");
    fs::write(
        &harness_path,
        format!("#include \"{}\"\n\n{harness}\n", generated_path.display()),
    )
    .expect("harness should be writable");

    let compile = Command::new("cc")
        .arg("-std=c11")
        .arg("-pedantic-errors")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-o")
        .arg(&executable)
        .arg(&harness_path)
        .arg("-lm")
        .output()
        .expect("cc should run");
    assert!(
        compile.status.success(),
        "cc failed for {name}:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(&executable)
        .output()
        .expect("compiled program should run");
    let _ = fs::remove_dir_all(&dir);
    run.status.code()
}

#[test]
fn compiled_guard_releases_on_every_exit_path_in_lifo_order() {
    // Two resources log their release order into a global integer, where each
    // release appends its digit: 1 for File, 2 for Socket.
    let source = "shape File [\n    fd: i32\n]\n\nshape Socket [\n    fd: i32\n]\n\nmut events: i32 <- 0\n\nsealed flow file_release\n    <-> f: File\n{\n    events = events * 10 + 1\n}\n\nsealed flow socket_release\n    <-> s: Socket\n{\n    events = events * 10 + 2\n}\n\nopen flow file_open\n    => File\n{\n    => File[ fd: 7 ]\n}\n\nopen flow socket_open\n    => Socket\n{\n    => Socket[ fd: 8 ]\n}\n\nopen flow nested\n    => i32\n{\n    guard f <- file_open() {\n        guard s <- socket_open() {\n        }\n    }\n    => events\n}\n\nopen flow early\n    => i32\n{\n    guard f <- file_open() {\n        => 42\n    }\n    => 0\n}\n\nopen flow loops\n    => i32\n{\n    guard f <- file_open() {\n        for i in 0..3 {\n            guard s <- socket_open() {\n                if i == 1 { continue }\n                if i == 2 { break }\n            }\n        }\n    }\n    => 0\n}\n";
    let harness = r#"
int main(void) {
    (void)loi_fn_nested();
    if (loi_g_events != 21) return 10;   /* socket then file */
    loi_g_events = 0;
    if (loi_fn_early() != 42) return 11;
    if (loi_g_events != 1) return 12;    /* released before return */
    loi_g_events = 0;
    (void)loi_fn_loops();
    if (loi_g_events != 2221) return 13; /* three inner, then outer */
    return 0;
}
"#;
    let code = run_with_harness("exits", &generated(source), harness);
    if let Some(code) = code {
        assert_eq!(code, 0, "guard exit paths should release in LIFO order");
    }
}

#[test]
fn release_runs_before_the_enclosing_protect_is_restored_on_return() {
    let source = "shape Buffer [\n    data: [u8]\n]\n\nsealed flow buffer_release\n    <-> b: Buffer\n{\n    print_int(b.data[0] as i32)\n}\n\nopen flow main\n    => i32\n{\n    protect {\n        mut data: [u8]\n        alloc(-> data, u8, 2)\n        data[0] = 9\n        guard b <- Buffer[ data: give(data) ] {\n            => b.data[0] as i32\n        }\n    }\n    => 0\n}\n";
    let c = generated(source);
    let release = c
        .find("loi_fn_buffer_release")
        .expect("release call should be generated");
    let restore = c
        .find("loi_arena_top = loi_tmp_arena")
        .expect("protect restore should be generated");
    assert!(
        release < restore,
        "release must run while the resource's slices are still live:\n{c}"
    );
}

#[test]
fn guard_accepts_a_generic_shape_type_after_monomorphization() {
    assert_clean(
        "shape Box<T> { value: T }\n\nsealed flow box__i32_release\n    <-> box: Box<i32>\n{\n}\n\nflow make_box<T>(<- x: T) => Box<T> {\n    => Box<T> { value: x }\n}\n\nflow use_box<T>(<- x: T) {\n    guard b: Box<T> <- make_box<T>(x) {\n        print_int(b.value)\n    }\n}\n\nflow main {\n    use_box<i32>(1)\n}\n",
    );
}
