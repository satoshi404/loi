use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        "loi-fmt-alias-{}-{name}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn check(dir: &Path, source: &str) -> std::process::Output {
    fs::write(dir.join("main.loi"), source).expect("source should be writable");
    Command::new(loi())
        .arg("check")
        .arg("main.loi")
        .current_dir(dir)
        .output()
        .expect("loi should run")
}

fn assert_checks(dir: &Path, source: &str) {
    let output = check(dir, source);
    assert!(
        output.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const MAIN_SHORT: &str = "import std::fmt\n\
                          open flow main\n\
                          {\n\
                              protect {\n\
                                  print(fmt::line(fmt::label(\"total\", \"16\")))\n\
                                  print(fmt::parens(\"ok\"))\n\
                              }\n\
                          }\n";

const MAIN_LONG: &str = "import std::fmt\n\
                         open flow main\n\
                         {\n\
                             protect {\n\
                                 print(fmt::fmt_line(fmt::fmt_label(\"total\", \"16\")))\n\
                                 print(fmt::fmt_parens(\"ok\"))\n\
                             }\n\
                         }\n";

#[test]
fn short_names_resolve_through_a_logical_import() {
    let dir = test_dir("logical");
    let lib = repo_lib();
    let source = "import std::fmt\n\
                  open flow main\n\
                  {\n\
                      protect {\n\
                          print(fmt::line(fmt::label(\"a\", \"b\")))\n\
                          print(fmt::parens(\"c\"))\n\
                      }\n\
                  }\n";
    fs::write(dir.join("main.loi"), source).expect("source should be writable");
    let output = Command::new(loi())
        .args(["check", "main.loi", "--lib-dir", lib.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("loi should run");
    assert!(
        output.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn short_names_build_and_run() {
    let dir = test_dir("build");
    let lib = repo_lib();
    fs::write(dir.join("main.loi"), MAIN_SHORT).expect("source should be writable");
    let output = Command::new(loi())
        .args([
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            lib.to_str().unwrap(),
        ])
        .current_dir(&dir)
        .output()
        .expect("loi should run");
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "total: 16\n(ok)");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn long_names_still_work_after_the_aliases_are_added() {
    let dir = test_dir("compat");
    let lib = repo_lib();
    fs::write(dir.join("main.loi"), MAIN_LONG).expect("source should be writable");
    let output = Command::new(loi())
        .args(["check", "main.loi", "--lib-dir", lib.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("loi should run");
    assert!(
        output.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn short_names_do_not_break_the_broad_std_import() {
    let dir = test_dir("broad");
    let std = repo_lib().join("std.loi");
    let source = format!(
        "import \"{}\"\n\
         open flow main\n\
         {{\n\
             protect {{\n\
                 print(std::line(std::label(\"a\", \"b\")))\n\
             }}\n\
         }}\n",
        std.display()
    );
    assert_checks(&dir, &source);
    let _ = fs::remove_dir_all(&dir);
}
