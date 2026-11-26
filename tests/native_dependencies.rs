use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("loi-native-{}-{name}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn write(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent should be creatable");
    }
    fs::write(&path, source).expect("source should be writable");
    path
}

fn package(name: &str, pkg: &str, shim: &str) -> PathBuf {
    let dir = test_dir(name);
    write(&dir, &format!("lib/{pkg}.loi"), "// package root\n");
    write(
        &dir,
        &format!("lib/{pkg}/ffi.loi"),
        "open extern flow loi_answer\n    => i32\n",
    );
    write(&dir, &format!("lib/{pkg}_shim.c"), shim);
    write(
        &dir,
        "main.loi",
        "import pkg::ffi\nopen flow main\n    => i32\n{\n    => ffi::loi_answer()\n}\n",
    );
    dir
}

const SHIM: &str = "#include <stdint.h>\nint32_t loi_answer(void) { return 42; }\n";

fn build_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(loi())
        .arg("build")
        .arg("main.loi")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("loi should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_build(dir: &Path, args: &[&str]) {
    let output = build_in(dir, args);
    assert!(output.status.success(), "build failed: {}", stderr(&output));
}

fn assert_exit(run: &Output, code: i32) {
    assert_eq!(run.status.code(), Some(code), "unexpected exit status");
}

#[test]
fn auto_c_source_links_the_package_shim_found_through_a_logical_import() {
    let dir = package("auto-basic", "pkg", SHIM);
    assert_build(&dir, &["-o", "app", "--auto-c-source"]);
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_exit(&run, 42);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_is_opt_in_and_keeps_the_explicit_link_error() {
    let dir = package("auto-opt-in", "pkg", SHIM);
    let output = build_in(&dir, &["-o", "app"]);
    let text = stderr(&output);
    assert!(!output.status.success(), "shim was linked without the flag");
    assert!(
        text.contains("build with --c-source") && text.contains("lib/pkg_shim.c"),
        "explicit hint changed: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn explicit_and_auto_c_sources_do_not_link_the_same_file_twice() {
    let dir = package("auto-dedupe", "pkg", SHIM);
    assert_build(
        &dir,
        &[
            "-o",
            "app",
            "--c-source",
            "lib/pkg_shim.c",
            "--auto-c-source",
        ],
    );
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_exit(&run, 42);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_prefers_the_shim_beside_the_declaring_module() {
    let dir = package(
        "auto-adjacent",
        "pkg",
        "#include <stdint.h>\nint32_t loi_other(void) { return 1; }\n",
    );
    write(
        &dir,
        "lib/pkg/ffi_shim.c",
        "#include <stdint.h>\nint32_t loi_answer(void) { return 7; }\n",
    );
    assert_build(&dir, &["-o", "app", "--auto-c-source"]);
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_exit(&run, 7);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_does_not_select_a_shim_that_only_mentions_the_symbol() {
    let dir = package(
        "auto-comment",
        "pkg",
        "// int32_t loi_answer(void) { return 999; }\n",
    );
    let output = build_in(&dir, &["-o", "app", "--auto-c-source"]);
    let text = stderr(&output);
    assert!(
        !output.status.success(),
        "comment was treated as a definition"
    );
    assert!(
        text.contains("is not defined by any linked C source"),
        "unexpected diagnostics: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_accepts_a_definition_with_a_linkage_attribute() {
    let dir = package(
        "auto-attribute",
        "pkg",
        "#include <stdint.h>\nint32_t loi_answer(void) [[gnu::weak]] { return 42; }\n",
    );
    assert_build(&dir, &["-o", "app", "--auto-c-source"]);
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_exit(&run, 42);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_ignores_a_function_like_macro() {
    let dir = package(
        "auto-macro",
        "pkg",
        "#define loi_answer(x) { return 42; }\n",
    );
    let output = build_in(&dir, &["-o", "app", "--auto-c-source"]);
    let text = stderr(&output);
    assert!(
        !output.status.success(),
        "macro was treated as a C definition"
    );
    assert!(
        text.contains("is not defined by any linked C source")
            && !text.contains("this library ships a C shim"),
        "macro must not be advertised as a shim: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_ignores_a_definition_hidden_behind_if_zero() {
    let dir = package(
        "auto-if-zero",
        "pkg",
        "#if 0\nint32_t loi_answer(void) { return 42; }\n#endif\n",
    );
    let output = build_in(&dir, &["-o", "app", "--auto-c-source"]);
    let text = stderr(&output);
    assert!(
        !output.status.success(),
        "disabled C code was treated as a definition"
    );
    assert!(
        text.contains("is not defined by any linked C source")
            && !text.contains("this library ships a C shim"),
        "#if 0 must not be advertised as a shim: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_reports_two_packages_defining_one_symbol() {
    let dir = test_dir("auto-ambiguous");
    write(&dir, "lib/a.loi", "// package root\n");
    write(
        &dir,
        "lib/a/ffi.loi",
        "open extern flow loi_a_answer as loi_answer\n    => i32\n",
    );
    write(&dir, "lib/a_shim.c", SHIM);
    write(&dir, "lib/b.loi", "// package root\n");
    write(
        &dir,
        "lib/b/ffi.loi",
        "open extern flow loi_b_answer as loi_answer\n    => i32\n",
    );
    write(&dir, "lib/b_shim.c", SHIM);
    write(
        &dir,
        "main.loi",
        "import a::ffi as a\nimport b::ffi as b\nopen flow main\n    => i32\n{\n    => a::loi_a_answer() + b::loi_b_answer()\n}\n",
    );
    let output = build_in(&dir, &["-o", "app", "--auto-c-source"]);
    let text = stderr(&output);
    assert!(
        !output.status.success(),
        "duplicate symbol was linked silently"
    );
    assert!(
        text.contains("ambiguous C shim")
            && text.contains("lib/a_shim.c")
            && text.contains("lib/b_shim.c"),
        "unexpected diagnostics: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_resolves_from_the_importing_file_not_the_process_cwd() {
    let dir = package("auto-cwd", "pkg", SHIM);
    let elsewhere = test_dir("auto-cwd-elsewhere");
    let output = Command::new(loi())
        .arg("build")
        .arg(dir.join("main.loi"))
        .arg("-o")
        .arg(dir.join("app"))
        .arg("--auto-c-source")
        .current_dir(&elsewhere)
        .output()
        .expect("loi should run");
    let _ = fs::remove_dir_all(&elsewhere);
    assert!(
        output.status.success(),
        "build failed from another cwd: {}",
        stderr(&output)
    );
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_exit(&run, 42);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_rejects_object_and_llvm_outputs() {
    let dir = package("auto-reject", "pkg", SHIM);
    for args in [
        vec!["--emit", "obj", "--auto-c-source"],
        vec!["--backend", "llvm", "--auto-c-source"],
    ] {
        let output = build_in(&dir, &args);
        assert!(!output.status.success(), "unexpected success for {args:?}");
        assert!(
            stderr(&output).contains("'--auto-c-source'"),
            "flag was not named: {}",
            stderr(&output)
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn auto_c_source_object_rejection_names_both_link_flags() {
    let dir = package("auto-both-flags", "pkg", SHIM);
    let output = build_in(
        &dir,
        &[
            "--emit",
            "obj",
            "--c-source",
            "lib/pkg_shim.c",
            "--auto-c-source",
        ],
    );
    let text = stderr(&output);
    assert!(!output.status.success(), "unexpected success");
    assert!(
        text.contains("'--c-source'") && text.contains("'--auto-c-source'"),
        "both flags should be named: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn libc_alias_replaces_a_pass_through_shim_function() {
    let dir = test_dir("libc-alias");
    write(
        &dir,
        "main.loi",
        "extern flow loi_test_exit as exit\n    <- code: i32\nflow main\n{\n    loi_test_exit(7)\n}\n",
    );
    let output = build_in(&dir, &["-o", "app"]);
    assert!(output.status.success(), "build failed: {}", stderr(&output));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    let _ = fs::remove_dir_all(&dir);
    assert_exit(&run, 7);
}

#[test]
fn libc_close_alias_replaces_duplicate_shim_wrappers() {
    let dir = test_dir("libc-close");
    write(
        &dir,
        "main.loi",
        "extern flow loi_test_close as close\n    <- fd: i32\n    => i32\nflow main\n{\n    _ <- loi_test_close(-1)\n}\n",
    );
    let output = build_in(&dir, &["-o", "app"]);
    assert!(output.status.success(), "build failed: {}", stderr(&output));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    let _ = fs::remove_dir_all(&dir);
    assert_exit(&run, 0);
}

#[test]
fn auto_c_source_invalidates_the_incremental_stamp() {
    let dir = package("auto-stamp", "pkg", SHIM);
    assert_build(&dir, &["-o", "app", "--auto-c-source"]);
    write(
        &dir,
        "lib/pkg_shim.c",
        "#include <stdint.h>\nint32_t loi_answer(void) { return 99; }\n",
    );
    assert_build(&dir, &["-o", "app", "--auto-c-source"]);
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(run.status.code(), Some(99), "stale shim was reused");
    let _ = fs::remove_dir_all(&dir);
}
