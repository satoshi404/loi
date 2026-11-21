//! Golden diagnostics for the error corpus.
//!
//! Every `tests/errors/NAME.loi` has a `NAME.expected` file holding the exact
//! `loi check --error-format short --color never` output for that file. Run
//! with `LOI_BLESS=1` to regenerate the expected files after an intentional
//! diagnostic change.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn corpus_files() -> Vec<PathBuf> {
    let dir = manifest_dir().join("tests/errors");
    let mut files = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("corpus entry should be readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "loi"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn run_short(path: &Path) -> std::process::Output {
    Command::new(loi())
        .arg("check")
        .arg("--error-format")
        .arg("short")
        .arg("--color")
        .arg("never")
        .arg(path)
        .current_dir(manifest_dir())
        .output()
        .expect("loi should run")
}

#[test]
fn error_corpus_matches_expected_short_output() {
    let bless = std::env::var_os("LOI_BLESS").is_some_and(|value| value == "1");
    let files = corpus_files();
    assert!(!files.is_empty(), "the error corpus should not be empty");

    let mut checked = 0;
    for input in files {
        let relative = input
            .strip_prefix(manifest_dir())
            .expect("corpus file should live under the manifest directory");
        let output = run_short(relative);
        assert!(
            !output.status.success(),
            "{} should fail the check",
            relative.display()
        );
        assert!(
            output.stdout.is_empty(),
            "{} should not write to stdout: {}",
            relative.display(),
            String::from_utf8_lossy(&output.stdout)
        );
        let actual = String::from_utf8(output.stderr).expect("diagnostics should be UTF-8");

        let expected_path = input.with_extension("expected");
        if bless {
            fs::write(&expected_path, &actual).unwrap_or_else(|error| {
                panic!("cannot write {}: {error}", expected_path.display())
            });
        } else {
            let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| {
                panic!(
                    "cannot read {}: {error}; run with LOI_BLESS=1 to create it",
                    expected_path.display()
                )
            });
            assert_eq!(
                actual,
                expected,
                "diagnostic mismatch for {} (run with LOI_BLESS=1 to update)",
                relative.display()
            );
        }
        checked += 1;
    }

    assert!(checked > 0);
}

#[test]
fn recursive_comptime_global_error_has_a_secondary_call_label() {
    let output = Command::new(loi())
        .arg("check")
        .arg("--error-format")
        .arg("json")
        .arg("--color")
        .arg("never")
        .arg("tests/errors/comptime_recursion.loi")
        .current_dir(manifest_dir())
        .output()
        .expect("loi should run");
    let actual = String::from_utf8(output.stderr).expect("diagnostics should be UTF-8");

    let call = actual
        .lines()
        .find(|line| {
            line.contains("\"message\":\"comptime call depth exceeded; is the flow recursive?\"")
        })
        .expect("the recursive call should be reported");
    assert!(call.contains("\"line\":4,\"column\":17"));

    let global = actual
        .lines()
        .find(|line| line.contains("initializer for global 'LIMIT'"))
        .expect("the global initializer should be reported");
    assert!(global.contains("\"line\":7,\"column\":1"));
    assert!(global.contains("\"label\":\"non-constant global initializer\""));
    assert!(global.contains("\"line\":4,\"column\":17,\"end_line\":4,\"end_column\":23,\"label\":\"recursive comptime call\""));
}

#[test]
fn every_example_checks_cleanly() {
    let dir = manifest_dir().join("examples");
    let mut files = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("example should be readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "loi"))
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            !name.ends_with("_error.loi") && name != "cycle_interface.loi"
        })
        .collect::<Vec<_>>();
    files.sort();
    assert!(!files.is_empty(), "examples directory should not be empty");

    for example in files {
        let relative = example
            .strip_prefix(manifest_dir())
            .expect("example should live under the manifest directory");
        let output = Command::new(loi())
            .arg("check")
            .arg("--error-format")
            .arg("json")
            .arg("--color")
            .arg("never")
            .arg(relative)
            .current_dir(manifest_dir())
            .output()
            .expect("loi should run");
        assert!(
            output.status.success(),
            "{} should check cleanly:\n{}{}",
            relative.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostics.contains("\"severity\":\"error\""),
            "{} should not emit errors:\n{diagnostics}",
            relative.display()
        );
    }
}
