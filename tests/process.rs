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
        PathBuf::from("/tmp/opencode").join(format!("loi-process-{}-{name}", std::process::id()));
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

fn c_source(name: &str) -> PathBuf {
    repo_root().join("lib").join(name)
}

fn check(program: &Path) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(program)
        .output()
        .expect("loi should run")
}

fn build(program: &Path, output: &Path, shims: &[&str]) -> Output {
    let mut command = Command::new(loi());
    command.arg("build").arg(program).arg("-o").arg(output);
    for shim in shims {
        command.arg("--c-source").arg(c_source(shim));
    }
    command.output().expect("loi build should run")
}

fn run_program(dir: &Path, name: &str, source: &str, shims: &[&str]) -> Output {
    let program = dir.join("main.loi");
    let checked_source = source.replace("{LIB}", &repo_root().join("lib").to_string_lossy());
    fs::write(&program, checked_source).expect("program should be writable");
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = dir.join(name);
    let built = build(&program, &executable, shims);
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
fn process_library_captures_stdout_stderr_and_exit_code() {
    if !has_c_compiler() {
        eprintln!("skipping process_library_captures_stdout_stderr_and_exit_code: cc not found");
        return;
    }
    let dir = test_dir("capture");
    let source = r#"
import "{LIB}/process.loi"

open flow main
    => i32
{
    protect {
        guard child <- process_spawn("printf out; printf err >&2; exit 7") {
            mut out: [u8]
            alloc(-> out, u8, process_stdout_len(child) as u64)
            process_read_stdout(child, <-> out)
            mut err: [u8]
            alloc(-> err, u8, process_stderr_len(child) as u64)
            process_read_stderr(child, <-> err)
            print(out)
            print("|")
            print(err)
            print("|")
            print_int(process_exit_code(child))
            print("\n")
        }
    }
    => 0
}
"#;
    let output = run_program(&dir, "capture", source, &["process_shim.c"]);
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "out|err|7\n");
}

#[test]
fn process_spawn_args_quotes_a_program_path_and_its_arguments() {
    if !has_c_compiler() {
        eprintln!(
            "skipping process_spawn_args_quotes_a_program_path_and_its_arguments: cc not found"
        );
        return;
    }
    let dir = test_dir("quotes");
    let nested = dir.join("a dir");
    fs::create_dir_all(&nested).expect("nested directory should be creatable");
    let echo = nested.join("say tool.sh");
    fs::write(&echo, "#!/bin/sh\nprintf '%s' \"$1\"\n").expect("script should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&echo)
            .expect("metadata should work")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&echo, permissions).expect("permissions should be set");
    }
    let echo = echo
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    let source = format!(
        r#"
import "{{LIB}}/process.loi"
import "{{LIB}}/vec.loi"

open flow main
    => i32
{{
    protect {{
        mut args: Vec<[u8]>
        vec_new<[u8]>(-> args)
        mut arg: [u8]
        alloc(-> arg, u8, 5)
        arg = "a'b c"
        vec_push<[u8]>(<-> args, arg)
        guard child <- process_spawn_args("{echo}", vec_view<[u8]>(<- args, 0, args.len)) {{
            mut out: [u8]
            alloc(-> out, u8, process_stdout_len(child) as u64)
            process_read_stdout(child, <-> out)
            print(out)
        }}
    }}
    => 0
}}
"#
    )
    .replace("{LIB}", &repo_root().join("lib").to_string_lossy());
    let program = dir.join("main.loi");
    fs::write(&program, source).expect("program should be writable");
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = dir.join("program");
    let built = build(&program, &executable, &["process_shim.c"]);
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output = Command::new(&executable)
        .output()
        .expect("program should run");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a'b c");
}

#[test]
fn process_library_captures_large_output_without_deadlock() {
    if !has_c_compiler() {
        eprintln!("skipping process_library_captures_large_output_without_deadlock: cc not found");
        return;
    }
    let dir = test_dir("large");
    let source = r#"
import "{LIB}/process.loi"

open flow main
    => i32
{
    protect {
        guard child <- process_spawn("yes x | head -c 200000") {
            print_int(process_stdout_len(child))
            print("\n")
        }
    }
    => 0
}
"#;
    let output = run_program(&dir, "large", source, &["process_shim.c"]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "200000\n");
}

#[test]
fn process_library_reports_a_failed_spawn() {
    if !has_c_compiler() {
        eprintln!("skipping process_library_reports_a_failed_spawn: cc not found");
        return;
    }
    let dir = test_dir("failed");
    let source = r#"
import "{LIB}/process.loi"
import "{LIB}/vec.loi"

open flow main
    => i32
{
    protect {
        mut args: Vec<[u8]>
        vec_new<[u8]>(-> args)
        guard child <- process_spawn_args("/no/such/loi/compiler", vec_view<[u8]>(<- args, 0, args.len)) {
            print_int(process_exit_code(child))
            print("\n")
        }
    }
    => 0
}
"#;
    let output = run_program(&dir, "failed", source, &["process_shim.c"]);
    assert!(output.status.success());
    // `/bin/sh -c` starts successfully but cannot find the program, so the
    // failure is reported through the conventional shell exit status 127.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "127\n");
}

#[test]
fn os_env_reads_runtime_byte_strings() {
    if !has_c_compiler() {
        eprintln!("skipping os_env_reads_runtime_byte_strings: cc not found");
        return;
    }
    let dir = test_dir("os-env");
    let source = r#"
import "{LIB}/os.loi"

open flow main
    => i32
{
    protect {
        present <- os_env("LOI_TEST_STRING")
        print(present)
        print("|")
        missing <- os_env("LOI_TEST_MISSING_VARIABLE")
        print_int(missing.len)
        print("\n")
    }
    => 0
}
"#;
    let program = dir.join("main.loi");
    fs::write(
        &program,
        source.replace("{LIB}", &repo_root().join("lib").to_string_lossy()),
    )
    .expect("program should be writable");
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = dir.join("program");
    let built = build(&program, &executable, &["os_shim.c"]);
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output = Command::new(&executable)
        .env("LOI_TEST_STRING", "hello from env")
        .output()
        .expect("program should run");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "hello from env|0\n"
    );
}

#[test]
fn process_release_is_generated_by_guard() {
    if !has_c_compiler() {
        eprintln!("skipping process_release_is_generated_by_guard: cc not found");
        return;
    }
    let dir = test_dir("release");
    let program = dir.join("main.loi");
    fs::write(
        &program,
        format!(
            "import \"{}/process.loi\"\n\nopen flow main\n    => i32\n{{\n    guard child <- process_spawn(\"true\") {{\n        print_int(process_exit_code(child))\n    }}\n    => 0\n}}\n",
            repo_root().join("lib").to_string_lossy()
        ),
    )
    .expect("program should be writable");
    let generated = dir.join("main.c");
    let output = Command::new(loi())
        .arg("gen")
        .arg(&program)
        .arg("-o")
        .arg(&generated)
        .output()
        .expect("loi gen should run");
    assert!(
        output.status.success(),
        "gen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let c = fs::read_to_string(&generated).expect("generated C should be readable");
    assert!(
        c.contains("loi_fn_process_release(&(loi_v_child))"),
        "missing synthesized process release in:\n{c}"
    );
}
