use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-namespace-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
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

/// Minimal package layout: `lib/<pkg>.loi` plus `lib/<pkg>/<module>.loi`.
fn package(name: &str, pkg: &str) -> PathBuf {
    let dir = test_dir(name);
    fs::create_dir_all(dir.join("lib").join(pkg)).expect("package directory should be creatable");
    write(&dir, &format!("lib/{pkg}.loi"), "// package root\n");
    dir
}

const FMT: &str = "open flow fmt_answer\n    => i32\n{\n    => 42\n}\n";
const VEC: &str = "open flow vec_answer\n    => i32\n{\n    => 7\n}\n";
const STR: &str = "open flow str_answer\n    => i32\n{\n    => 3\n}\n";

fn check_in(dir: &Path, file: &str) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(file)
        .current_dir(dir)
        .output()
        .expect("loi should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_checks(dir: &Path, file: &str) {
    let output = check_in(dir, file);
    assert!(output.status.success(), "check failed: {}", stderr(&output));
}

fn assert_diagnostic(dir: &Path, file: &str, message: &str) -> String {
    let output = check_in(dir, file);
    let text = stderr(&output);
    assert!(!output.status.success(), "diagnostic was not reported");
    assert!(text.contains(message), "unexpected diagnostics: {text}");
    text
}

#[test]
fn logical_import_resolves_the_narrow_module_file() {
    let dir = package("logical-basic", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(
        &dir,
        "main.loi",
        "import std::fmt\nopen flow main\n    => i32\n{\n    => fmt::fmt_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn logical_import_alias_binds_the_requested_name() {
    let dir = package("logical-alias", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(
        &dir,
        "main.loi",
        "import std::fmt as format\nopen flow main\n    => i32\n{\n    => format::fmt_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn grouped_import_resolves_each_item() {
    let dir = package("group-basic", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std/vec.loi", VEC);
    write(
        &dir,
        "main.loi",
        "import std::{fmt, vec}\nopen flow main\n    => i32\n{\n    => fmt::fmt_answer() + vec::vec_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn grouped_import_aliases_bind_each_item() {
    let dir = package("group-aliases", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std/vec.loi", VEC);
    write(&dir, "lib/std/str.loi", STR);
    write(
        &dir,
        "main.loi",
        "import std::{fmt as f, vec as v, str as string}\n\
         open flow main\n    => i32\n{\n    => f::fmt_answer() + v::vec_answer() + string::str_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn deeper_logical_paths_resolve_nested_module_files() {
    let dir = package("nested-logical", "std");
    write(
        &dir,
        "lib/std/fmt/detail.loi",
        "open flow detail_answer\n    => i32\n{\n    => 11\n}\n",
    );
    write(
        &dir,
        "main.loi",
        "import std::fmt::detail\nopen flow main\n    => i32\n{\n    => detail::detail_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn grouped_import_with_a_qualified_base_resolves_nested_files() {
    let dir = package("nested-group", "std");
    write(
        &dir,
        "lib/std/fmt/detail.loi",
        "open flow detail_answer\n    => i32\n{\n    => 11\n}\n",
    );
    write(
        &dir,
        "lib/std/fmt/width.loi",
        "open flow width_answer\n    => i32\n{\n    => 4\n}\n",
    );
    write(
        &dir,
        "main.loi",
        "import std::fmt::{detail as d, width}\n\
         open flow main\n    => i32\n{\n    => d::detail_answer() + width::width_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn open_grouped_import_reexports_through_a_package_root() {
    let dir = package("group-open", "os");
    write(
        &dir,
        "lib/os/file.loi",
        "open flow file_answer\n    => i32\n{\n    => 2\n}\n",
    );
    write(
        &dir,
        "lib/os/path.loi",
        "open flow path_answer\n    => i32\n{\n    => 5\n}\n",
    );
    write(&dir, "lib/os.loi", "open import os::{file, path}\n");
    write(
        &dir,
        "main.loi",
        "import \"lib/os.loi\" as os\nopen flow main\n    => i32\n{\n    => os::file_answer() + os::path_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn importing_a_package_root_loads_its_root_module() {
    let dir = package("package-root", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std/vec.loi", VEC);
    write(
        &dir,
        "lib/std.loi",
        "open import \"std/fmt.loi\"\nopen import \"std/vec.loi\"\n",
    );
    write(
        &dir,
        "main.loi",
        "import std\nopen flow main\n    => i32\n{\n    => std::fmt_answer() + std::vec_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn importing_a_package_root_honours_the_alias() {
    let dir = package("package-root-alias", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std.loi", "open import \"std/fmt.loi\"\n");
    write(
        &dir,
        "main.loi",
        "import std as s\nopen flow main\n    => i32\n{\n    => s::fmt_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn importing_a_package_root_does_not_create_submodule_aliases() {
    let dir = package("package-root-flat", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std.loi", "open import \"std/fmt.loi\"\n");
    write(
        &dir,
        "main.loi",
        "import std\nopen flow main\n    => i32\n{\n    => fmt::fmt_answer()\n}\n",
    );
    assert_diagnostic(&dir, "main.loi", "unknown module 'fmt'");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn importing_a_package_root_deduplicates_the_explicit_path_import() {
    let dir = package("package-root-dedup", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std.loi", "open import \"std/fmt.loi\"\n");
    write(
        &dir,
        "main.loi",
        "import std\nimport \"lib/std.loi\" as s\nopen flow main\n    => i32\n{\n    => std::fmt_answer() + s::fmt_answer()\n}\n",
    );
    assert_checks(&dir, "main.loi");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn importing_a_package_root_resolves_from_lib_dir() {
    let dir = test_dir("package-root-lib-dir");
    let lib = dir.join("libroot");
    write(&lib, "std/fmt.loi", FMT);
    write(&lib, "std.loi", "open import \"std/fmt.loi\"\n");
    write(
        &dir,
        "main.loi",
        "import std\nopen flow main\n    => i32\n{\n    => std::fmt_answer()\n}\n",
    );
    let lib = lib.to_string_lossy().to_string();
    let output = Command::new(loi())
        .arg("check")
        .arg("main.loi")
        .arg("--lib-dir")
        .arg(&lib)
        .current_dir(&dir)
        .output()
        .expect("loi should run");
    assert!(output.status.success(), "check failed: {}", stderr(&output));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unknown_package_names_the_searched_layout() {
    let dir = package("unknown-package", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(
        &dir,
        "main.loi",
        "import nope::fmt\nflow main {\n    => 0\n}\n",
    );
    let text = assert_diagnostic(&dir, "main.loi", "unknown package 'nope'");
    assert!(
        text.contains("lib/nope.loi"),
        "unexpected diagnostics: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unknown_module_names_the_expected_file() {
    let dir = package("unknown-module", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(
        &dir,
        "main.loi",
        "import std::missing\nflow main {\n    => 0\n}\n",
    );
    let text = assert_diagnostic(&dir, "main.loi", "unknown module 'std::missing'");
    assert!(
        text.contains("lib/std/missing.loi"),
        "unexpected diagnostics: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn grouped_unknown_item_points_at_the_item() {
    let dir = package("unknown-item", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(
        &dir,
        "main.loi",
        "import std::{fmt, missing}\nflow main {\n    => 0\n}\n",
    );
    let text = assert_diagnostic(&dir, "main.loi", "unknown module 'std::missing'");
    assert!(
        text.contains("main.loi:1:19"),
        "diagnostic should point at the group item: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_alias_between_logical_imports_is_rejected() {
    let dir = package("alias-collision", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    write(&dir, "lib/std/vec.loi", VEC);
    write(
        &dir,
        "main.loi",
        "import std::fmt as x\nimport std::vec as x\nflow main {\n    => 0\n}\n",
    );
    assert_diagnostic(&dir, "main.loi", "duplicate module alias 'x'");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn logical_import_cycles_are_rejected() {
    let dir = package("logical-cycle", "a");
    write(
        &dir,
        "lib/a/x.loi",
        "import a::y\nopen flow x\n    => i32\n{\n    => 0\n}\n",
    );
    write(
        &dir,
        "lib/a/y.loi",
        "import a::x\nopen flow y\n    => i32\n{\n    => 0\n}\n",
    );
    write(
        &dir,
        "main.loi",
        "import a::x\nflow main\n    => i32\n{\n    => 0\n}\n",
    );
    assert_diagnostic(&dir, "main.loi", "import cycle detected while loading");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn logical_resolution_does_not_depend_on_the_process_cwd() {
    let dir = package("cwd-independent", "std");
    write(&dir, "lib/std/fmt.loi", FMT);
    let main = write(
        &dir,
        "main.loi",
        "import std::fmt\nopen flow main\n    => i32\n{\n    => fmt::fmt_answer()\n}\n",
    );
    let elsewhere = test_dir("cwd-independent-elsewhere");
    let output = Command::new(loi())
        .arg("check")
        .arg(&main)
        .current_dir(&elsewhere)
        .output()
        .expect("loi should run");
    let _ = fs::remove_dir_all(&elsewhere);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "check failed from another cwd: {}",
        stderr(&output)
    );
}

#[test]
fn extern_flows_inside_logically_imported_modules_are_collected_for_linking() {
    let has_cc = Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !has_cc {
        eprintln!("skipping extern-flow collection test: no C compiler found");
        return;
    }
    let dir = package("extern-flow", "std");
    write(
        &dir,
        "lib/std/ffi.loi",
        "extern flow c_answer\n    => i32\n",
    );
    write(
        &dir,
        "main.loi",
        "import std::ffi\nopen flow main\n    => i32\n{\n    => ffi::c_answer()\n}\n",
    );
    let output = Command::new(loi())
        .args(["build", "main.loi", "-o", "app"])
        .current_dir(&dir)
        .output()
        .expect("loi should build");
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "missing shim was not detected");
    assert!(
        text.contains("c_answer"),
        "extern flow from logical import was not collected: {text}"
    );
}
