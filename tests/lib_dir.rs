use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_lib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib")
}

fn test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("loi-lib-dir-{}-{name}-{nanos}", std::process::id()));
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

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(loi())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("loi should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_ok(output: &Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed: {}",
        stderr(output)
    );
}

fn assert_error(output: &Output, message: &str) {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    assert!(
        stderr(output).contains(message),
        "unexpected diagnostics: {}",
        stderr(output)
    );
}

const MAIN_FMT: &str = "import std::fmt\n\
                        open flow main\n\
                        {\n\
                            protect {\n\
                                print(fmt::fmt_line(\"ola\"))\n\
                            }\n\
                        }\n";

const MAIN_PKG: &str = "import pkg::ffi\n\
                        open flow main\n\
                        => i32\n\
                        {\n\
                            => ffi::answer()\n\
                        }\n";

fn fake_root(dir: &Path, name: &str, answer: i32) -> PathBuf {
    let root = dir.join(name);
    write(&root, "pkg.loi", "// package root\n");
    write(
        &root,
        "pkg/ffi.loi",
        &format!("open flow answer\n    => i32\n{{\n    => {answer}\n}}\n"),
    );
    root
}

#[test]
fn lib_dir_resolves_a_package_from_an_external_project() {
    let dir = test_dir("check");
    write(&dir, "main.loi", MAIN_FMT);
    let lib = repo_lib();
    let output = run_in(
        &dir,
        &["check", "main.loi", "--lib-dir", lib.to_str().unwrap()],
    );
    assert_ok(&output, "check");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lib_dir_builds_and_runs_an_external_project() {
    let dir = test_dir("build");
    write(&dir, "main.loi", MAIN_FMT);
    let lib = repo_lib();
    let output = run_in(
        &dir,
        &[
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            lib.to_str().unwrap(),
        ],
    );
    assert_ok(&output, "build");
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "ola\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lib_dir_is_repeatable_and_searched_in_order() {
    let dir = test_dir("order");
    write(&dir, "main.loi", MAIN_PKG);
    let first = fake_root(&dir, "first", 7);
    let second = fake_root(&dir, "second", 42);
    let output = run_in(
        &dir,
        &[
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            first.to_str().unwrap(),
            "--lib-dir",
            second.to_str().unwrap(),
        ],
    );
    assert_ok(&output, "build");
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(run.status.code(), Some(7), "first root should win");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lib_dir_falls_back_to_a_later_root_when_the_first_has_no_package() {
    let dir = test_dir("fallback");
    write(&dir, "main.loi", MAIN_PKG);
    let empty = dir.join("empty");
    fs::create_dir_all(&empty).expect("empty root should be creatable");
    let second = fake_root(&dir, "second", 42);
    let output = run_in(
        &dir,
        &[
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            empty.to_str().unwrap(),
            "--lib-dir",
            second.to_str().unwrap(),
        ],
    );
    assert_ok(&output, "build");
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(run.status.code(), Some(42));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_project_lib_still_takes_precedence_over_lib_dir() {
    let dir = test_dir("precedence");
    write(&dir, "main.loi", MAIN_PKG);
    let project = fake_root(&dir, "lib", 7);
    let external = fake_root(&dir, "external", 42);
    let output = run_in(
        &dir,
        &[
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            external.to_str().unwrap(),
        ],
    );
    assert_ok(&output, "build");
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(run.status.code(), Some(7), "project lib should win");
    let _ = fs::remove_dir_all(&dir);
    assert!(project.is_absolute());
}

#[test]
fn lib_dir_diagnostic_lists_the_extra_root() {
    let dir = test_dir("diagnostic");
    write(
        &dir,
        "main.loi",
        "import nope::fmt\nflow main {\n    => 0\n}\n",
    );
    let lib = repo_lib();
    let output = run_in(
        &dir,
        &["check", "main.loi", "--lib-dir", lib.to_str().unwrap()],
    );
    assert_error(&output, "unknown package 'nope'");
    assert!(
        stderr(&output).contains(lib.to_str().unwrap()),
        "diagnostic should list the extra root: {}",
        stderr(&output)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lib_dir_auto_c_source_links_a_shim_from_an_extra_root() {
    let dir = test_dir("native");
    write(&dir, "main.loi", MAIN_PKG);
    let lib = dir.join("external-lib");
    write(&lib, "pkg.loi", "// package root\n");
    write(&lib, "pkg/ffi.loi", "open extern flow answer\n    => i32\n");
    write(
        &lib,
        "pkg_shim.c",
        "#include <stdint.h>\nint32_t answer(void) { return 42; }\n",
    );
    let output = run_in(
        &dir,
        &[
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            lib.to_str().unwrap(),
            "--auto-c-source",
        ],
    );
    assert_ok(&output, "build");
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(run.status.code(), Some(42));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lib_dir_rejects_modes_that_do_not_resolve_imports() {
    let dir = test_dir("modes");
    write(&dir, "main.loi", MAIN_FMT);
    let lib = repo_lib();
    let output = run_in(
        &dir,
        &["lex", "main.loi", "--lib-dir", lib.to_str().unwrap()],
    );
    assert_error(
        &output,
        "'--lib-dir' is only supported by 'check', 'gen' and 'build'",
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lib_dir_rejects_a_non_directory_and_a_missing_value() {
    let dir = test_dir("validation");
    let main = write(&dir, "main.loi", MAIN_FMT);
    let output = run_in(
        &dir,
        &["check", "main.loi", "--lib-dir", main.to_str().unwrap()],
    );
    assert_error(&output, "'--lib-dir' is not a directory");
    let output = run_in(&dir, &["check", "main.loi", "--lib-dir"]);
    assert_error(&output, "'--lib-dir' requires a path");
    let _ = fs::remove_dir_all(&dir);
}
