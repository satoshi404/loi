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
        "loi-build-model-{}-{name}-{nanos}",
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

fn compile(dir: &Path, script: &Path) -> Option<PathBuf> {
    let runner = dir.join("runner");
    let output = Command::new(loi())
        .arg("build")
        .arg(script)
        .arg("-o")
        .arg(&runner)
        .arg("--auto-c-source")
        .arg("--lib-dir")
        .arg(repo_lib())
        .current_dir(dir)
        .output()
        .expect("loi build should run");
    if !output.status.success() {
        panic!(
            "model script failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Some(runner)
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

fn model_script(body: &str) -> String {
    format!(
        "import build::model\n\
         import build::validate\n\
         \n\
         open flow main\n\
             => i32\n\
         {{\n\
             protect {{\n\
                 mut project: model::Project\n\
                 model::project(\"demo\", -> project)\n\
         {body}\
                 => validate::validate_project(project)\n\
             }}\n\
         }}\n"
    )
}

fn one_target_script(settings: &str) -> String {
    model_script(&format!(
        "                mut target: u64\n\
                         model::executable(<-> project, \"app\", -> target)\n\
                         model::add_source(<-> project, target, \"src/main.loi\")\n\
         {settings}"
    ))
}

#[test]
fn model_validates_a_single_target_project() {
    if !has_c_compiler() {
        eprintln!("skipping model_validates_a_single_target_project: cc not found");
        return;
    }
    let dir = test_dir("single-target");
    write(
        &dir,
        "src/main.loi",
        "open flow main\n    => i32\n{\n    => 0\n}\n",
    );
    let script = write(&dir, "script.loi", &one_target_script(""));
    let runner = compile(&dir, &script).expect("runner");
    let output = run(&runner, &dir);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "valid project was rejected: {}",
        stderr(&output)
    );
}

#[test]
fn model_exposes_explicit_config_and_context_types() {
    if !has_c_compiler() {
        eprintln!("skipping model_exposes_explicit_config_and_context_types: cc not found");
        return;
    }
    let dir = test_dir("config-context");
    write(
        &dir,
        "src/main.loi",
        "open flow main\n    => i32\n{\n    => 0\n}\n",
    );
    let script = write(
        &dir,
        "script.loi",
        &model_script(
            "                mut settings: model::BuildConfig\n\
                             model::config(-> settings)\n\
                             mut ctx: model::BuildContext\n\
                             model::context(\".\", \"lib\", -> ctx)\n\
                             model::set_context(<-> project, ctx)\n\
                             mut target: u64\n\
                             model::executable(<-> project, \"app\", -> target)\n\
                             model::add_source(<-> project, target, \"src/main.loi\")\n\
                             model::default_target(<-> project, \"app\")\n",
        ),
    );
    let runner = compile(&dir, &script).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "explicit config/context should validate: {text}"
    );
}

#[test]
fn model_rejects_duplicate_target_ids() {
    if !has_c_compiler() {
        eprintln!("skipping model_rejects_duplicate_target_ids: cc not found");
        return;
    }
    let dir = test_dir("duplicate-target");
    write(
        &dir,
        "src/main.loi",
        "open flow main\n    => i32\n{\n    => 0\n}\n",
    );
    let script = write(
        &dir,
        "script.loi",
        &model_script(
            "                mut first: u64\n\
                             mut second: u64\n\
                             model::executable(<-> project, \"app\", -> first)\n\
                             model::executable(<-> project, \"app\", -> second)\n\
                             model::add_source(<-> project, first, \"src/main.loi\")\n\
                             model::add_source(<-> project, second, \"src/main.loi\")\n",
        ),
    );
    let runner = compile(&dir, &script).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1), "unexpected exit: {text}");
    assert!(
        text.contains("duplicate target 'app'"),
        "unexpected: {text}"
    );
}

#[test]
fn model_rejects_duplicate_task_ids() {
    if !has_c_compiler() {
        eprintln!("skipping model_rejects_duplicate_task_ids: cc not found");
        return;
    }
    let dir = test_dir("duplicate-task");
    let script = write(
        &dir,
        "script.loi",
        &model_script(
            "                mut first: u64\n\
                             mut second: u64\n\
                             model::task(<-> project, \"gen\", \"generate\", \"noop\", -> first)\n\
                             model::task(<-> project, \"gen\", \"generate\", \"noop\", -> second)\n",
        ),
    );
    let runner = compile(&dir, &script).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1), "unexpected exit: {text}");
    assert!(text.contains("duplicate task 'gen'"), "unexpected: {text}");
}

#[test]
fn model_rejects_missing_input() {
    if !has_c_compiler() {
        eprintln!("skipping model_rejects_missing_input: cc not found");
        return;
    }
    let dir = test_dir("missing-input");
    let script = write(&dir, "script.loi", &one_target_script(""));
    let runner = compile(&dir, &script).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1), "unexpected exit: {text}");
    assert!(
        text.contains("missing input 'src/main.loi'"),
        "unexpected: {text}"
    );
}

#[test]
fn model_rejects_invalid_config() {
    if !has_c_compiler() {
        eprintln!("skipping model_rejects_invalid_config: cc not found");
        return;
    }
    let dir = test_dir("invalid-config");
    write(
        &dir,
        "src/main.loi",
        "open flow main\n    => i32\n{\n    => 0\n}\n",
    );
    let script = write(
        &dir,
        "script.loi",
        &one_target_script(
            "                model::set_target_os(<-> project, target, \"plan9\")\n",
        ),
    );
    let runner = compile(&dir, &script).expect("runner");
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(output.status.code(), Some(1), "unexpected exit: {text}");
    assert!(text.contains("invalid target_os"), "unexpected: {text}");
}

#[test]
fn model_preserves_the_legacy_plan_surface() {
    let dir = test_dir("legacy-surface");
    let script = write(
        &dir,
        "build.loi",
        "import build as loi_build\n\
         \n\
         open flow main\n\
             => i32\n\
         {\n\
             protect {\n\
                 mut plan: loi_build::BuildPlan\n\
                 loi_build::new(\"demo\", -> plan)\n\
                 loi_build::source(<-> plan, \"src/main.loi\")\n\
                 loi_build::lib(<-> plan, \"m\")\n\
                 loi_build::cc_flag(<-> plan, \"-DLEGACY=1\")\n\
                 loi_build::target(<-> plan, \"exe\")\n\
                 loi_build::profile(<-> plan, \"debug\")\n\
                 loi_build::optimize(<-> plan, \"none\")\n\
                 loi_build::mode(<-> plan, \"native\")\n\
                 => loi_build::run(plan)\n\
             }\n\
         }\n",
    );
    let output = Command::new(loi())
        .arg("check")
        .arg(&script)
        .arg("--lib-dir")
        .arg(repo_lib())
        .current_dir(&dir)
        .output()
        .expect("loi check should run");
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "legacy plan surface should still check: {text}"
    );
}
