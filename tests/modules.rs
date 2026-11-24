use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-modules-{}-{name}", std::process::id()));
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

const UTIL: &str = "open flow answer\n    => i32\n{\n    => 42\n}\n\nsealed flow secret\n    => i32\n{\n    => 7\n}\n";

#[test]
fn qualified_open_item_from_an_aliased_module_is_callable() {
    let dir = test_dir("qualified-open");
    write(&dir, "util.loi", UTIL);
    let main = write(
        &dir,
        "main.loi",
        "import \"util.loi\" as u\nopen flow main\n    => i32\n{\n    => u::answer()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "check failed: {}", stderr(&output));
}

#[test]
fn plain_import_binds_the_file_stem_as_a_module_name() {
    let dir = test_dir("file-stem");
    write(&dir, "util.loi", UTIL);
    let main = write(
        &dir,
        "main.loi",
        "import \"util.loi\"\nopen flow main\n    => i32\n{\n    => util::answer()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "check failed: {}", stderr(&output));
}

#[test]
fn qualified_sealed_item_is_rejected() {
    let dir = test_dir("qualified-sealed");
    write(&dir, "util.loi", UTIL);
    let main = write(
        &dir,
        "main.loi",
        "import \"util.loi\" as u\nopen flow main\n    => i32\n{\n    => u::secret()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "sealed item was accepted");
    assert!(text.contains("sealed"), "unexpected diagnostics: {text}");
    assert!(text.contains("secret"), "unexpected diagnostics: {text}");
}

#[test]
fn unqualified_sealed_item_from_another_module_is_rejected() {
    let dir = test_dir("unqualified-sealed");
    write(&dir, "util.loi", UTIL);
    let main = write(
        &dir,
        "main.loi",
        "import \"util.loi\"\nopen flow main\n    => i32\n{\n    => secret()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "sealed item was accepted");
    assert!(text.contains("sealed"), "unexpected diagnostics: {text}");
}

#[test]
fn a_module_may_use_its_own_sealed_items() {
    let dir = test_dir("own-sealed");
    let main = write(
        &dir,
        "main.loi",
        "sealed flow helper\n    => i32\n{\n    => 9\n}\nopen flow main\n    => i32\n{\n    => helper() + main::helper()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "check failed: {}", stderr(&output));
}

#[test]
fn qualified_types_and_constructors_resolve() {
    let dir = test_dir("qualified-types");
    write(
        &dir,
        "geo.loi",
        "open shape Point [\n    x: i32\n]\nopen mark Color [ Red, Green ]\n",
    );
    let main = write(
        &dir,
        "main.loi",
        "import \"geo.loi\" as g\nopen flow main\n    <- p: g::Point\n    <- c: g::Color\n    => i32\n{\n    q <- g::Point[ x: p.x ]\n    if c == g::Color.Red { => q.x }\n    => 0\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "check failed: {}", stderr(&output));
}

#[test]
fn unknown_module_is_rejected() {
    let dir = test_dir("unknown-module");
    let main = write(
        &dir,
        "main.loi",
        "open flow main\n    => i32\n{\n    => missing::answer()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "unknown module was accepted");
    assert!(
        text.contains("unknown module 'missing'"),
        "unexpected diagnostics: {text}"
    );
}

#[test]
fn missing_item_in_a_module_is_rejected() {
    let dir = test_dir("missing-item");
    write(&dir, "util.loi", UTIL);
    let main = write(
        &dir,
        "main.loi",
        "import \"util.loi\" as u\nopen flow main\n    => i32\n{\n    => u::absent()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "missing item was accepted");
    assert!(
        text.contains("module 'u' has no item 'absent'"),
        "unexpected diagnostics: {text}"
    );
}

#[test]
fn duplicate_module_aliases_are_rejected() {
    let dir = test_dir("duplicate-alias");
    write(&dir, "a.loi", "open flow a\n    => i32\n{\n    => 1\n}\n");
    write(&dir, "b.loi", "open flow b\n    => i32\n{\n    => 2\n}\n");
    let main = write(
        &dir,
        "main.loi",
        "import \"a.loi\" as m\nimport \"b.loi\" as m\nopen flow main\n    => i32\n{\n    => m::a()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "duplicate alias was accepted");
    assert!(
        text.contains("duplicate module alias 'm'"),
        "unexpected diagnostics: {text}"
    );
}

#[test]
fn import_cycles_are_rejected() {
    let dir = test_dir("cycle");
    write(
        &dir,
        "a.loi",
        "import \"b.loi\"\nopen flow a\n    => i32\n{\n    => 1\n}\n",
    );
    write(
        &dir,
        "b.loi",
        "import \"a.loi\"\nopen flow b\n    => i32\n{\n    => 2\n}\n",
    );
    let main = write(
        &dir,
        "main.loi",
        "import \"a.loi\"\nopen flow main\n    => i32\n{\n    => a() + b()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "import cycle was accepted");
    assert!(
        text.contains("import cycle"),
        "unexpected diagnostics: {text}"
    );
}

#[test]
fn diamond_imports_still_deduplicate() {
    let dir = test_dir("diamond");
    write(
        &dir,
        "leaf.loi",
        "open flow leaf\n    => i32\n{\n    => 3\n}\n",
    );
    write(
        &dir,
        "left.loi",
        "import \"leaf.loi\"\nopen flow left\n    => i32\n{\n    => leaf()\n}\n",
    );
    write(
        &dir,
        "right.loi",
        "import \"leaf.loi\"\nopen flow right\n    => i32\n{\n    => leaf()\n}\n",
    );
    let main = write(
        &dir,
        "main.loi",
        "import \"left.loi\" as l\nimport \"right.loi\" as r\nopen flow main\n    => i32\n{\n    => l::left() + r::right()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "check failed: {}", stderr(&output));
}

#[test]
fn multi_file_build_with_aliases_links_and_runs() {
    let dir = test_dir("multi-file");
    write(&dir, "util.loi", UTIL);
    let first = write(
        &dir,
        "first.loi",
        "import \"util.loi\" as u\nopen flow first\n    => i32\n{\n    => u::answer()\n}\n\nopen flow main\n    => i32\n{\n    => first()\n}\n",
    );
    let second = write(
        &dir,
        "second.loi",
        "import \"util.loi\"\nopen flow second\n    => i32\n{\n    => util::answer() + 1\n}\n",
    );
    let out = dir.join("program");

    let output = Command::new(loi())
        .arg("build")
        .arg(&first)
        .arg(&second)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("loi build should run");
    assert!(output.status.success(), "build failed: {}", stderr(&output));

    let run_output = Command::new(&out).output().expect("program should run");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(run_output.status.code(), Some(42));
}

#[test]
fn a_sealed_release_flow_from_another_module_is_rejected_by_guard() {
    let dir = test_dir("sealed-guard-release");
    write(
        &dir,
        "resource.loi",
        "open shape Handle [\n    fd: i32\n]\n\nsealed flow handle_release\n    <-> handle: Handle\n{\n}\n\nopen flow make_handle\n    => Handle\n{\n    => Handle[ fd: 7 ]\n}\n",
    );
    let main = write(
        &dir,
        "main.loi",
        "import \"resource.loi\"\nopen flow main\n    => i32\n{\n    guard handle <- make_handle() {\n    }\n    => 0\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "sealed release was accepted");
    assert!(text.contains("sealed"), "unexpected diagnostics: {text}");
    assert!(
        text.contains("handle_release"),
        "unexpected diagnostics: {text}"
    );
}

#[test]
fn a_sealed_release_flow_is_rejected_for_a_shape_literal_guard() {
    let dir = test_dir("sealed-guard-literal");
    write(
        &dir,
        "resource.loi",
        "open shape Handle [\n    fd: i32\n]\n\nsealed flow handle_release\n    <-> handle: Handle\n{\n}\n",
    );
    let main = write(
        &dir,
        "main.loi",
        "import \"resource.loi\"\nopen flow main\n    => i32\n{\n    guard handle <- Handle[ fd: 7 ] {\n    }\n    => 0\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "sealed release was accepted");
    assert!(
        text.contains("handle_release") && text.contains("sealed"),
        "unexpected diagnostics: {text}"
    );
}

const OPTION_MODULE: &str = "open flow option_answer\n    => i32\n{\n    => 42\n}\n\nsealed flow option_secret\n    => i32\n{\n    => 7\n}\n";

fn package_dir(name: &str, package: &str) -> PathBuf {
    let dir = test_dir(name);
    fs::create_dir_all(dir.join(package)).expect("package directory should be creatable");
    dir
}

#[test]
fn open_import_reexports_open_items_through_a_package_root() {
    let dir = package_dir("package-reexport", "core");
    write(&dir, "core/option.loi", OPTION_MODULE);
    write(&dir, "core.loi", "open import \"core/option.loi\"\n");
    let _main = write(
        &dir,
        "main.loi",
        "import \"core.loi\" as core\nopen flow main\n    => i32\n{\n    => core::option_answer()\n}\n",
    );

    let build = Command::new(loi())
        .args(["build", "main.loi", "-o", "app"])
        .current_dir(&dir)
        .output()
        .expect("loi should build");
    assert!(build.status.success(), "build failed: {}", stderr(&build));
    let run = Command::new(dir.join("app"))
        .output()
        .expect("program should run");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(run.status.code(), Some(42));
}

#[test]
fn open_import_does_not_reexport_sealed_items() {
    let dir = package_dir("package-reexport-sealed", "core");
    write(&dir, "core/option.loi", OPTION_MODULE);
    write(&dir, "core.loi", "open import \"core/option.loi\"\n");
    let main = write(
        &dir,
        "main.loi",
        "import \"core.loi\" as core\nopen flow main\n    => i32\n{\n    => core::option_secret()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "sealed item was re-exported");
    assert!(text.contains("sealed"), "unexpected diagnostics: {text}");
    assert!(
        text.contains("option_secret"),
        "unexpected diagnostics: {text}"
    );
}

#[test]
fn reexports_keep_the_original_c_symbol() {
    let dir = package_dir("package-reexport-abi", "core");
    write(&dir, "core/option.loi", OPTION_MODULE);
    write(&dir, "core.loi", "open import \"core/option.loi\"\n");
    let _main = write(
        &dir,
        "main.loi",
        "import \"core.loi\" as core\nopen flow main\n    => i32\n{\n    => core::option_answer()\n}\n",
    );
    let c = dir.join("main.c");

    let output = Command::new(loi())
        .args(["gen", "main.loi", "-o"])
        .arg(&c)
        .current_dir(&dir)
        .output()
        .expect("loi should generate C");
    let text = if output.status.success() {
        fs::read_to_string(&c).expect("generated C should be readable")
    } else {
        stderr(&output)
    };
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "gen failed: {text}");
    assert!(
        text.contains("loi_fn_option_answer("),
        "unexpected C: {text}"
    );
    assert!(
        !text.contains("loi_fn_core__option_answer"),
        "re-export changed the C symbol: {text}"
    );
}

#[test]
fn open_import_cycles_are_rejected() {
    let dir = package_dir("package-reexport-cycle", "core");
    write(
        &dir,
        "core/option.loi",
        "open import \"../core.loi\"\nopen flow option_answer\n    => i32\n{\n    => 42\n}\n",
    );
    write(&dir, "core.loi", "open import \"core/option.loi\"\n");
    let main = write(
        &dir,
        "main.loi",
        "import \"core.loi\" as core\nopen flow main\n    => i32\n{\n    => core::option_answer()\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "re-export cycle was accepted");
    assert!(
        text.contains("import cycle detected"),
        "unexpected diagnostics: {text}"
    );
}

const GUARDED_RESOURCE: &str = "open shape HttpClient { id: i32 }\n\nopen flow http_client_release\n    <-> c: HttpClient\n{\n    c.id = 0\n}\n\nopen flow make_client\n    => HttpClient\n{\n    => HttpClient { id: 7 }\n}\n";

#[test]
fn guard_with_qualified_shape_resolves_its_release_flow() {
    let dir = test_dir("qualified-guard");
    write(&dir, "a.loi", GUARDED_RESOURCE);
    write(&dir, "b.loi", GUARDED_RESOURCE);
    let main = write(
        &dir,
        "main.loi",
        "import \"a.loi\" as a\nimport \"b.loi\" as b\nopen flow main\n    => i32\n{\n    guard c: a::HttpClient <- a::make_client() {\n        print_int(c.id)\n    }\n    guard d: b::HttpClient <- b::make_client() {\n        print_int(d.id)\n    }\n    => 0\n}\n",
    );

    let output = run(&[Path::new("check"), &main]);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "qualified guard could not resolve its release flow: {text}"
    );
}
