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
        "loi-vec-alias-{}-{name}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn main_source(prefix: &str, imports: &str) -> String {
    format!(
        "{imports}\n\
         open flow main\n\
         {{\n\
             protect {{\n\
                 mut v: Vec<u64>\n\
                 {p}::new<u64>(-> v)\n\
                 {p}::push<u64>(<-> v, 3)\n\
                 {p}::push<u64>(<-> v, 1)\n\
                 {p}::push<u64>(<-> v, 2)\n\
                 n <- {p}::size<u64>(v)\n\
                 print({line}(u64_to_bytes(n)))\n\
                 first <- {p}::get<u64>(v, 0)\n\
                 print({line}(u64_to_bytes(first)))\n\
                 {p}::set<u64>(<-> v, 1, 9)\n\
                 at <- {p}::find<u64>(v, 9)\n\
                 print({line}(i64_to_bytes(at)))\n\
                 {p}::sort<u64>(<-> v)\n\
                 sorted <- {p}::get<u64>(v, 0)\n\
                 print({line}(u64_to_bytes(sorted)))\n\
                 last <- {p}::pop<u64>(<-> v)\n\
                 print({line}(u64_to_bytes(last)))\n\
             }}\n\
         }}\n",
        p = prefix,
        line = if prefix == "vec" {
            "fmt::line"
        } else {
            "std::line"
        },
    )
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

const EXPECTED: &str = "3\n3\n1\n2\n9\n";

#[test]
fn short_names_resolve_through_a_logical_import() {
    let dir = test_dir("logical");
    let source = main_source(
        "vec",
        "import std::convert\nimport std::fmt\nimport std::vec",
    );
    fs::write(dir.join("main.loi"), source).expect("source should be writable");
    let output = Command::new(loi())
        .args([
            "check",
            "main.loi",
            "--lib-dir",
            repo_lib().to_str().unwrap(),
        ])
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
    fs::write(
        dir.join("main.loi"),
        main_source(
            "vec",
            "import std::convert\nimport std::fmt\nimport std::vec",
        ),
    )
    .expect("source should be writable");
    let output = Command::new(loi())
        .args([
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            repo_lib().to_str().unwrap(),
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
    assert_eq!(String::from_utf8_lossy(&run.stdout), EXPECTED);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn long_names_still_work_after_the_aliases_are_added() {
    let dir = test_dir("compat");
    let source = "import std::convert\n\
                  import std::fmt\n\
                  import std::vec\n\
                  open flow main\n\
                  {\n\
                      protect {\n\
                          mut v: Vec<u64>\n\
                          vec_new<u64>(-> v)\n\
                          vec_push<u64>(<-> v, 3)\n\
                          n <- vec_len<u64>(v)\n\
                          print(fmt::line(u64_to_bytes(n)))\n\
                      }\n\
                  }\n";
    fs::write(dir.join("main.loi"), source).expect("source should be writable");
    let output = Command::new(loi())
        .args([
            "check",
            "main.loi",
            "--lib-dir",
            repo_lib().to_str().unwrap(),
        ])
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
fn short_names_do_not_collide_in_the_broad_std_import() {
    let dir = test_dir("broad");
    let source = format!(
        "import \"{}\"\n{}",
        repo_lib().join("std.loi").display(),
        main_source("std", "")
    );
    assert_checks(&dir, &source);
    let _ = fs::remove_dir_all(&dir);
}
