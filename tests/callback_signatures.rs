use loi::{check, generate_c, generate_header, parse, tokenize};

fn diagnostics(source: &str) -> Vec<loi::Diagnostic> {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    check(&program)
}

fn assert_clean(source: &str) {
    let diagnostics = diagnostics(source);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
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

const TYPED_SOURCE: &str = r#"open extern flow apply_one
    <- value: i32
    <- f: callback(i32) => i32
    => i32

open extern flow notify
    <- f: callback(i32) => void

open extern flow zero_arg
    <- f: callback() => i32
    => i32

flow inc
    <- value: i32
    => i32
{
    => value + 1
}

flow show
    <- value: i32
{
    print_int(value)
}

flow answer
    => i32
{
    => 42
}

open flow main
    => i32
{
    got <- apply_one(41, callback(inc))
    if got != 42 { => 1 }
    notify(callback(show))
    if zero_arg(callback(answer)) != 42 { => 2 }
    => 0
}
"#;

#[test]
fn typed_callback_signatures_type_check() {
    assert_clean(TYPED_SOURCE);
}

#[test]
fn fixed_alias_and_explicit_fixed_signature_are_the_same_type() {
    assert_clean(
        r#"extern flow apply
    <- a: i32
    <- b: i32
    <- f: callback
    => i32

extern flow apply_explicit
    <- a: i32
    <- b: i32
    <- f: callback(i32, i32) => i32
    => i32

flow add
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}

flow main
    => i32
{
    f <- callback(add)
    if apply(1, 2, f) != 3 { => 1 }
    if apply_explicit(2, 3, f) != 5 { => 2 }
    => 0
}
"#,
    );
}

#[test]
fn typed_callback_rejects_a_mismatched_flow_signature() {
    assert_message(
        r#"open extern flow apply_one
    <- value: i32
    <- f: callback(i32) => i32
    => i32

flow add
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}

flow main
    => i32
{
    f <- callback(add)
    g <- apply_one(1, f)
    => 0
}
"#,
        "argument 'f' has type callback, expected callback(i32) => i32",
    );
}

#[test]
fn typed_callback_rejects_a_wrong_result_type() {
    assert_message(
        r#"open extern flow notify
    <- f: callback(i32) => void

flow inc
    <- value: i32
    => i32
{
    => value + 1
}

flow main {
    notify(callback(inc))
}
"#,
        "argument 'f' has type callback(i32) => i32, expected callback(i32) => void",
    );
}

#[test]
fn typed_callback_rejects_a_void_parameter() {
    assert_message(
        r#"extern flow bad
    <- f: callback(void) => i32

flow main {
    => 0
}
"#,
        "callback parameter type cannot be void",
    );
}

#[test]
fn typed_callback_rejects_a_non_input_flow_parameter() {
    assert_message(
        r#"open extern flow apply_one
    <- value: i32
    <- f: callback(i32) => i32
    => i32

flow bad
    -> value: i32
    => i32
{
    => 0
}

flow main {
    f <- callback(bad)
}
"#,
        "callback flow 'bad' must have signature callback(i32) => i32",
    );
}

#[test]
fn typed_callbacks_generate_separate_c_typedefs_and_run() {
    let tokens = tokenize(TYPED_SOURCE, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    assert!(check(&program).is_empty(), "source should type-check");
    let c = generate_c(&program, None).expect("source should generate C");

    assert!(
        c.contains("typedef int32_t (*LoiCallback_1_i32_i32)(int32_t);"),
        "missing value callback typedef:\n{c}"
    );
    assert!(
        c.contains("typedef void (*LoiCallback_1_i32_void)(int32_t);"),
        "missing void callback typedef:\n{c}"
    );
    assert!(
        c.contains("typedef int32_t (*LoiCallback_0_i32)(void);"),
        "missing zero-argument callback typedef:\n{c}"
    );
    assert!(
        c.contains("= loi_fn_inc, apply_one"),
        "callback(flow) should emit the flow symbol directly:\n{c}"
    );
    assert!(
        c.contains("= loi_fn_show, notify"),
        "void callback should emit the flow symbol directly:\n{c}"
    );

    let harness = r#"#include <stdint.h>
typedef int32_t (*LoiCallback_1_i32_i32)(int32_t);
typedef void (*LoiCallback_1_i32_void)(int32_t);
typedef int32_t (*LoiCallback_0_i32)(void);

int32_t apply_one(int32_t value, LoiCallback_1_i32_i32 f) {
    return f(value);
}
void notify(LoiCallback_1_i32_void f) {
    f(7);
}
int32_t zero_arg(LoiCallback_0_i32 f) {
    return f();
}
"#;
    let dir = std::env::temp_dir()
        .join("opencode")
        .join(format!("loi-callback-sigs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test directory should be creatable");
    let generated_path = dir.join("generated.c");
    let harness_path = dir.join("harness.c");
    let binary = dir.join("program");
    std::fs::write(&generated_path, &c).expect("generated C should be writable");
    std::fs::write(&harness_path, harness).expect("harness should be writable");
    let output = std::process::Command::new("cc")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg(&generated_path)
        .arg(&harness_path)
        .arg("-lm")
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("cc should run");
    assert!(
        output.status.success(),
        "typed callback program should compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = std::process::Command::new(&binary)
        .output()
        .expect("program should run");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "7");
}

#[test]
fn typed_callbacks_header_exposes_every_typedef() {
    let tokens = tokenize(TYPED_SOURCE, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    assert!(check(&program).is_empty(), "source should type-check");
    let h = generate_header(&program, None).expect("source should generate a header");

    for typedef in [
        "typedef int32_t (*LoiCallback_1_i32_i32)(int32_t);",
        "typedef void (*LoiCallback_1_i32_void)(int32_t);",
        "typedef int32_t (*LoiCallback_0_i32)(void);",
    ] {
        assert!(
            h.contains(typedef),
            "header should contain {typedef:?}:\n{h}"
        );
    }
    assert!(
        h.contains("LoiCallback_1_i32_i32") && h.contains("LoiCallback_1_i32_void"),
        "header should expose the typed callback names:\n{h}"
    );
}

#[test]
fn typed_callbacks_are_copyable_but_not_comparable() {
    assert_message(
        r#"flow add
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}

flow main
    => i32
{
    a <- callback(add)
    b <- a
    if a == b { => 1 }
    => 0
}
"#,
        "cannot compare callback with callback",
    );
    assert_clean(
        r#"extern flow apply
    <- a: i32
    <- b: i32
    <- f: callback
    => i32

flow add
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}

flow main
    => i32
{
    a <- callback(add)
    b <- a
    if apply(1, 2, b) != 3 { => 1 }
    => 0
}
"#,
    );
}
