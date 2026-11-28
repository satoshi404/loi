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
        "loi-build-cache-{}-{name}-{nanos}",
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
            "cache script failed to compile: {}",
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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn script(body: &str) -> String {
    format!(
        "import build::model\n\
         import build::cache\n\
         import os\n\
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
                 => 0\n\
             }}\n\
         }}\n"
    )
}

fn source(name: &str) -> String {
    format!("open flow main\n    => i32\n{{\n    => 0\n}}\n// {name}\n")
}

fn compile_body(dir: &Path, body: &str) -> PathBuf {
    let check = write(dir, "check.loi", &script(body));
    compile(dir, &check).expect("runner")
}

#[test]
fn cache_key_is_stable_for_identical_projects() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_is_stable_for_identical_projects: cc not found");
        return;
    }
    let dir = test_dir("stable-key");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile_body(
        &dir,
        "                mut other: model::Project\n\
                         mut other_app: u64\n\
                         model::project(\"demo\", -> other)\n\
                         model::executable(<-> other, \"app\", -> other_app)\n\
                         model::set_output(<-> other, other_app, \"output/app\")\n\
                         model::add_source(<-> other, other_app, \"src/app.loi\")\n\
                         left <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         right <- cache::artifact_key(other, \"app\", \"cc\")\n\
                         if not str_eq(left, right) {\n\
                             panic(\"unstable cache key\\n\")\n\
                         }\n\
                         print(left)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(!text.trim().is_empty(), "empty cache key");
}

#[test]
fn cache_key_tracks_source_bytes() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_tracks_source_bytes: cc not found");
        return;
    }
    let dir = test_dir("source-bytes");
    let path = write(&dir, "src/app.loi", &source("first"));
    let runner = compile_body(
        &dir,
        "                key <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         print(key)\n\
                         print(\"\\n\")\n",
    );
    let first = stdout(&run(&runner, &dir));
    fs::write(&path, source("second")).expect("source should be writable");
    let second = stdout(&run(&runner, &dir));
    let _ = fs::remove_dir_all(&dir);
    assert_ne!(first, second, "source change did not invalidate the key");
}

#[test]
fn cache_source_change_forces_rebuild() {
    if !has_c_compiler() {
        eprintln!("skipping cache_source_change_forces_rebuild: cc not found");
        return;
    }
    let dir = test_dir("source-invalidation");
    let path = write(&dir, "src/app.loi", &source("first"));
    write(&dir, "output/app", "binary\n");
    let runner = compile_body(
        &dir,
        "                if not os_path_exists(\"output/.loi-build/app.cache\") {\n\
                             cache::write_manifest(project, \"app\", \"cc\")\n\
                         }\n\
                         stale <- cache::needs_build(project, \"app\", \"cc\")\n\
                         if stale { print(\"stale\\n\") } else { print(\"fresh\\n\") }\n",
    );
    let first = stdout(&run(&runner, &dir));
    fs::write(&path, source("second")).expect("source should be writable");
    let second = stdout(&run(&runner, &dir));
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(first, "fresh\n", "unchanged source should be fresh");
    assert_eq!(second, "stale\n", "changed source should be stale");
}

#[test]
fn cache_key_tracks_c_source_bytes() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_tracks_c_source_bytes: cc not found");
        return;
    }
    let dir = test_dir("shim-bytes");
    write(&dir, "src/app.loi", &source("app"));
    let shim = write(&dir, "src/shim.c", "int one(void) { return 1; }\n");
    let runner = compile_body(
        &dir,
        "                model::add_c_source(<-> project, app, \"src/shim.c\")\n\
                         key <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         print(key)\n\
                         print(\"\\n\")\n",
    );
    let first_output = run(&runner, &dir);
    assert!(
        first_output.status.success(),
        "first run failed: {}",
        stderr(&first_output)
    );
    let first = stdout(&first_output);
    fs::write(&shim, "int one(void) { return 2; }\n").expect("shim should be writable");
    let second_output = run(&runner, &dir);
    assert!(
        second_output.status.success(),
        "second run failed: {}",
        stderr(&second_output)
    );
    let second = stdout(&second_output);
    let _ = fs::remove_dir_all(&dir);
    assert_ne!(first, second, "c source change did not invalidate the key");
}

#[test]
fn cache_key_tracks_flags_and_compiler() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_tracks_flags_and_compiler: cc not found");
        return;
    }
    let dir = test_dir("flags-compiler");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile_body(
        &dir,
        "                before <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         model::add_cc_flag(<-> project, app, \"-O2\")\n\
                         after_flag <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         mut target <- vec_get<model::BuildTarget>(project.targets, app)\n\
                         target.config.compiler = \"clang\"\n\
                         vec_set<model::BuildTarget>(<-> project.targets, app, target)\n\
                         after_compiler <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         if str_eq(before, after_flag) {\n\
                             panic(\"cc flag did not invalidate the key\\n\")\n\
                         }\n\
                         if str_eq(after_flag, after_compiler) {\n\
                             panic(\"compiler did not invalidate the key\\n\")\n\
                         }\n\
                         print(after_compiler)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(!text.trim().is_empty(), "empty cache key");
}

#[test]
fn cache_key_tracks_defines_and_libs() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_tracks_defines_and_libs: cc not found");
        return;
    }
    let dir = test_dir("defines-libs");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile_body(
        &dir,
        "                before <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         model::add_define(<-> project, app, \"MODE\", \"fast\")\n\
                         after_define <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         model::add_lib(<-> project, app, \"m\")\n\
                         after_lib <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         if str_eq(before, after_define) {\n\
                             panic(\"define did not invalidate the key\\n\")\n\
                         }\n\
                         if str_eq(after_define, after_lib) {\n\
                             panic(\"lib did not invalidate the key\\n\")\n\
                         }\n\
                         print(after_lib)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(!text.trim().is_empty(), "empty cache key");
}

#[test]
fn cache_reports_stale_when_output_is_missing() {
    if !has_c_compiler() {
        eprintln!("skipping cache_reports_stale_when_output_is_missing: cc not found");
        return;
    }
    let dir = test_dir("missing-output");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile_body(
        &dir,
        "                cache::write_manifest(project, \"app\", \"cc\")\n\
                         stale <- cache::needs_build(project, \"app\", \"cc\")\n\
                         if stale { print(\"stale\\n\") } else { print(\"fresh\\n\") }\n",
    );
    let first = stdout(&run(&runner, &dir));
    write(&dir, "output/app", "binary\n");
    let second = stdout(&run(&runner, &dir));
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(first, "stale\n", "missing output should be stale");
    assert_eq!(second, "fresh\n", "complete output should be fresh");
}

#[test]
fn cache_invalid_manifest_forces_rebuild() {
    if !has_c_compiler() {
        eprintln!("skipping cache_invalid_manifest_forces_rebuild: cc not found");
        return;
    }
    let dir = test_dir("invalid-manifest");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "output/app", "binary\n");
    let runner = compile_body(
        &dir,
        "                stale <- cache::needs_build(project, \"app\", \"cc\")\n\
                         if stale { print(\"stale\\n\") } else { print(\"fresh\\n\") }\n",
    );
    let first = stdout(&run(&runner, &dir));
    write(&dir, "output/.loi-build/app.cache", "not-a-digest\n");
    let second = stdout(&run(&runner, &dir));
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(first, "stale\n", "missing manifest should be stale");
    assert_eq!(second, "stale\n", "invalid manifest should be stale");
}

#[test]
fn cache_rejects_missing_input() {
    if !has_c_compiler() {
        eprintln!("skipping cache_rejects_missing_input: cc not found");
        return;
    }
    let dir = test_dir("missing-input");
    let runner = compile_body(
        &dir,
        "                key <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         print(key)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stderr(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(!output.status.success(), "missing input should fail");
    assert!(text.contains("missing input"), "unexpected: {text}");
}

#[test]
fn cache_manifest_path_uses_declared_artifact() {
    if !has_c_compiler() {
        eprintln!("skipping cache_manifest_path_uses_declared_artifact: cc not found");
        return;
    }
    let dir = test_dir("manifest-path");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile_body(
        &dir,
        "                mut item: u64\n\
                         model::artifact(<-> project, \"app-manifest\", \"output/app\", \"executable\", \"app\", -> item)\n\
                         path <- cache::manifest_path(project, \"app\")\n\
                         print(path)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert_eq!(text, "output/.loi-build/app-manifest.json\n");
}

#[test]
fn cache_key_tracks_header_and_asset_bytes() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_tracks_header_and_asset_bytes: cc not found");
        return;
    }
    let dir = test_dir("header-asset");
    write(&dir, "src/app.loi", &source("app"));
    let header = write(&dir, "include/app.h", "#define VALUE 1\n");
    let asset = write(&dir, "assets/data.txt", "one\n");
    let runner = compile_body(
        &dir,
        "                model::add_header(<-> project, app, \"include/app.h\")\n\
                         model::add_asset(<-> project, app, \"assets/data.txt\")\n\
                         key <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         print(key)\n\
                         print(\"\\n\")\n",
    );
    let first_output = run(&runner, &dir);
    assert!(
        first_output.status.success(),
        "first run failed: {}",
        stderr(&first_output)
    );
    let first = stdout(&first_output);
    fs::write(&header, "#define VALUE 2\n").expect("header should be writable");
    let second_output = run(&runner, &dir);
    assert!(
        second_output.status.success(),
        "second run failed: {}",
        stderr(&second_output)
    );
    let second = stdout(&second_output);
    fs::write(&asset, "two\n").expect("asset should be writable");
    let third_output = run(&runner, &dir);
    assert!(
        third_output.status.success(),
        "third run failed: {}",
        stderr(&third_output)
    );
    let third = stdout(&third_output);
    let _ = fs::remove_dir_all(&dir);
    assert_ne!(first, second, "header change did not invalidate the key");
    assert_ne!(second, third, "asset change did not invalidate the key");
}

#[test]
fn cache_key_tracks_config_and_dependencies() {
    if !has_c_compiler() {
        eprintln!("skipping cache_key_tracks_config_and_dependencies: cc not found");
        return;
    }
    let dir = test_dir("config-dep");
    write(&dir, "src/app.loi", &source("app"));
    let runner = compile_body(
        &dir,
        "                before <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         mut target <- vec_get<model::BuildTarget>(project.targets, app)\n\
                         target.config.optimize = \"speed\"\n\
                         vec_set<model::BuildTarget>(<-> project.targets, app, target)\n\
                         after_config <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         mut helper: u64\n\
                         model::object(<-> project, \"helper\", -> helper)\n\
                         model::add_dep(<-> project, app, \"helper\")\n\
                         after_dep <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         if str_eq(before, after_config) {\n\
                             panic(\"config did not invalidate the key\\n\")\n\
                         }\n\
                         if str_eq(after_config, after_dep) {\n\
                             panic(\"dependency did not invalidate the key\\n\")\n\
                         }\n\
                         print(after_dep)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(!text.trim().is_empty(), "empty cache key");
}

#[test]
fn cache_keys_are_per_target() {
    if !has_c_compiler() {
        eprintln!("skipping cache_keys_are_per_target: cc not found");
        return;
    }
    let dir = test_dir("per-target");
    write(&dir, "src/app.loi", &source("app"));
    write(&dir, "src/helper.loi", &source("helper"));
    let runner = compile_body(
        &dir,
        "                mut helper: u64\n\
                         model::object(<-> project, \"helper\", -> helper)\n\
                         model::set_output(<-> project, helper, \"output/helper.o\")\n\
                         model::add_source(<-> project, helper, \"src/helper.loi\")\n\
                         left <- cache::artifact_key(project, \"app\", \"cc\")\n\
                         right <- cache::artifact_key(project, \"helper\", \"cc\")\n\
                         if str_eq(left, right) {\n\
                             panic(\"targets shared a cache key\\n\")\n\
                         }\n\
                         print(right)\n\
                         print(\"\\n\")\n",
    );
    let output = run(&runner, &dir);
    let text = stdout(&output);
    let _ = fs::remove_dir_all(&dir);
    assert!(output.status.success(), "unexpected: {}", stderr(&output));
    assert!(!text.trim().is_empty(), "empty cache key");
}
