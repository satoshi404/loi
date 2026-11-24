use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-const-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn assert_success(label: &str, output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_source(dir: &Path, source: &str) -> PathBuf {
    let path = dir.join("main.loi");
    fs::write(&path, source).expect("source should be writable");
    path
}

fn build_and_run(source: &str, name: &str) -> String {
    let dir = test_dir(name);
    let source = write_source(&dir, source);
    let exe = dir.join("program");
    let build = Command::new(loi())
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("loi build should run");
    assert_success("build", &build);
    let run = Command::new(exe).output().expect("program should run");
    assert_success("run", &run);
    String::from_utf8(run.stdout).expect("program output should be UTF-8")
}

fn generated_c(source: &str, name: &str) -> String {
    let dir = test_dir(name);
    let source = write_source(&dir, source);
    let c = dir.join("out.c");
    let generated = Command::new(loi())
        .arg("gen")
        .arg(&source)
        .arg("-o")
        .arg(&c)
        .output()
        .expect("loi gen should run");
    assert_success("gen", &generated);
    fs::read_to_string(c).expect("generated C should be readable")
}

fn assert_strict_c11(source: &str, name: &str) {
    let c = generated_c(source, name);
    let path =
        std::env::temp_dir().join(format!("loi-const-strict-{}-{name}.c", std::process::id()));
    fs::write(&path, c).expect("generated C should be writable");
    let output = Command::new("cc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
        ])
        .arg(&path)
        .output()
        .expect("C compiler should run");
    assert_success("strict C11 syntax check", &output);
}

fn preamble() -> &'static str {
    r#"
open mark Mode [ Fast, Safe ]
open choice Result [ Ok: i32, Err: i64 ]

open shape Config [
    mode: Mode = Mode.Safe
    fallback: Result = Result.Err(-11)
    u8_value: u8 = 200 + 55
    i16_value: i16 = -32768
    f32_value: f32 = 0.1
]

GlobalMode: Mode <- Mode.Fast
GlobalResult: Result <- Result.Ok(42)
ConfigValue: Config <- Config[]

flow digit
    <- value: bool
    => i32
{
    if value {
        => 1
    }
    => 0
}
"#
}

#[test]
fn mark_and_choice_constants_match_runtime_values() {
    let source = format!(
        r#"{}
open flow main
    => i32
{{
    cm <- comptime Mode.Fast
    cv <- comptime Result.Ok(7)
    ce <- comptime Result.Err(-9)
    gm <- clone(GlobalMode)
    gr <- clone(GlobalResult)
    config <- Config[]
    rm <- Mode.Fast
    rv <- Result.Ok(7)
    printf("%d %d %d %d %d %d %d %d %d %d %d %d %d\n",
        digit(cm == Mode.Fast), digit(gm == Mode.Fast), cv.Ok, gr.Ok, digit(ce.Err == -9),
        digit(config.mode == Mode.Safe), digit(config.fallback.Err == -11),
        digit(config.u8_value as u32 == 255), digit(config.i16_value as i32 == -32768),
        digit(rm == Mode.Fast), rv.Ok, digit(ce.Err == -9), digit(ce is Result.Err))
    => 0
}}
"#,
        preamble()
    );
    assert_eq!(
        build_and_run(&source, "runtime-agreement"),
        "1 1 7 42 1 1 1 1 1 1 7 1 1\n"
    );
}

#[test]
fn comptime_operations_support_marks_and_choices() {
    let source = format!(
        r#"{}
open flow main
    => i32
{{
    same_mark <- comptime GlobalMode == Mode.Fast
    same_choice <- comptime GlobalResult is Result.Ok
    nested <- comptime Config[].fallback is Result.Err
    chosen <- comptime GlobalResult.Ok
    printf("%d %d %d %d\n", digit(same_mark), digit(same_choice), digit(nested), chosen)
    => 0
}}
"#,
        preamble()
    );
    assert_eq!(build_and_run(&source, "operations"), "1 1 1 42\n");
}

#[test]
fn mark_and_choice_constants_emit_static_c_initializers() {
    let source = format!(
        r#"{}
open flow main
    => i32
{{
    printf("%d\n", GlobalResult.Ok)
    => 0
}}
"#,
        preamble()
    );
    let c = generated_c(&source, "static-init");
    assert!(
        c.contains("loi_t_Mode loi_g_GlobalMode = loi_m_Mode_Fast;"),
        "global mark initializer was not emitted:\n{c}"
    );
    assert!(
        c.contains("loi_t_Result loi_g_GlobalResult = { 0u, { .loi_p_Ok = 42 } };"),
        "global choice initializer was not emitted:\n{c}"
    );
    assert!(
        c.contains(".loi_f_mode = loi_m_Mode_Safe")
            && c.contains(".loi_f_fallback = { 1u, { .loi_p_Err = -11 } }"),
        "shape default choice initializer was not emitted:\n{c}"
    );
    assert_strict_c11(&source, "static-init-strict");
}

#[test]
fn mark_variant_names_are_rejected_as_variables() {
    let source = r#"
open mark Mode [ Fast, Safe ]
open flow main
    => i32
{
    m <- Fast
    printf("%u\n", m as u32)
    => 0
}
"#;
    let dir = test_dir("bare-variant");
    let source = write_source(&dir, source);
    let check = Command::new(loi())
        .arg("check")
        .arg(&source)
        .output()
        .expect("loi check should run");
    assert!(
        !check.status.success(),
        "bare mark variant should not compile"
    );
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains("unknown variable 'Fast'"),
        "unexpected diagnostic:\n{stderr}"
    );
}

#[test]
fn bad_constant_subexpression_gets_an_exact_span() {
    let source = r#"
open choice Result [ Ok: i32, Err: i32 ]
Bad: Result <- Result.Ok(runtime_value())

flow runtime_value
    => i32
{
    => 3
}

open flow main
    => i32
{
    r <- Bad
    printf("%d\n", r.Ok)
    => 0
}
"#;
    let dir = test_dir("bad-span");
    let source = write_source(&dir, source);
    let check = Command::new(loi())
        .arg("check")
        .arg("--error-format")
        .arg("short")
        .arg("--color")
        .arg("never")
        .arg(&source)
        .output()
        .expect("loi check should run");
    assert!(
        !check.status.success(),
        "impure constructor should not compile"
    );
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains(":3:26: initializer for global 'Bad' must be a compile-time constant: call to 'runtime_value' is not allowed in a constant"),
        "diagnostic should underline the offending call:\n{stderr}"
    );
    assert!(
        !stderr.contains("Result.Ok"),
        "diagnostic should not blame the choice type:\n{stderr}"
    );
}
