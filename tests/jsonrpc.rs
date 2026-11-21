use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn test_dir(name: &str) -> PathBuf {
    let dir =
        PathBuf::from("/tmp/opencode").join(format!("loi-jsonrpc-{}-{name}", std::process::id()));
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

fn check(path: &Path) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(path)
        .output()
        .expect("loi should run")
}

fn write_program(dir: &Path, source: impl AsRef<str>) -> PathBuf {
    let path = dir.join("main.loi");
    fs::write(&path, source.as_ref()).expect("program should be writable");
    path
}

fn build_with_shim(dir: &Path, program: &Path) -> PathBuf {
    let checked = check(program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = dir.join("program");
    let built = Command::new(loi())
        .arg("build")
        .arg(program)
        .arg("-o")
        .arg(&executable)
        .arg("--c-source")
        .arg(repo_root().join("lib").join("stdio_shim.c"))
        .arg("--c-source")
        .arg(repo_root().join("lib").join("process_shim.c"))
        .arg("--c-source")
        .arg(repo_root().join("lib").join("os_shim.c"))
        .arg("--c-source")
        .arg(repo_root().join("lib").join("lsp_shim.c"))
        .output()
        .expect("loi build should run");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    executable
}

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

struct RunResult {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Feed `input`, close stdin so the server sees EOF, and put a hard limit on
/// the wait so a transport that still waits for a live client cannot hang.
fn run_with_input(executable: &Path, input: &[u8]) -> RunResult {
    let child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("built program should run");
    let child = Arc::new(Mutex::new(child));

    {
        let mut guard = child.lock().expect("child lock should work");
        let stdin = guard.stdin.as_mut().expect("stdin pipe should exist");
        stdin.write_all(input).expect("input should be written");
        stdin.flush().expect("input should be flushed");
    }
    child.lock().expect("child lock should work").stdin.take();

    let reader_child = Arc::clone(&child);
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stdout, mut stderr) = {
            let mut guard = reader_child.lock().expect("child lock should work");
            (
                guard.stdout.take().expect("stdout pipe should exist"),
                guard.stderr.take().expect("stderr pipe should exist"),
            )
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        let _ = stderr.read_to_end(&mut err);
        let status = reader_child
            .lock()
            .expect("child lock should work")
            .wait()
            .ok();
        let _ = sender.send(RunResult {
            status,
            stdout: out,
            stderr: err,
        });
    });

    let result = receiver.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        let _ = child.lock().expect("child lock should work").kill();
    }
    result.expect("server should exit when stdin reaches EOF")
}

fn assert_server(dir: &Path, input: &[u8], expected: &[u8]) {
    let executable = build_with_shim(dir, &repo_root().join("examples").join("lsp_server.loi"));
    let run = run_with_input(&executable, input);
    let _ = fs::remove_dir_all(dir);
    assert_eq!(run.status.and_then(|status| status.code()), Some(0));
    assert!(
        run.stderr.is_empty(),
        "server wrote stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        expected,
        "stdout mismatch: {}",
        String::from_utf8_lossy(&run.stdout)
    );
}

#[test]
fn lsp_server_answers_initialize_and_shutdown() {
    if !has_c_compiler() {
        eprintln!("skipping lsp_server_answers_initialize_and_shutdown: cc not found");
        return;
    }
    let mut input = frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input.extend(frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#,
    ));
    let mut expected = frame(
        r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"textDocumentSync":1,"publishDiagnosticsProvider":true}}}"#,
    );
    expected.extend(frame(r#"{"jsonrpc":"2.0","id":2,"result":null}"#));
    assert_server(&test_dir("initialize-shutdown"), &input, &expected);
}

#[test]
fn lsp_server_reports_an_unknown_method() {
    if !has_c_compiler() {
        eprintln!("skipping lsp_server_reports_an_unknown_method: cc not found");
        return;
    }
    let input = frame(r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#);
    let expected =
        frame(r#"{"jsonrpc":"2.0","id":9,"error":{"code":-32601,"message":"Method not found"}}"#);
    assert_server(&test_dir("unknown-method"), &input, &expected);
}

#[test]
fn lsp_server_reports_a_parse_error_for_malformed_json() {
    if !has_c_compiler() {
        eprintln!("skipping lsp_server_reports_a_parse_error_for_malformed_json: cc not found");
        return;
    }
    let input = frame("{");
    let expected =
        frame(r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32700,"message":"Parse error"}}"#);
    assert_server(&test_dir("parse-error"), &input, &expected);
}

#[test]
fn jsonrpc_read_frame_rejects_a_header_without_content_length() {
    if !has_c_compiler() {
        eprintln!("skipping jsonrpc_read_frame_rejects_a_header_without_content_length");
        return;
    }
    let dir = test_dir("missing-length");
    let program = write_program(
        &dir,
        format!(
            "import \"{}/jsonrpc.loi\"\n\nopen flow main\n    => i32\n{{\n    protect {{\n        mut body: [u8]\n        ok <- jsonrpc_read_frame(-> body)\n        if ok {{\n            print(\"unexpected frame\\n\")\n            => 1\n        }}\n        print(\"no-frame\\n\")\n    }}\n    => 0\n}}\n",
            repo_root().join("lib").to_string_lossy()
        ),
    );
    let executable = build_with_shim(&dir, &program);
    let run = run_with_input(&executable, b"Content-Type: application/json\r\n\r\n{}");
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(run.status.and_then(|status| status.code()), Some(0));
    assert_eq!(run.stdout, b"no-frame\n");
}

#[test]
fn jsonrpc_parse_request_outside_protect_is_rejected() {
    let dir = test_dir("outside-protect");
    let program = write_program(
        &dir,
        format!(
            "import \"{}/jsonrpc.loi\"\n\nopen flow main\n    => i32\n{{\n    body: [u8] <- \"{{}}\"\n    mut request: JsonRpcRequest\n    ok <- jsonrpc_parse_request(body, -> request)\n    print_int(ok as i32)\n    => 0\n}}\n",
            repo_root().join("lib").to_string_lossy()
        ),
    );
    let checked = check(&program);
    let stderr = String::from_utf8_lossy(&checked.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);
    assert!(!checked.status.success(), "parse without protect must fail");
    assert!(
        stderr.contains("protect") || stderr.contains("allocation"),
        "unexpected diagnostic: {stderr}"
    );
}
