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
        "loi-build-phase8-{}-{name}-{nanos}",
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

fn compile_runner(dir: &Path, script: &str) -> PathBuf {
    let path = write(dir, "runner.loi", script);
    let runner = dir.join("runner");
    let output = Command::new(loi())
        .arg("build")
        .arg(&path)
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
    assert!(
        output.status.success(),
        "runner failed to compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    runner
}

fn run(runner: &Path, dir: &Path) -> Output {
    Command::new(runner)
        .current_dir(dir)
        .output()
        .expect("runner should run")
}

#[test]
fn optional_graph_api_compiles_without_breaking_legacy_build() {
    let dir = test_dir("optional-graph-api");
    write(
        &dir,
        "src/main.loi",
        "open flow main\n    => i32\n{\n    => 0\n}\n",
    );
    let legacy_script = "import build\n\nopen flow main\n    => i32\n{\n    protect {\n        mut plan: build::BuildPlan\n        build::new(\"demo\", -> plan)\n        build::source(<-> plan, \"src/main.loi\")\n        => build::run(plan)\n    }\n}\n";
    write(&dir, "build.loi", legacy_script);
    fs::create_dir_all(dir.join("output")).expect("output directory should be creatable");
    let legacy = compile_runner(&dir, legacy_script);
    let legacy_out = Command::new(&legacy)
        .current_dir(&dir)
        .env("LOI_SELF", loi())
        .env("LOI_TOOLCHAIN_LIB", repo_lib())
        .output()
        .expect("legacy build runner should run");
    assert!(
        legacy_out.status.success(),
        "legacy build runner failed: {}",
        String::from_utf8_lossy(&legacy_out.stderr)
    );
    let _ = fs::remove_dir_all(&dir);

    let dir2 = test_dir("optional-graph-api-new");
    write(
        &dir2,
        "src/main.loi",
        "open flow main\n    => i32\n{\n    => 0\n}\n",
    );
    let modern_script = format!(
        "import build::model\nimport build::config\nimport build::exec\n\n\
         open flow main\n    => i32\n{{\n    protect {{\n        \
         mut project: model::Project\n        model::project(\"demo\", -> project)\n        \
         config::apply_profile(<-> project, \"release\")\n        mut app: u64\n        \
         model::executable(<-> project, \"app\", -> app)\n        \
         model::add_source(<-> project, app, \"src/main.loi\")\n        \
         => exec::run_project(project, \"{}\")\n    }}\n}}\n",
        loi()
    );
    let modern = compile_runner(&dir2, &modern_script);
    let modern_out = run(&modern, &dir2);
    let _ = fs::remove_dir_all(&dir2);
    assert!(
        modern_out.status.success(),
        "optional graph runner failed: {}",
        String::from_utf8_lossy(&modern_out.stderr)
    );
}

#[test]
fn generated_scaffold_still_uses_legacy_build_plan() {
    let dir = test_dir("scaffold-compat");
    let init = Command::new(loi())
        .arg("init")
        .arg("demo")
        .current_dir(&dir)
        .output()
        .expect("loi init should run");
    assert!(
        init.status.success(),
        "loi init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let build = fs::read_to_string(dir.join("demo/build.loi")).expect("build.loi should exist");
    assert!(
        build.contains("import build as loi_build"),
        "scaffold does not import the legacy build module: {build}"
    );
    assert!(
        build.contains("loi_build::BuildPlan"),
        "scaffold does not use the legacy BuildPlan: {build}"
    );
    assert!(
        !build.contains("build::model"),
        "scaffold unexpectedly uses the optional graph model: {build}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn documentation_describes_the_optional_graph_and_legacy_compatibility() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let readme = fs::read_to_string(root.join("lib/README.md")).expect("lib README");
    let syntax = fs::read_to_string(root.join("SINTAXE.md")).expect("SINTAXE");
    let decisions = fs::read_to_string(root.join("development/DECISIONS.md")).expect("DECISIONS");
    let progress = fs::read_to_string(root.join("development/PROGRESS.md")).expect("PROGRESS");
    assert!(
        readme.contains("build::model") && readme.contains("build::exec"),
        "lib/README.md does not describe the optional graph API"
    );
    assert!(
        readme.contains("loi make") && readme.contains("legacy"),
        "lib/README.md does not state that loi make remains legacy"
    );
    assert!(
        syntax.contains("build::model") && syntax.contains("BuildGraph"),
        "SINTAXE.md does not document BuildGraph"
    );
    assert!(
        decisions.contains("FASE 7") && decisions.contains("FASE 8"),
        "DECISIONS.md has no FASE 7/8 decision records"
    );
    assert!(
        progress.contains("FASE 7") && progress.contains("FASE 8"),
        "PROGRESS.md has no FASE 7/8 status"
    );
}

#[test]
fn configuration_precedence_is_documented() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let readme = fs::read_to_string(root.join("lib/README.md")).expect("lib README");
    let syntax = fs::read_to_string(root.join("SINTAXE.md")).expect("SINTAXE");
    for text in [&readme, &syntax] {
        assert!(
            text.contains("LOI_BUILD_PROFILE"),
            "configuration environment variable is not documented"
        );
        assert!(
            text.contains("explicit"),
            "explicit-over-environment precedence is not documented"
        );
    }
}
