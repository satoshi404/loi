use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-packages-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory should be creatable");
    for entry in fs::read_dir(source).expect("source directory should be readable") {
        let entry = entry.expect("directory entry should be readable");
        let name = entry.file_name();
        let destination = target.join(&name);
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), &destination).expect("file should be copied");
        }
    }
}

fn check(program: &Path) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(program)
        .output()
        .expect("loi should run")
}

fn build_and_run_example(dir: &Path, name: &str) -> Option<Output> {
    let program = dir.join("examples").join(name);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "{name} did not check: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if !has_c_compiler() {
        return None;
    }

    let executable = dir.join("program");
    let mut build = Command::new(loi());
    build
        .args(["build", "examples/main.loi", "-o"])
        .arg(&executable)
        .current_dir(dir);
    for shim in [
        "lsp_shim.c",
        "net_shim.c",
        "os_shim.c",
        "process_shim.c",
        "stdio_shim.c",
    ] {
        build
            .arg("--c-source")
            .arg(repo_root().join("lib").join(shim));
    }
    let built = build.output().expect("loi build should run");
    assert!(
        built.status.success(),
        "{name} did not build: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    Some(
        Command::new(&executable)
            .output()
            .expect("built program should run"),
    )
}

#[test]
fn package_roots_reexport_public_items_without_changing_owners() {
    let dir = test_dir("roots");
    let root = repo_root().join("lib");
    let program = dir.join("main.loi");
    let source = format!(
        "import \"{}\"\nimport \"{}\"\nimport \"{}\"\nimport \"{}\"\nimport \"{}\"\nimport \"{}\"\n\
         open flow main\n    => i32\n{{\n\
             mut maybe: Option<i32>\n\
             core::option_some<i32>(7, -> maybe)\n\
             bounded <- std::clamp_i32(20, 0, 10)\n\
             value <- math::math_lerp(0.0, 1.0, 0.5)\n\
             port <- net::net_port_is_valid(80)\n\
             absolute <- os::path_is_absolute(\"/tmp/a\")\n\
             if core::option_get_or<i32>(maybe, 0) != 7 {{ => 1 }}\n\
             if bounded != 10 {{ => 2 }}\n\
             if not (value > 0.4 and value < 0.6) {{ => 3 }}\n\
             if not port {{ => 4 }}\n\
             if not absolute {{ => 5 }}\n\
             mut hooks: RuntimeHooks\n\
             runtime::hooks_none(-> hooks)\n\
             if runtime::hooks_wants_setup(hooks) {{ => 6 }}\n\
             => 0\n\
         }}\n",
        root.join("core.loi").display(),
        root.join("std.loi").display(),
        root.join("runtime.loi").display(),
        root.join("math.loi").display(),
        root.join("net.loi").display(),
        root.join("os.loi").display(),
    );
    fs::write(&program, source).expect("program should be writable");

    let output = check(&program);
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "package roots failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn package_examples_build_and_run() {
    for name in [
        "package_core.loi",
        "package_std.loi",
        "package_runtime.loi",
        "package_os.loi",
        "package_net.loi",
        "package_math.loi",
        "package_encoding.loi",
    ] {
        let dir = test_dir(name.trim_end_matches(".loi"));
        let examples = dir.join("examples");
        fs::create_dir_all(&examples).expect("examples directory should be creatable");
        copy_tree(&repo_root().join("lib"), &dir.join("lib"));
        fs::copy(
            repo_root().join("examples").join(name),
            examples.join("main.loi"),
        )
        .expect("example should be copied");

        if let Some(run) = build_and_run_example(&dir, "main.loi") {
            assert_eq!(
                run.status.code(),
                Some(0),
                "{name} failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
