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
    let dir = std::env::temp_dir().join(format!(
        "loi-build-phase6-{}-{name}-{nanos}",
        std::process::id()
    ));
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

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn compile(dir: &Path, body: &str) -> PathBuf {
    let script = format!(
        "import build::model\n\
         import build::exec\n\
         \n\
         open flow main\n\
             => i32\n\
         {{\n\
             protect {{\n\
                 mut project: model::Project\n\
                 model::project(\"demo\", -> project)\n\
         {body}\n\
             }}\n\
         }}\n"
    );
    let check = write(dir, "check.loi", &script);
    let runner = dir.join("runner");
    let output = Command::new(loi())
        .arg("build")
        .arg(&check)
        .arg("-o")
        .arg(&runner)
        .arg("--auto-c-source")
        .arg("--c-source")
        .arg(repo_lib().join("os_shim.c"))
        .arg("--c-source")
        .arg(repo_lib().join("process_shim.c"))
        .arg("--lib-dir")
        .arg(repo_lib())
        .current_dir(dir)
        .output()
        .expect("loi build should run");
    if !output.status.success() {
        panic!(
            "phase6 script failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    runner
}

fn run(runner: &Path, dir: &Path) -> Output {
    Command::new(runner)
        .current_dir(dir)
        .output()
        .expect("runner should run")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn finish(dir: &Path, output: &Output) {
    let _ = fs::remove_dir_all(dir);
    let _ = output;
}

fn exit_source(code: &str) -> String {
    format!("open flow main\n    => i32\n{{\n    => {code}\n}}\n")
}

#[test]
fn asset_dir_copies_nested_files_and_rebuilds_after_a_change() {
    if !has_c_compiler() {
        eprintln!(
            "skipping asset_dir_copies_nested_files_and_rebuilds_after_a_change: cc not found"
        );
        return;
    }
    let dir = test_dir("asset-change");
    write(&dir, "assets/tree/a.txt", "one\n");
    write(&dir, "assets/tree/sub/b.txt", "two\n");
    let body = format!(
        "                mut item: u64\n\
                         model::asset_dir(<-> project, \"tree\", \"assets/tree\", \"output/assets/tree\", -> item)\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let first = run(&runner, &dir);
    let a = fs::read_to_string(dir.join("output/assets/tree/a.txt")).unwrap_or_default();
    let b = fs::read_to_string(dir.join("output/assets/tree/sub/b.txt")).unwrap_or_default();
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    assert_eq!(a, "one\n");
    assert_eq!(b, "two\n");

    write(&dir, "assets/tree/sub/b.txt", "changed\n");
    write(&dir, "output/assets/tree/sub/b.txt", "sentinel\n");
    let second = run(&runner, &dir);
    let b = fs::read_to_string(dir.join("output/assets/tree/sub/b.txt")).unwrap_or_default();
    finish(&dir, &second);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert_eq!(b, "changed\n", "changed asset did not invalidate the copy");
}

#[test]
fn asset_dir_is_skipped_when_the_declared_tree_is_fresh() {
    if !has_c_compiler() {
        eprintln!("skipping asset_dir_is_skipped_when_the_declared_tree_is_fresh: cc not found");
        return;
    }
    let dir = test_dir("asset-fresh");
    write(&dir, "assets/tree/a.txt", "one\n");
    let body = format!(
        "                mut item: u64\n\
                         model::asset_dir(<-> project, \"tree\", \"assets/tree\", \"output/assets/tree\", -> item)\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let first = run(&runner, &dir);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    write(&dir, "output/assets/tree/a.txt", "sentinel\n");
    let second = run(&runner, &dir);
    let text = fs::read_to_string(dir.join("output/assets/tree/a.txt")).unwrap_or_default();
    finish(&dir, &second);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert_eq!(text, "sentinel\n", "fresh asset tree was recopied");
}

#[test]
fn asset_dir_rejects_a_missing_source_without_writing() {
    if !has_c_compiler() {
        eprintln!("skipping asset_dir_rejects_a_missing_source_without_writing: cc not found");
        return;
    }
    let dir = test_dir("asset-missing");
    let body = format!(
        "                mut item: u64\n\
                         model::asset_dir(<-> project, \"tree\", \"assets/missing\", \"output/assets/tree\", -> item)\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let written = dir.join("output/assets/tree").exists();
    finish(&dir, &output);
    assert!(!output.status.success(), "missing asset source should fail");
    assert!(text.contains("asset source"), "unexpected: {text}");
    assert!(!written, "missing asset source wrote an output directory");
}

#[test]
fn embed_file_generates_bytes_that_a_target_compiles() {
    if !has_c_compiler() {
        eprintln!("skipping embed_file_generates_bytes_that_a_target_compiles: cc not found");
        return;
    }
    let dir = test_dir("embed");
    write(&dir, "assets/config.bin", "abc");
    write(
        &dir,
        "output/generated/app.loi",
        "open import \"data.loi\"\n\
         open flow main\n\
             => i32\n\
         {\n\
             if _loi_embed_config.len == 3 {\n\
                 => 7\n\
             }\n\
             => 1\n\
         }\n",
    );
    let body = format!(
        "                mut data: u64\n\
                         model::embed_file(<-> project, \"config\", \"assets/config.bin\", \"output/generated/data.loi\", -> data)\n\
                         mut app: u64\n\
                         model::executable(<-> project, \"app\", -> app)\n\
                         model::set_output(<-> project, app, \"output/app\")\n\
                         model::add_source(<-> project, app, \"output/generated/app.loi\")\n\
                         model::add_asset(<-> project, app, \"output/generated/data.loi\")\n\
                         model::add_task_ref(<-> project, app, \"config\")\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let binary = dir.join("output/app");
    let generated = fs::read_to_string(dir.join("output/generated/data.loi")).unwrap_or_default();
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(binary.exists(), "target did not build from embedded bytes");
    let app = Command::new(&binary)
        .current_dir(&dir)
        .output()
        .expect("app should run");
    assert_eq!(
        app.status.code(),
        Some(7),
        "unexpected generated bytes: {generated}"
    );

    write(&dir, "assets/config.bin", "xy");
    write(&dir, "output/generated/data.loi", "sentinel\n");
    let second = run(&runner, &dir);
    let generated = fs::read_to_string(dir.join("output/generated/data.loi")).unwrap_or_default();
    let app = Command::new(&binary)
        .current_dir(&dir)
        .output()
        .expect("app should run");
    finish(&dir, &second);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert!(
        generated.contains("[u8: 120, 121]"),
        "embedded bytes were not regenerated: {generated}"
    );
    assert_eq!(app.status.code(), Some(1), "stale embedded bytes survived");
}

#[test]
fn run_tests_executes_only_the_named_test() {
    if !has_c_compiler() {
        eprintln!("skipping run_tests_executes_only_the_named_test: cc not found");
        return;
    }
    let dir = test_dir("test-filter");
    write(&dir, "tests/parser.loi", &exit_source("0"));
    write(&dir, "tests/checker.loi", &exit_source("0"));
    let body = format!(
        "                mut parser: u64\n\
                         model::test(<-> project, \"parser\", \"tests/parser.loi\", -> parser)\n\
                         mut checker: u64\n\
                         model::test(<-> project, \"checker\", \"tests/checker.loi\", -> checker)\n\
                         => exec::run_tests(project, \"{}\", \"parser\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let checker_built = dir.join("output/tests/checker").exists();
    finish(&dir, &output);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(text.contains("test parser"), "unexpected: {text}");
    assert!(!checker_built, "the unselected test binary was built");
}

#[test]
fn a_failing_test_reports_its_output_and_stops() {
    if !has_c_compiler() {
        eprintln!("skipping a_failing_test_reports_its_output_and_stops: cc not found");
        return;
    }
    let dir = test_dir("test-fail");
    write(
        &dir,
        "tests/parser.loi",
        "open flow main\n    => i32\n{\n    eprint(\"BOOM\\n\")\n    => 3\n}\n",
    );
    write(&dir, "tests/checker.loi", &exit_source("0"));
    let body = format!(
        "                mut parser: u64\n\
                         model::test(<-> project, \"parser\", \"tests/parser.loi\", -> parser)\n\
                         mut checker: u64\n\
                         model::test(<-> project, \"checker\", \"tests/checker.loi\", -> checker)\n\
                         => exec::run_tests(project, \"{}\", \"\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let checker_built = dir.join("output/tests/checker").exists();
    finish(&dir, &output);
    assert!(
        !output.status.success(),
        "failing test should fail the build"
    );
    assert!(
        text.contains("BOOM"),
        "child stderr was not forwarded: {text}"
    );
    assert!(text.contains("test parser failed"), "unexpected: {text}");
    assert!(!checker_built, "a later test ran after the failure");
}

#[test]
fn unknown_test_filter_is_rejected() {
    if !has_c_compiler() {
        eprintln!("skipping unknown_test_filter_is_rejected: cc not found");
        return;
    }
    let dir = test_dir("test-unknown");
    write(&dir, "tests/parser.loi", &exit_source("0"));
    let body = format!(
        "                mut parser: u64\n\
                         model::test(<-> project, \"parser\", \"tests/parser.loi\", -> parser)\n\
                         => exec::run_tests(project, \"{}\", \"missing\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    finish(&dir, &output);
    assert!(!output.status.success(), "unknown filter should fail");
    assert!(
        text.contains("unknown test 'missing'"),
        "unexpected: {text}"
    );
}

#[test]
fn install_copies_a_declared_target_outside_output() {
    if !has_c_compiler() {
        eprintln!("skipping install_copies_a_declared_target_outside_output: cc not found");
        return;
    }
    let dir = test_dir("install");
    write(&dir, "src/app.loi", &exit_source("7"));
    let body = format!(
        "                mut app: u64\n\
                         model::executable(<-> project, \"app\", -> app)\n\
                         model::set_output(<-> project, app, \"output/app\")\n\
                         model::add_source(<-> project, app, \"src/app.loi\")\n\
                         mut install_index: u64\n\
                         model::install(<-> project, \"release\", \"output/app\", \"dist/app\", -> install_index)\n\
                         status <- exec::run_project(project, \"{}\")\n\
                          if status != 0 {{\n\
                              => status\n\
                          }}\n\
                          => exec::run_install(project, \"{}\")\n",
        loi(),
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let installed = dir.join("dist/app");
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(installed.exists(), "install did not copy the target");
    let app = Command::new(&installed)
        .current_dir(&dir)
        .output()
        .expect("installed app should run");
    finish(&dir, &output);
    assert_eq!(app.status.code(), Some(7), "installed bytes differ");
}

#[test]
fn clean_removes_declared_outputs_and_manifests() {
    if !has_c_compiler() {
        eprintln!("skipping clean_removes_declared_outputs_and_manifests: cc not found");
        return;
    }
    let dir = test_dir("clean");
    write(&dir, "src/app.loi", &exit_source("0"));
    let build = format!(
        "                mut app: u64\n\
                         model::executable(<-> project, \"app\", -> app)\n\
                         model::set_output(<-> project, app, \"output/app\")\n\
                         model::add_source(<-> project, app, \"src/app.loi\")\n\
                         => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &build);
    let first = run(&runner, &dir);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    assert!(dir.join("output/app").exists());
    assert!(dir.join("output/.loi-build/app.cache").exists());

    let clean_body = "                mut all: u64\n\
                      model::clean(<-> project, \"all\", -> all)\n\
                      model::add_clean_path(<-> project, all, \"output/app\")\n\
                      model::add_clean_path(<-> project, all, \"output/.loi-build/app.cache\")\n\
                      => exec::run_clean(project)\n";
    let cleaner = compile(&dir, clean_body);
    let second = run(&cleaner, &dir);
    let binary = dir.join("output/app").exists();
    let manifest = dir.join("output/.loi-build/app.cache").exists();
    finish(&dir, &second);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert!(!binary, "clean left the target behind");
    assert!(!manifest, "clean left the manifest behind");
}

#[test]
fn install_rejects_a_path_outside_the_project() {
    if !has_c_compiler() {
        eprintln!("skipping install_rejects_a_path_outside_the_project: cc not found");
        return;
    }
    let dir = test_dir("install-escape");
    let body = format!(
        "                mut install_index: u64\n\
                         model::install(<-> project, \"release\", \"src/app.loi\", \"../outside\", -> install_index)\n\
                         => exec::run_install(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir);
    let text = stderr(&output);
    finish(&dir, &output);
    assert!(
        !output.status.success(),
        "escaping install should be rejected"
    );
    assert!(
        text.contains("path must stay inside the project"),
        "unexpected: {text}"
    );
}
