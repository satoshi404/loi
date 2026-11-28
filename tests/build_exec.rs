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
        "loi-build-exec-{}-{name}-{nanos}",
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

fn script(body: &str) -> String {
    format!(
        "import build::model\n\
         import build::exec\n\
         \n\
         open flow main\n\
             => i32\n\
         {{\n\
             protect {{\n\
                 mut project: model::Project\n\
                 model::project(\"demo\", -> project)\n\
                 mut app: u64\n\
                 model::executable(<-> project, \"app\", -> app)\n\
                 model::set_output(<-> project, app, \"output/app\")\n\
                 model::add_source(<-> project, app, \"src/app.loi\")\n\
         {body}\
             }}\n\
         }}\n"
    )
}

fn compile(dir: &Path, body: &str) -> PathBuf {
    let check = write(dir, "check.loi", &script(body));
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
            "exec script failed to compile: {}",
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

fn run_project(runner: &Path, dir: &Path) -> Output {
    run(runner, dir)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn source(name: &str) -> String {
    format!("open flow main\n    => i32\n{{\n    => 0\n}}\n// {name}\n")
}

fn body_run_project() -> String {
    format!(
        "                => exec::run_project(project, \"{}\")\n",
        loi()
    )
}

#[test]
fn exec_builds_a_target_and_writes_its_manifest() {
    if !has_c_compiler() {
        eprintln!("skipping exec_builds_a_target_and_writes_its_manifest: cc not found");
        return;
    }
    let dir = test_dir("build-one");
    write(&dir, "src/app.loi", &source("one"));
    let runner = compile(&dir, &body_run_project());
    let output = run_project(&runner, &dir);
    let manifest = fs::read_to_string(dir.join("output/.loi-build/app.cache")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(
        manifest.starts_with("loi-cache-v1 "),
        "bad manifest: {manifest}"
    );
}

#[test]
fn exec_skips_a_fresh_target_and_rebuilds_after_a_source_change() {
    if !has_c_compiler() {
        eprintln!(
            "skipping exec_skips_a_fresh_target_and_rebuilds_after_a_source_change: cc not found"
        );
        return;
    }
    let dir = test_dir("skip-and-rebuild");
    let path = write(&dir, "src/app.loi", &source("first"));
    let runner = compile(&dir, &body_run_project());
    let first = run_project(&runner, &dir);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    fs::write(dir.join("output/app"), "sentinel\n").expect("output should be writable");
    let second = run_project(&runner, &dir);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    let skipped = fs::read_to_string(dir.join("output/app")).unwrap_or_default();
    fs::write(&path, source("second")).expect("source should be writable");
    let _third = run_project(&runner, &dir);
    let rebuilt = fs::read_to_string(dir.join("output/app")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(skipped, "sentinel\n", "fresh artifact was rebuilt");
    assert_ne!(rebuilt, "sentinel\n", "changed source did not rebuild");
}

#[test]
fn exec_builds_every_target_in_the_project() {
    if !has_c_compiler() {
        eprintln!("skipping exec_builds_every_target_in_the_project: cc not found");
        return;
    }
    let dir = test_dir("two-targets");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/helper.loi", &source("helper"));
    let runner = compile(
        &dir,
        &format!(
            "                mut helper: u64\n\
                         model::object(<-> project, \"helper\", -> helper)\n\
                         model::set_output(<-> project, helper, \"output/helper.o\")\n\
                         model::add_source(<-> project, helper, \"src/helper.loi\")\n\
                         model::add_dep(<-> project, app, \"helper\")\n\
                         => exec::run_project(project, \"{}\")\n",
            loi()
        ),
    );
    let output = run_project(&runner, &dir);
    let app = dir.join("output/app").exists();
    let helper = dir.join("output/helper.o").exists();
    let app_manifest = dir.join("output/.loi-build/app.cache").exists();
    let helper_manifest = dir.join("output/.loi-build/helper.cache").exists();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(app && helper, "not every target was built");
    assert!(app_manifest && helper_manifest, "manifest missing");
}

#[test]
fn exec_stops_without_a_manifest_when_the_compiler_fails() {
    if !has_c_compiler() {
        eprintln!("skipping exec_stops_without_a_manifest_when_the_compiler_fails: cc not found");
        return;
    }
    let dir = test_dir("failed-build");
    write(
        &dir,
        "src/app.loi",
        "open flow main\n    => i32\n{\n    => 0\n",
    );
    let runner = compile(&dir, &body_run_project());
    let output = run_project(&runner, &dir);
    let manifest = dir.join("output/.loi-build/app.cache").exists();
    let _ = fs::remove_dir_all(&dir);
    assert!(
        !output.status.success(),
        "invalid source should fail the build"
    );
    assert!(!manifest, "failed build wrote a cache manifest");
}

#[test]
fn exec_accepts_a_noop_task() {
    if !has_c_compiler() {
        eprintln!("skipping exec_accepts_a_noop_task: cc not found");
        return;
    }
    let dir = test_dir("noop-task");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile(
        &dir,
        &format!(
            "                mut task: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"noop\", -> task)\n\
                         => exec::run_project(project, \"{}\")\n",
            loi()
        ),
    );
    let output = run_project(&runner, &dir);
    let built = dir.join("output/app").exists();
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(built, "noop task prevented the target build");
}

#[test]
fn exec_stops_before_a_target_when_a_declared_task_fails() {
    if !has_c_compiler() {
        eprintln!("skipping exec_stops_before_a_target_when_a_declared_task_fails: cc not found");
        return;
    }
    let dir = test_dir("task-order");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile(
        &dir,
        &format!(
            "                mut task: u64\n\
                         model::task(<-> project, \"gen\", \"generate\", \"run\", -> task)\n\
                         model::add_task_ref(<-> project, app, \"gen\")\n\
                         => exec::run_project(project, \"{}\")\n",
            loi()
        ),
    );
    let output = run_project(&runner, &dir);
    let text = stderr(&output);
    let built = dir.join("output/app").exists();
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "unsupported task should fail");
    assert!(
        text.contains("unsupported task operation"),
        "unexpected: {text}"
    );
    assert!(!built, "target ran before its failing dependency");
}

#[test]
fn exec_rejects_an_unsupported_task_operation() {
    if !has_c_compiler() {
        eprintln!("skipping exec_rejects_an_unsupported_task_operation: cc not found");
        return;
    }
    let dir = test_dir("unsupported-task");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile(
        &dir,
        &format!(
            "                mut task: u64\n\
                         model::task(<-> project, \"gen\", \"run\", \"arbitrary\", -> task)\n\
                         => exec::run_project(project, \"{}\")\n",
            loi()
        ),
    );
    let output = run_project(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        !output.status.success(),
        "unknown operation should be rejected"
    );
    assert!(
        text.contains("unsupported task operation"),
        "unexpected: {text}"
    );
}
