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
        "loi-build-phase7-{}-{name}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn write(dir: &Path, name: &str, source: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent should be creatable");
    }
    fs::write(path, source).expect("source should be writable");
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
        "import std\n\
         import build::model\n\
         import build::exec\n\
         import build::config\n\
         import build::validate\n\
         import build::cache\n\
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
    let check = write_file(dir, "check.loi", &script);
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
            "phase7 script failed to compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    runner
}

fn write_file(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent should be creatable");
    }
    fs::write(&path, source).expect("source should be writable");
    path
}

fn run(runner: &Path, dir: &Path, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(runner);
    command.current_dir(dir);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("runner should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn finish(dir: &Path, output: &Output) {
    let _ = output;
    let _ = fs::remove_dir_all(dir);
}

fn exit_source(code: &str) -> String {
    format!("open flow main\n    => i32\n{{\n    => {code}\n}}\n")
}

#[test]
fn toolchain_defaults_are_explicit() {
    if !has_c_compiler() {
        eprintln!("skipping toolchain_defaults_are_explicit: cc not found");
        return;
    }
    let dir = test_dir("toolchain-defaults");
    let body = "                mut tools: model::BuildToolchain\n\
                          model::toolchain(-> tools)\n\
                          if str_eq(tools.host_os, \"native\") and str_eq(tools.host_arch, \"native\") and str_eq(tools.host_abi, \"native\") {\n\
                              if str_eq(tools.c_compiler, \"cc\") and str_eq(tools.linker, \"cc\") and str_eq(tools.archiver, \"ar\") {\n\
                                  => 0\n\
                              }\n\
                          }\n\
                          => 1\n";
    let runner = compile(&dir, body);
    let output = run(&runner, &dir, &[]);
    finish(&dir, &output);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
}

#[test]
fn project_profile_is_inherited_by_later_targets() {
    if !has_c_compiler() {
        eprintln!("skipping project_profile_is_inherited_by_later_targets: cc not found");
        return;
    }
    let dir = test_dir("profile-inherit");
    let body = "                config::apply_profile(<-> project, \"release\")\n\
                          mut app: u64\n\
                          model::executable(<-> project, \"app\", -> app)\n\
                          target <- vec_get<model::BuildTarget>(project.targets, app)\n\
                          if str_eq(target.config.profile, \"release\") {\n\
                              => 0\n\
                          }\n\
                          => 1\n";
    let runner = compile(&dir, body);
    let output = run(&runner, &dir, &[]);
    finish(&dir, &output);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
}

#[test]
fn explicit_target_config_beats_environment() {
    if !has_c_compiler() {
        eprintln!("skipping explicit_target_config_beats_environment: cc not found");
        return;
    }
    let dir = test_dir("env-precedence");
    let body = "                mut changes: Vec<[u8]>\n\
                          config::apply_env(<-> project, -> changes)\n\
                          mut app: u64\n\
                          model::executable(<-> project, \"app\", -> app)\n\
                          model::set_profile(<-> project, app, \"debug\")\n\
                          target <- vec_get<model::BuildTarget>(project.targets, app)\n\
                          if str_eq(target.config.profile, \"debug\") {\n\
                              => 0\n\
                          }\n\
                          => 1\n";
    let runner = compile(&dir, body);
    let output = run(&runner, &dir, &[("LOI_BUILD_PROFILE", "release")]);
    finish(&dir, &output);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
}

#[test]
fn environment_override_is_reported() {
    if !has_c_compiler() {
        eprintln!("skipping environment_override_is_reported: cc not found");
        return;
    }
    let dir = test_dir("env-report");
    let body = "                mut changes: Vec<[u8]>\n\
                          config::apply_env(<-> project, -> changes)\n\
                          if changes.len > 0 {\n\
                              => 0\n\
                          }\n\
                          => 1\n";
    let runner = compile(&dir, body);
    let output = run(&runner, &dir, &[("LOI_BUILD_PROFILE", "release")]);
    let text = stdout(&output);
    finish(&dir, &output);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(
        text.contains("[config] profile <- env LOI_BUILD_PROFILE=release"),
        "environment override was silent: {text}"
    );
}

#[test]
fn shared_library_is_a_declared_target_kind() {
    if !has_c_compiler() {
        eprintln!("skipping shared_library_is_a_declared_target_kind: cc not found");
        return;
    }
    let dir = test_dir("shared-library");
    write(&dir, "src/plugin.loi", &exit_source("0"));
    let body = "                mut plugin: u64\n\
                          model::shared_library(<-> project, \"plugin\", -> plugin)\n\
                          model::add_source(<-> project, plugin, \"src/plugin.loi\")\n\
                          model::set_output(<-> project, plugin, \"output/libplugin.so\")\n\
                          validate::validate_project(project)\n\
                          => 0\n";
    let runner = compile(&dir, body);
    let output = run(&runner, &dir, &[]);
    finish(&dir, &output);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
}

#[test]
fn cross_target_without_a_toolchain_is_rejected() {
    if !has_c_compiler() {
        eprintln!("skipping cross_target_without_a_toolchain_is_rejected: cc not found");
        return;
    }
    let dir = test_dir("cross-reject");
    write(&dir, "src/app.loi", &exit_source("0"));
    let body = format!(
        "                mut app: u64\n\
                          model::executable(<-> project, \"app\", -> app)\n\
                          model::add_source(<-> project, app, \"src/app.loi\")\n\
                          model::set_target_os(<-> project, app, \"wasm32\")\n\
                          model::set_target_arch(<-> project, app, \"wasm32\")\n\
                          model::set_target_abi(<-> project, app, \"wasm\")\n\
                          => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir, &[]);
    let text = stderr(&output);
    finish(&dir, &output);
    assert!(!output.status.success(), "cross target should be rejected");
    assert!(
        text.contains("cross toolchain"),
        "unexpected diagnostic: {text}"
    );
}

#[test]
fn toolchain_change_invalidates_the_artifact() {
    if !has_c_compiler() {
        eprintln!("skipping toolchain_change_invalidates_the_artifact: cc not found");
        return;
    }
    let dir = test_dir("toolchain-key");
    write(&dir, "src/app.loi", &exit_source("0"));
    let body = format!(
        "                mut changes: Vec<[u8]>\n\
                          config::apply_env(<-> project, -> changes)\n\
                          mut app: u64\n\
                          model::executable(<-> project, \"app\", -> app)\n\
                          model::add_source(<-> project, app, \"src/app.loi\")\n\
                          => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let first = run(&runner, &dir, &[("LOI_BUILD_C_COMPILER", "cc")]);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    let second = run(&runner, &dir, &[("LOI_BUILD_C_COMPILER", "clang")]);
    let text = stdout(&second);
    finish(&dir, &second);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert!(
        text.contains("[exec] building app"),
        "toolchain change did not invalidate: {text}"
    );
}

#[test]
fn transitive_dependency_change_rebuilds_downstream() {
    if !has_c_compiler() {
        eprintln!("skipping transitive_dependency_change_rebuilds_downstream: cc not found");
        return;
    }
    let dir = test_dir("transitive-cache");
    write(&dir, "src/engine.loi", &exit_source("0"));
    write(&dir, "src/game.loi", &exit_source("0"));
    let body = format!(
        "                mut engine: u64\n\
                          model::static_library(<-> project, \"engine\", -> engine)\n\
                          model::set_output(<-> project, engine, \"output/libengine.a\")\n\
                          model::add_source(<-> project, engine, \"src/engine.loi\")\n\
                          mut game: u64\n\
                          model::executable(<-> project, \"game\", -> game)\n\
                          model::add_source(<-> project, game, \"src/game.loi\")\n\
                          model::add_dep(<-> project, game, \"engine\")\n\
                          => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let first = run(&runner, &dir, &[]);
    assert!(first.status.success(), "unexpected: {}", stderr(&first));
    write(&dir, "src/engine.loi", &exit_source("1"));
    let second = run(&runner, &dir, &[]);
    let text = stdout(&second);
    finish(&dir, &second);
    assert!(second.status.success(), "unexpected: {}", stderr(&second));
    assert!(
        text.matches("[exec] building game").count() >= 1,
        "downstream target was not invalidated: {text}"
    );
}

#[test]
fn manifest_writes_do_not_leave_temporary_files() {
    if !has_c_compiler() {
        eprintln!("skipping manifest_writes_do_not_leave_temporary_files: cc not found");
        return;
    }
    let dir = test_dir("atomic-manifest");
    write(&dir, "src/app.loi", &exit_source("0"));
    let body = format!(
        "                mut app: u64\n\
                          model::executable(<-> project, \"app\", -> app)\n\
                          model::add_source(<-> project, app, \"src/app.loi\")\n\
                          => exec::run_project(project, \"{}\")\n",
        loi()
    );
    let runner = compile(&dir, &body);
    let output = run(&runner, &dir, &[]);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    let manifests = fs::read_dir(dir.join("output/.loi-build"))
        .expect("manifest directory should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    finish(&dir, &output);
    assert!(
        manifests.iter().any(|name| name == "app.cache"),
        "missing manifest: {manifests:?}"
    );
    assert!(
        !manifests.iter().any(|name| name.contains(".tmp")),
        "temporary manifest survived: {manifests:?}"
    );
}
