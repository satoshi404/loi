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
    let dir = PathBuf::from("/tmp/opencode").join(format!("loi-lsp-{}-{name}", std::process::id()));
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

fn run_with_input(executable: &Path, input: &[u8], compiler: &str) -> RunResult {
    let child = Command::new(executable)
        .env("LOI_COMPILER", compiler)
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
    let run = run_with_input(&executable, input, loi());
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

/// Ask the real compiler for JSON diagnostics about a broken Loi program. The
/// server repeats this check itself; the harness keeps the result as an oracle.
fn compiler_diagnostics(dir: &Path, source: &str) -> (PathBuf, String) {
    let source_path = dir.join("broken.loi");
    fs::write(&source_path, source).expect("source should be writable");
    let output = Command::new(loi())
        .arg("check")
        .arg(&source_path)
        .arg("--error-format")
        .arg("json")
        .arg("--color")
        .arg("never")
        .output()
        .expect("loi check should run");
    (source_path, String::from_utf8(output.stderr).unwrap())
}

fn did_open(uri: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"loi","version":1,"text":""}}}}}}"#
    )
}

fn published(uri: &str, diagnostics: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}","diagnostics":[{diagnostics}]}}}}"#
    )
}

#[test]
fn lsp_server_publishes_real_compiler_diagnostics() {
    if !has_c_compiler() {
        eprintln!("skipping lsp_server_publishes_real_compiler_diagnostics: cc not found");
        return;
    }
    let dir = test_dir("publish-real");
    let (_, json) = compiler_diagnostics(&dir, "flow main => i32 {\n    => missing\n}\n");
    assert!(
        json.contains(r#""severity":"error""#),
        "compiler should report an error: {json}"
    );
    let uri = format!("file://{}", dir.join("broken.loi").to_string_lossy());

    let mut input = frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input.extend(frame(&did_open(&uri)));

    // Compiler spans are 1-based; LSP line and character are 0-based.
    let diagnostic = r#"{"range":{"start":{"line":1,"character":7},"end":{"line":1,"character":14}},"severity":1,"message":"unknown variable 'missing'"}"#;
    let mut expected = frame(r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#);
    expected.extend(frame(&published(&uri, diagnostic)));

    assert_server(&dir, &input, &expected);
}

#[test]
fn lsp_server_clears_diagnostics_for_a_clean_document() {
    if !has_c_compiler() {
        eprintln!("skipping lsp_server_clears_diagnostics_for_a_clean_document: cc not found");
        return;
    }
    let dir = test_dir("publish-empty");
    let (_, json) = compiler_diagnostics(&dir, "flow main => i32 {\n    => 0\n}\n");
    assert!(
        json.is_empty(),
        "clean source should produce no diagnostics: {json}"
    );
    let uri = format!("file://{}", dir.join("broken.loi").to_string_lossy());

    let input = frame(&did_open(&uri));
    let expected = frame(&published(&uri, ""));
    assert_server(&dir, &input, &expected);
}

#[test]
fn lsp_server_accepts_notifications_without_an_id() {
    if !has_c_compiler() {
        eprintln!("skipping lsp_server_accepts_notifications_without_an_id: cc not found");
        return;
    }
    let dir = test_dir("notification-no-id");
    let mut input = frame(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);
    input.extend(frame(r#"{"jsonrpc":"2.0","id":7,"method":"shutdown"}"#));
    let expected = frame(r#"{"jsonrpc":"2.0","id":7,"result":null}"#);
    assert_server(&dir, &input, &expected);
}

#[test]
fn lsp_publish_diagnostics_outside_protect_is_rejected() {
    let dir = test_dir("outside-protect");
    let program = dir.join("main.loi");
    fs::write(
        &program,
        format!(
            "import \"{}/lsp.loi\"\n\nopen flow main\n    => i32\n{{\n    text: [u8] <- \"[]\"\n    uri: [u8] <- \"file:///x.loi\"\n    lsp_publish_diagnostics(uri, text)\n    => 0\n}}\n",
            repo_root().join("lib").to_string_lossy()
        ),
    )
    .expect("program should be writable");
    let checked = check(&program);
    let stderr = String::from_utf8_lossy(&checked.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);
    assert!(
        !checked.status.success(),
        "publish without protect must fail"
    );
    assert!(
        stderr.contains("protect") || stderr.contains("allocation"),
        "unexpected diagnostic: {stderr}"
    );
}
