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
    let dir =
        PathBuf::from("/tmp/opencode").join(format!("loi-handles-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn write_program(dir: &Path, imports: &[&str], body: &str) -> PathBuf {
    let mut source = String::new();
    for name in imports {
        source.push_str(&format!(
            "import \"{}\"\n",
            repo_root().join("lib").join(name).display()
        ));
    }
    source.push_str(body);
    let path = dir.join("main.loi");
    fs::write(&path, source).expect("program should be writable");
    path
}

fn check(path: &Path) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(path)
        .output()
        .expect("loi should run")
}

fn generate(path: &Path, dir: &Path) -> String {
    let c_path = dir.join("generated.c");
    let output = Command::new(loi())
        .arg("gen")
        .arg(path)
        .arg("-o")
        .arg(&c_path)
        .output()
        .expect("loi gen should run");
    assert!(
        output.status.success(),
        "gen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::read_to_string(&c_path).expect("generated C should be readable")
}

fn build_and_run(path: &Path, dir: &Path) -> Output {
    let executable = dir.join("program");
    let mut build = Command::new(loi());
    build.arg("build").arg(path).arg("-o").arg(&executable);
    for shim in ["os_shim.c", "net_shim.c"] {
        build
            .arg("--c-source")
            .arg(repo_root().join("lib").join(shim));
    }
    let built = build.output().expect("loi build should run");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    Command::new(&executable)
        .output()
        .expect("built program should run")
}

#[test]
fn file_handle_guard_writes_and_closes_the_file() {
    let dir = test_dir("file-write");
    let path = dir.join("data.txt");
    let body = format!(
        "open flow main\n    => i32\n{{\n    guard out <- file_open_write(\"{}\") {{\n        written <- file_write(<- out, \"hello guard\\n\")\n        if written != 12 {{ => 1 }}\n    }}\n    => 0\n}}\n",
        path.display()
    );
    let program = write_program(&dir, &["os.loi"], &body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let run = build_and_run(&program, &dir);
    assert_eq!(
        run.status.code(),
        Some(0),
        "program failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        fs::read_to_string(&path).expect("file should exist"),
        "hello guard\n"
    );
    let output = generate(&program, &dir);
    assert!(
        output.contains("loi_fn_file_release(&(loi_v_out))"),
        "missing synthesized file release in:\n{output}"
    );
}

#[test]
fn file_handle_guard_reads_back_a_file_inside_protect() {
    let dir = test_dir("file-read");
    let path = dir.join("data.txt");
    fs::write(&path, "from disk").expect("fixture should be writable");
    let body = format!(
        "open flow main\n    => i32\n{{\n    protect {{\n        mut data: [u8]\n        alloc(-> data, u8, 32)\n        guard input <- file_open_read(\"{}\") {{\n            n <- file_read(<- input, <-> data)\n            if n != 9 {{ => 1 }}\n            if not (data[0] == 102 and data[8] == 107) {{ => 2 }}\n        }}\n    }}\n    => 0\n}}\n",
        path.display()
    );
    let program = write_program(&dir, &["os.loi"], &body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let run = build_and_run(&program, &dir);
    assert_eq!(
        run.status.code(),
        Some(0),
        "program failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn tcp_stream_handle_guard_is_released_on_return() {
    let dir = test_dir("tcp-stream");
    let body = "open flow main\n    => i32\n{\n    guard stream <- tcp_stream_connect(\"127.0.0.1\", 0) {\n        if stream.fd < 0 {\n            => 0\n        }\n        if not tcp_stream_send(<- stream, \"ping\") {\n            => 1\n        }\n    }\n    => 2\n}\n";
    let program = write_program(&dir, &["net.loi"], body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let output = generate(&program, &dir);
    assert!(
        output.contains("loi_fn_tcp_stream_release(&(loi_v_stream))"),
        "missing synthesized stream release in:\n{output}"
    );
    let run = build_and_run(&program, &dir);
    assert_eq!(
        run.status.code(),
        Some(0),
        "program failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn a_raw_descriptor_is_not_a_guard_resource() {
    let dir = test_dir("raw-descriptor");
    let body = "open flow main\n    => i32\n{\n    guard fd <- net_tcp_connect(\"127.0.0.1\", 0) {\n    }\n    => 0\n}\n";
    let program = write_program(&dir, &["net.loi"], body);
    let checked = check(&program);
    assert!(!checked.status.success(), "raw i32 must not be guardable");
    assert!(
        String::from_utf8_lossy(&checked.stderr)
            .contains("guard requires a named shape with a release flow"),
        "unexpected diagnostic: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn a_guarded_file_handle_cannot_be_released_explicitly() {
    let dir = test_dir("double-release");
    let body = "open flow main\n    => i32\n{\n    guard out <- file_open_write(\"/tmp/opencode/loi-handles-unused.txt\") {\n        file_release(<-> out)\n    }\n    => 0\n}\n";
    let program = write_program(&dir, &["os.loi"], body);
    let checked = check(&program);
    assert!(
        !checked.status.success(),
        "explicit release must be rejected"
    );
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains("released automatically by 'guard'"),
        "unexpected diagnostic: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}
