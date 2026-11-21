use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const PROGRAM: &str = r#"flow fact
    <- n: i32
    => i32
{
    if n <= 1 {
        => 1
    }
    => n * fact(n - 1)
}

open flow main
    => i32
{
    mut total: i32 <- 0
    mut i: i32 <- 0
    while i < 10 {
        i += 1
        if i == 3 { continue }
        if i == 8 { break }
        total += i
    }
    print_int(fact(5))
    print(" ")
    print_int(total)
    print("\n")
    => 0
}
"#;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let dir =
        PathBuf::from("/tmp/opencode").join(format!("loi-llvm-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn has_tool(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn write_program(dir: &Path, source: &str) -> PathBuf {
    let input = dir.join("program.loi");
    fs::write(&input, source).expect("source should be writable");
    input
}

#[test]
fn emit_llvm_ir_writes_valid_llvm_text() {
    let dir = test_dir("emit-ir");
    let input = write_program(&dir, PROGRAM);
    let ir_path = dir.join("program.ll");
    let c_path = dir.join("program.c");

    let output = Command::new(loi())
        .arg("gen")
        .arg(&input)
        .arg("-o")
        .arg(&c_path)
        .arg("--emit-llvm-ir")
        .arg(&ir_path)
        .output()
        .expect("loi should run");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "loi gen --emit-llvm-ir failed: {stderr}"
    );

    let ir = fs::read_to_string(&ir_path).expect("LLVM IR file should be written");
    assert!(
        ir.contains("define i32 @loi_fn_fact"),
        "missing fact:\n{ir}"
    );
    assert!(ir.contains("define i32 @main"), "missing main:\n{ir}");
    assert!(ir.contains("icmp"), "missing comparison:\n{ir}");
    assert!(ir.contains("br i1"), "missing conditional branch:\n{ir}");
    assert!(
        ir.contains("br label"),
        "missing unconditional branch:\n{ir}"
    );
    assert!(
        ir.contains("call i32 @loi_fn_fact"),
        "missing recursive call:\n{ir}"
    );
    assert!(ir.contains("declare i32 @printf"), "missing printf:\n{ir}");

    // The emitted text must be real LLVM IR, not the draft pseudo-IR.
    if has_tool("clang") {
        let check = Command::new("clang")
            .arg("-c")
            .arg("-Wno-override-module")
            .arg("-x")
            .arg("ir")
            .arg(&ir_path)
            .arg("-o")
            .arg(dir.join("probe.o"))
            .output()
            .expect("clang should run");
        assert!(
            check.status.success(),
            "clang rejected the emitted IR: {}",
            String::from_utf8_lossy(&check.stderr)
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn llvm_backend_builds_and_runs_with_the_same_output_as_c11() {
    if !has_tool("cc") || !has_tool("clang") {
        eprintln!("skipping llvm_backend_builds_and_runs_with_the_same_output_as_c11");
        return;
    }
    let dir = test_dir("build");
    let input = write_program(&dir, PROGRAM);
    let c_exe = dir.join("c_program");
    let llvm_exe = dir.join("llvm_program");

    let c_build = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("-o")
        .arg(&c_exe)
        .output()
        .expect("loi should run");
    assert!(
        c_build.status.success(),
        "C11 build failed: {}",
        String::from_utf8_lossy(&c_build.stderr)
    );

    let llvm_build = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("--backend")
        .arg("llvm")
        .arg("-o")
        .arg(&llvm_exe)
        .output()
        .expect("loi should run");
    assert!(
        llvm_build.status.success(),
        "LLVM build failed: {}",
        String::from_utf8_lossy(&llvm_build.stderr)
    );

    let c_run = Command::new(&c_exe)
        .output()
        .expect("C11 binary should run");
    let llvm_run = Command::new(&llvm_exe)
        .output()
        .expect("LLVM binary should run");
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(c_run.status.code(), Some(0));
    assert_eq!(llvm_run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&c_run.stdout), "120 25\n");
    assert_eq!(
        String::from_utf8_lossy(&llvm_run.stdout),
        String::from_utf8_lossy(&c_run.stdout),
        "LLVM and C11 backends must agree"
    );
}

#[test]
fn llvm_example_matches_the_c11_backend() {
    if !has_tool("cc") || !has_tool("clang") {
        eprintln!("skipping llvm_example_matches_the_c11_backend");
        return;
    }
    let dir = test_dir("example");
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("llvm_scalars.loi");
    let c_exe = dir.join("example_c");
    let llvm_exe = dir.join("example_llvm");

    let c_build = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("-o")
        .arg(&c_exe)
        .output()
        .expect("loi should run");
    assert!(
        c_build.status.success(),
        "C11 build failed: {}",
        String::from_utf8_lossy(&c_build.stderr)
    );
    let llvm_build = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("--backend")
        .arg("llvm")
        .arg("-o")
        .arg(&llvm_exe)
        .output()
        .expect("loi should run");
    assert!(
        llvm_build.status.success(),
        "LLVM build failed: {}",
        String::from_utf8_lossy(&llvm_build.stderr)
    );

    let c_run = Command::new(&c_exe)
        .output()
        .expect("C11 binary should run");
    let llvm_run = Command::new(&llvm_exe)
        .output()
        .expect("LLVM binary should run");
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(c_run.status.code(), Some(0));
    assert_eq!(llvm_run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&c_run.stdout),
        "factorial=120 squares=55 mean=2\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&llvm_run.stdout),
        String::from_utf8_lossy(&c_run.stdout),
        "the example must agree across backends"
    );
}

#[test]
fn llvm_backend_emits_slice_runtime_helpers() {
    let dir = test_dir("slice-ir");
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("llvm_slices.loi");
    let ir_path = dir.join("program.ll");

    let output = Command::new(loi())
        .arg("gen")
        .arg(&input)
        .arg("--emit-llvm-ir")
        .arg(&ir_path)
        .output()
        .expect("loi should run");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "loi gen --emit-llvm-ir failed: {stderr}"
    );

    let ir = fs::read_to_string(&ir_path).expect("LLVM IR file should be written");
    for expected in [
        "%LoiSlice = type { ptr, i64 }",
        "define %LoiSlice @loi_alloc_slice",
        "define ptr @loi_index",
        "define void @loi_runtime_fail",
        "call i32 @fwrite",
    ] {
        assert!(ir.contains(expected), "missing '{expected}':\n{ir}");
    }

    // The emitted text must be real LLVM IR, not the draft pseudo-IR.
    if has_tool("clang") {
        let check = Command::new("clang")
            .arg("-c")
            .arg("-Wno-override-module")
            .arg("-x")
            .arg("ir")
            .arg(&ir_path)
            .arg("-o")
            .arg(dir.join("probe.o"))
            .output()
            .expect("clang should run");
        assert!(
            check.status.success(),
            "clang rejected the emitted IR: {}",
            String::from_utf8_lossy(&check.stderr)
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn llvm_backend_matches_c11_for_slices_protect_and_alloc() {
    if !has_tool("cc") || !has_tool("clang") {
        eprintln!("skipping llvm_backend_matches_c11_for_slices_protect_and_alloc");
        return;
    }
    let dir = test_dir("slice-parity");
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("llvm_slices.loi");
    let c_exe = dir.join("example_c");
    let llvm_exe = dir.join("example_llvm");

    let c_build = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("-o")
        .arg(&c_exe)
        .output()
        .expect("loi should run");
    assert!(
        c_build.status.success(),
        "C11 build failed: {}",
        String::from_utf8_lossy(&c_build.stderr)
    );
    let llvm_build = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("--backend")
        .arg("llvm")
        .arg("-o")
        .arg(&llvm_exe)
        .output()
        .expect("loi should run");
    assert!(
        llvm_build.status.success(),
        "LLVM build failed: {}",
        String::from_utf8_lossy(&llvm_build.stderr)
    );

    let c_run = Command::new(&c_exe)
        .output()
        .expect("C11 binary should run");
    let llvm_run = Command::new(&llvm_exe)
        .output()
        .expect("LLVM binary should run");
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(c_run.status.code(), Some(0));
    assert_eq!(llvm_run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&c_run.stdout),
        "sum=10 len=4\narena=60\nHi\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&llvm_run.stdout),
        String::from_utf8_lossy(&c_run.stdout),
        "LLVM and C11 backends must agree on slices, protect and alloc"
    );
}

#[test]
fn llvm_backend_rejects_unsupported_slice_elements() {
    let dir = test_dir("reject-slice-element");
    let input = write_program(
        &dir,
        r#"shape Point [ x: i32 ]

flow first
    <- xs: [Point]
    => i32
{
    => 0
}

open flow main
    => i32
{
    => 0
}
"#,
    );

    let output = Command::new(loi())
        .arg("gen")
        .arg(&input)
        .arg("--emit-llvm-ir")
        .arg(dir.join("program.ll"))
        .output()
        .expect("loi should run");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);

    assert!(!output.status.success(), "shape slices must be rejected");
    assert!(
        stderr.contains("LLVM backend does not support shape types"),
        "unexpected diagnostic: {stderr}"
    );
}

#[test]
fn llvm_backend_rejects_incompatible_build_options() {
    let dir = test_dir("options");
    let input = write_program(&dir, PROGRAM);
    let shim = dir.join("shim.c");
    fs::write(&shim, "void nothing(void) {}\n").expect("shim should be writable");

    let cases: Vec<Vec<&str>> = vec![
        vec!["--emit", "obj"],
        vec!["--emit", "lib"],
        vec!["--c-source", shim.to_str().unwrap()],
        vec!["--emit-c", "out.c"],
        vec!["--emit-h", "out.h"],
        vec!["--arena-size", "4096"],
        vec!["--source-map", "map.json"],
    ];

    for extra in cases {
        let mut command = Command::new(loi());
        command
            .arg("build")
            .arg(&input)
            .arg("--backend")
            .arg("llvm")
            .arg("-o")
            .arg(dir.join("program"));
        for flag in &extra {
            command.arg(flag);
        }
        let output = command.output().expect("loi should run");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert_eq!(output.status.code(), Some(2), "flags {extra:?}: {stderr}");
        assert!(
            stderr.contains("LLVM backend"),
            "flags {extra:?} gave an unexpected message: {stderr}"
        );
    }

    let multi = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg(&input)
        .arg("--backend")
        .arg("llvm")
        .arg("-o")
        .arg(dir.join("program"))
        .output()
        .expect("loi should run");
    let multi_stderr = String::from_utf8_lossy(&multi.stderr).to_string();
    assert_eq!(multi.status.code(), Some(2), "{multi_stderr}");
    assert!(multi_stderr.contains("LLVM backend"), "{multi_stderr}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn backend_flag_is_only_supported_by_build() {
    let dir = test_dir("build-only");
    let input = write_program(&dir, PROGRAM);

    for mode in ["check", "gen", "lex", "parse", "fmt"] {
        let output = Command::new(loi())
            .arg(mode)
            .arg(&input)
            .arg("--backend")
            .arg("llvm")
            .output()
            .expect("loi should run");
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert_eq!(output.status.code(), Some(2), "mode {mode}: {stderr}");
        assert!(
            stderr.contains("'--backend' is only supported by 'build'"),
            "mode {mode}: {stderr}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn build_rejects_an_unknown_backend() {
    let dir = test_dir("unknown-backend");
    let input = write_program(&dir, PROGRAM);

    let output = Command::new(loi())
        .arg("build")
        .arg(&input)
        .arg("--backend")
        .arg("gcc")
        .output()
        .expect("loi should run");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("unknown backend 'gcc'"), "{stderr}");
}
