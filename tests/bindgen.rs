use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn test_dir(name: &str) -> PathBuf {
    let dir =
        PathBuf::from("/tmp/opencode").join(format!("loi-bindgen-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn has_tool(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn bindgen_tools_available() -> bool {
    has_tool("python3") && has_tool("clang") && has_tool("cc")
}

fn run_bindgen(dir: &Path) {
    let script = repo_root().join("tools/bindgen/bindgen.py");
    let output = Command::new("python3")
        .arg(&script)
        .arg("--header")
        .arg(dir.join("sample.h"))
        .arg("--output")
        .arg(dir.join("sample.loi"))
        .arg("--shim")
        .arg(dir.join("sample_shim.c"))
        .arg("--skipped")
        .arg(dir.join("skipped.txt"))
        .arg("--module")
        .arg("sample")
        .current_dir(dir)
        .output()
        .expect("bindgen should run");

    assert!(
        output.status.success(),
        "bindgen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const PROBE: &str = r#"
import "sample.loi"

open flow main
    => i32
{
    a <- Rect[ w: 3, h: 4 ]
    mut b <- Rect[ w: 0, h: 0 ]
    sample_copy_rect(a, <-> b)
    if b.w != 3 { => 1 }
    if b.h != 4 { => 2 }

    handle <- sample_open("probe")
    mut w <- 0
    mut h <- 0
    code <- sample_get_size(handle, -> w, -> h)
    sample_close(handle)
    if code != 0 { => 3 }
    if w != 640 { => 4 }
    if h != 480 { => 5 }

    thing <- sample_make(7, 2.5)
    if thing.x != 7 { => 6 }
    if thing.y != 2.5 { => 7 }

    rect <- sample_rect(3, 4)
    if rect.w != 3 { => 8 }
    if rect.h != 4 { => 9 }

    init <- sample_init(SAMPLE_FLAG + (SAMPLE_SHIFT as u32))
    if init != 8 { => 10 }
    => 0
}
"#;

#[test]
fn bindgen_generates_buildable_bindings() {
    if !bindgen_tools_available() {
        eprintln!("skipping bindgen test: python3, clang or cc is unavailable");
        return;
    }

    let dir = test_dir("sample");
    let fixtures = repo_root().join("tools/bindgen/tests");
    fs::copy(fixtures.join("sample.h"), dir.join("sample.h"))
        .expect("sample header should be copied");
    fs::copy(fixtures.join("sample.c"), dir.join("sample.c"))
        .expect("sample source should be copied");
    fs::write(dir.join("main.loi"), PROBE).expect("probe should be written");

    run_bindgen(&dir);

    let bindings = fs::read_to_string(dir.join("sample.loi")).expect("bindings should be readable");
    assert!(
        bindings.contains("opaque Opaque"),
        "incomplete record should become an opaque handle:\n{bindings}"
    );
    assert!(
        bindings.contains("shape Rect ["),
        "complete record should become a shape:\n{bindings}"
    );
    assert!(
        bindings.contains("SAMPLE_FLAG: u32 <- 4"),
        "integer macro should be bound:\n{bindings}"
    );
    assert!(
        bindings.contains("SAMPLE_SHIFT: i32 <- 8"),
        "expression macro should be evaluated:\n{bindings}"
    );
    assert!(
        bindings.contains("extern flow sample_copy_rect as loi_bind_sample_sample_copy_rect"),
        "struct pointer conversion should use the shim symbol:\n{bindings}"
    );
    assert!(
        !bindings.contains("__clang__"),
        "compiler predefined macros must not leak into bindings:\n{bindings}"
    );

    let skipped = fs::read_to_string(dir.join("skipped.txt")).expect("skip report should exist");
    assert!(
        skipped.contains("sample_fill: parameter 'buf' is a mutable char buffer"),
        "mutable char buffers should be reported:\n{skipped}"
    );
    assert!(
        skipped.contains("sample_varargs: variadic function"),
        "variadic functions should be reported:\n{skipped}"
    );

    let program = dir.join("main.loi");
    let binary = dir.join("probe");
    let build = Command::new(loi())
        .arg("build")
        .arg(&program)
        .arg("-o")
        .arg(&binary)
        .arg("--c-source")
        .arg(dir.join("sample_shim.c"))
        .arg("--c-source")
        .arg(dir.join("sample.c"))
        .arg("--cc-flag")
        .arg("-I.")
        .current_dir(&dir)
        .output()
        .expect("loi build should run");
    assert!(
        build.status.success(),
        "loi build failed: {}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&binary)
        .current_dir(&dir)
        .output()
        .expect("generated program should run");
    assert_eq!(
        run.status.code(),
        Some(0),
        "generated program failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = fs::remove_dir_all(&dir);
}
