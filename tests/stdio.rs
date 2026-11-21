use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
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
        PathBuf::from("/tmp/opencode").join(format!("loi-stdio-{}-{name}", std::process::id()));
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

fn write_program(dir: &Path, source: impl AsRef<str>) -> PathBuf {
    let path = dir.join("main.loi");
    fs::write(&path, source.as_ref()).expect("program should be writable");
    path
}

fn check(path: &Path) -> std::process::Output {
    Command::new(loi())
        .arg("check")
        .arg(path)
        .output()
        .expect("loi should run")
}

fn build(dir: &Path, program: &Path) -> PathBuf {
    let executable = dir.join("program");
    let output = Command::new(loi())
        .arg("build")
        .arg(program)
        .arg("-o")
        .arg(&executable)
        .arg("--c-source")
        .arg(repo_root().join("lib").join("stdio_shim.c"))
        .output()
        .expect("loi build should run");
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}

struct RunResult {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Feed `input` to the child and read its output while its stdin pipe stays
/// open when `keep_open` is true. A timeout keeps a wrong EOF-waiting
/// implementation from hanging the suite.
fn run_with_input(executable: &Path, input: &[u8], keep_open: bool) -> RunResult {
    let child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("built program should run");
    let child = Arc::new(Mutex::new(child));

    let mut stdin = Some({
        let mut guard = child.lock().expect("child lock should work");
        guard.stdin.take().expect("stdin pipe should exist")
    });
    stdin
        .as_mut()
        .expect("stdin pipe should exist")
        .write_all(input)
        .expect("input should be written");
    stdin
        .as_mut()
        .expect("stdin pipe should exist")
        .flush()
        .expect("input should be flushed");
    if !keep_open {
        stdin.take();
    }

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
    drop(stdin);
    if result.is_err() {
        let _ = child.lock().expect("child lock should work").kill();
    }
    let result = result.expect("child should finish without waiting for stdin EOF");
    assert!(
        result.stderr.is_empty(),
        "program wrote stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[test]
fn stdio_library_reads_exact_frames_and_flushes_without_eof() {
    if !has_c_compiler() {
        eprintln!("skipping stdio_library_reads_exact_frames_and_flushes_without_eof");
        return;
    }
    let dir = test_dir("frame-echo");
    let program = repo_root().join("examples").join("stdio_frames.loi");
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "example should check: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = build(&dir, &program);

    let run = run_with_input(
        &executable,
        b"Content-Length: 5\r\n\r\nhelloContent-Length: 0\r\n\r\n",
        true,
    );
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(run.status.and_then(|status| status.code()), Some(0));
    assert_eq!(run.stdout, b"Content-Length: 5\r\n\r\nhello");
}

#[test]
fn stdio_library_reports_a_short_read_at_eof() {
    if !has_c_compiler() {
        eprintln!("skipping stdio_library_reports_a_short_read_at_eof");
        return;
    }
    let dir = test_dir("short-read");
    let program = write_program(
        &dir,
        r#"import "LIB/stdio.loi"

open flow main
    => i32
{
    protect {
        mut data: [u8]
        alloc(-> data, u8, 8)
        got <- stdio_read_exact(<-> data)
        printf("got=%lld:", got)
        stdio_write_all(view(data, 0, got as u64))
        stdio_flush()
    }
    => 0
}
"#
        .replace("LIB", repo_root().join("lib").to_string_lossy().as_ref()),
    );
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "program should check: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = build(&dir, &program);

    let run = run_with_input(&executable, b"abc", false);
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(run.status.and_then(|status| status.code()), Some(0));
    assert_eq!(run.stdout, b"got=3:abc");
}

#[test]
fn stdio_library_rejects_a_non_writable_read_buffer() {
    let dir = test_dir("readonly-buffer");
    let program = write_program(
        &dir,
        r#"import "LIB/stdio.loi"

open flow main
    => i32
{
    data: [u8] <- [u8: 1, 2, 3]
    got <- stdio_read_exact(<- data)
    print_int(got)
    => 0
}
"#
        .replace("LIB", repo_root().join("lib").to_string_lossy().as_ref()),
    );
    let checked = check(&program);
    let stderr = String::from_utf8_lossy(&checked.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);

    assert!(
        !checked.status.success(),
        "read-only buffer must be rejected"
    );
    assert!(
        stderr.contains("<->") || stderr.contains("writable") || stderr.contains("mut"),
        "unexpected diagnostic: {stderr}"
    );
}
