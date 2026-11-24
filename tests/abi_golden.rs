use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-abi-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test dir should be creatable");
    dir
}

#[test]
fn abi_golden_c_output_is_stable() {
    let bless = std::env::var_os("LOI_BLESS").is_some_and(|v| v == "1");
    let fixture = repo_root().join("tests/abi/scalars.loi");
    let expected_path = repo_root().join("tests/abi/scalars.expected.c");
    let dir = test_dir("scalars");
    let out = dir.join("out.c");

    let output = Command::new(loi())
        .args([
            "gen",
            fixture.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("loi gen should run");
    assert!(
        output.status.success(),
        "loi gen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read_to_string(&out).expect("generated C should be readable");
    if bless {
        fs::write(&expected_path, &actual).expect("golden should be writable");
        let _ = fs::remove_dir_all(&dir);
        return;
    }

    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|_| {
        panic!(
            "missing ABI golden {}; run LOI_BLESS=1 cargo test --test abi_golden",
            expected_path.display()
        )
    });
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(
        actual, expected,
        "ABI golden changed; if intentional, regenerate with LOI_BLESS=1 cargo test --test abi_golden and commit"
    );
}
