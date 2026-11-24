use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-mangling-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn write(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("source should be writable");
    path
}

fn run(args: &[&Path]) -> Output {
    Command::new(loi())
        .args(args.iter().map(|arg| arg.as_os_str()))
        .output()
        .expect("loi should run")
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

const A: &str = "open flow helper\n    => i32\n{\n    => 1\n}\n\nopen flow own_value\n    => i32\n{\n    => helper()\n}\n";
const B: &str = "open flow helper\n    => i32\n{\n    => 2\n}\n\nopen flow own_value\n    => i32\n{\n    => helper() + 10\n}\n";

fn fixture(dir: &Path, main_body: &str) -> PathBuf {
    write(dir, "a.loi", A);
    write(dir, "b.loi", B);
    write(
        dir,
        "main.loi",
        &format!(
            "import \"a.loi\" as a\nimport \"b.loi\" as b\nopen flow main\n    => i32\n{{\n    {main_body}\n}}\n"
        ),
    )
}

#[test]
fn two_modules_may_declare_the_same_name() {
    let dir = test_dir("duplicate-name");
    let main = fixture(&dir, "=> a::helper() + b::helper()");
    let output = run(&[Path::new("check"), &main]);
    let message = text(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "check failed: {message}");
}

#[test]
fn unqualified_names_bind_to_the_module_that_declares_them() {
    let dir = test_dir("unqualified-own");
    let main = fixture(&dir, "=> a::own_value() + b::own_value()");
    let output = Command::new(loi())
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(dir.join("program"))
        .output()
        .expect("loi build should run");
    let build_message = text(&output);
    if output.status.success() {
        let run_output = Command::new(dir.join("program"))
            .output()
            .expect("program should run");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(run_output.status.code(), Some(13), "unexpected exit code");
    } else {
        let _ = fs::remove_dir_all(&dir);
        panic!("build failed: {build_message}");
    }
}

#[test]
fn an_unqualified_name_declared_in_two_modules_is_ambiguous() {
    let dir = test_dir("ambiguous");
    let main = fixture(&dir, "=> helper()");
    let output = run(&[Path::new("check"), &main]);
    let message = text(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "ambiguous name was accepted");
    assert!(
        message.contains("ambiguous name 'helper'"),
        "unexpected diagnostics: {message}"
    );
    assert!(
        message.contains("a::helper") && message.contains("b::helper"),
        "diagnostics should list the candidates: {message}"
    );
}

#[test]
fn generated_c_uses_distinct_module_qualified_symbols() {
    let dir = test_dir("symbols");
    let main = fixture(&dir, "=> a::helper() + b::helper()");
    let output = run(&[Path::new("gen"), &main]);
    let c = text(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "gen failed: {c}");
    assert!(c.contains("loi_fn_a__helper("), "missing module a symbol");
    assert!(c.contains("loi_fn_b__helper("), "missing module b symbol");
}

#[test]
fn headers_expose_module_qualified_symbols() {
    let dir = test_dir("headers");
    let main = fixture(&dir, "=> a::helper() + b::helper()");
    let header = dir.join("main.h");
    let output = Command::new(loi())
        .arg("gen")
        .arg(&main)
        .arg("--emit-h")
        .arg(&header)
        .output()
        .expect("loi gen should run");
    let message = text(&output);
    assert!(output.status.success(), "gen failed: {message}");
    let content = fs::read_to_string(&header).expect("header should exist");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        content.contains("loi_fn_a__helper("),
        "missing module a symbol in header: {content}"
    );
    assert!(
        content.contains("loi_fn_b__helper("),
        "missing module b symbol in header: {content}"
    );
}

#[test]
fn multi_file_build_links_duplicate_names() {
    let dir = test_dir("multi-file");
    let main = fixture(&dir, "=> a::helper() + b::helper()");
    let second = write(
        &dir,
        "second.loi",
        "import \"b.loi\" as b\nopen flow second\n    => i32\n{\n    => b::helper()\n}\n",
    );
    let out = dir.join("program");
    let output = Command::new(loi())
        .arg("build")
        .arg(&main)
        .arg(&second)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("loi build should run");
    let build_message = text(&output);
    if output.status.success() {
        let run_output = Command::new(&out).output().expect("program should run");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(run_output.status.code(), Some(3), "unexpected exit code");
    } else {
        let _ = fs::remove_dir_all(&dir);
        panic!("build failed: {build_message}");
    }
}

#[test]
fn all_declaration_kinds_get_distinct_module_qualified_c_symbols() {
    let dir = test_dir("declaration-kinds");
    let module = |alias: &str, value: &str| -> String {
        format!(
            "open shape Point [\n    x: i32\n]\n\nopen mark Color [ Red, Blue ]\n\nopen LIMIT: i32 <- {value}\n\nopen shape Box<T> [\n    value: T\n]\n\nopen flow boxed\n    => Box<i32>\n{{\n    => Box<i32>[ value: {alias}::LIMIT ]\n}}\n"
        )
    };
    write(&dir, "a.loi", &module("a", "7"));
    write(&dir, "b.loi", &module("b", "5"));
    let main = write(
        &dir,
        "main.loi",
        "import \"a.loi\" as a\nimport \"b.loi\" as b\nopen flow main\n    => i32\n{\n    p <- a::Point[ x: 1 ]\n    q <- b::Point[ x: 2 ]\n    if a::Color.Red == a::Color.Red { }\n    if b::Color.Blue == b::Color.Blue { }\n    x <- a::boxed()\n    y <- b::boxed()\n    => p.x + q.x + a::LIMIT + b::LIMIT + x.value + y.value\n}\n",
    );
    let output = Command::new(loi())
        .arg("gen")
        .arg(&main)
        .output()
        .expect("loi gen should run");
    let c = text(&output);
    assert!(output.status.success(), "gen failed: {c}");
    for symbol in [
        "loi_t_a__Point",
        "loi_t_b__Point",
        "loi_m_a__Color_Red",
        "loi_m_b__Color_Red",
        "loi_g_a__LIMIT",
        "loi_g_b__LIMIT",
        "loi_t_a__Box__i32",
        "loi_t_b__Box__i32",
    ] {
        assert!(c.contains(symbol), "missing symbol {symbol} in generated C");
    }
    let built = Command::new(loi())
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(dir.join("program"))
        .output()
        .expect("loi build should run");
    let build_message = text(&built);
    if built.status.success() {
        let run_output = Command::new(dir.join("program"))
            .output()
            .expect("program should run");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(run_output.status.code(), Some(27), "unexpected exit code");
    } else {
        let _ = fs::remove_dir_all(&dir);
        panic!("build failed: {build_message}");
    }
}

#[test]
fn sanitized_symbols_that_collide_with_a_user_name_are_rejected() {
    let dir = test_dir("symbol-collision");
    write(&dir, "a.loi", A);
    write(&dir, "b.loi", B);
    let main = write(
        &dir,
        "main.loi",
        "import \"a.loi\" as a\nimport \"b.loi\" as b\nopen flow a__helper\n    => i32\n{\n    => 99\n}\nopen flow main\n    => i32\n{\n    => a::helper() + a__helper()\n}\n",
    );
    let output = run(&[Path::new("gen"), &main]);
    let message = text(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "C symbol collision was accepted");
    assert!(
        message.contains("C symbol 'loi_fn_a__helper'"),
        "unexpected diagnostics: {message}"
    );
}

#[test]
fn several_modules_may_share_an_explicit_c_link_name() {
    let dir = test_dir("shared-link-name");
    let module = "extern flow native_add as shared_add\n    <- a: i32\n    <- b: i32\n    => i32\n\nopen flow add_one\n    => i32\n{\n    => native_add(1, 2)\n}\n";
    write(&dir, "a.loi", module);
    write(&dir, "b.loi", module);
    let main = write(
        &dir,
        "main.loi",
        "import \"a.loi\" as a\nimport \"b.loi\" as b\nopen flow main\n    => i32\n{\n    => a::add_one() + b::add_one()\n}\n",
    );
    let output = run(&[Path::new("gen"), &main]);
    let c = text(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "shared link name was rejected: {c}"
    );
    assert_eq!(c.matches("shared_add(").count(), 4, "unexpected C calls");
}

#[test]
fn duplicate_names_inside_one_module_are_still_rejected() {
    let dir = test_dir("same-module");
    let main = write(
        &dir,
        "main.loi",
        "open flow helper\n    => i32\n{\n    => 1\n}\n\nopen flow helper\n    => i32\n{\n    => 2\n}\n\nopen flow main\n    => i32\n{\n    => helper()\n}\n",
    );
    let output = run(&[Path::new("check"), &main]);
    let message = text(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "duplicate name was accepted");
    assert!(
        message.contains("duplicate function 'helper'"),
        "unexpected diagnostics: {message}"
    );
}
