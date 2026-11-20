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

    if MODE_A != 0 { => 11 }
    if MODE_B != 1 { => 12 }
    if SPARSE_B != 2 { => 13 }
    m <- Mode.MODE_A
    if m != Mode.MODE_A { => 14 }
    n <- sample_next_mode(m)
    if n != Mode.MODE_B { => 15 }
    mode_code <- sample_mode_code(n)
    if mode_code != 1 { => 16 }

    protect {
        bytes: [u8] <- [u8: 1, 2, 3]
        checksum <- sample_checksum(bytes, bytes.len)
        if checksum != 6 { => 17 }
        values: [i32] <- [i32: 4, 5]
        total <- sample_sum_i32(values, values.len)
        if total != 9 { => 18 }
    }
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
        bindings.contains("open opaque Opaque"),
        "incomplete record should become an open opaque handle:\n{bindings}"
    );
    assert!(
        bindings.contains("open shape Rect ["),
        "complete record should become an open shape:\n{bindings}"
    );
    assert!(
        bindings.contains("open SAMPLE_FLAG: u32 <- 4"),
        "integer macro should be bound as open:\n{bindings}"
    );
    assert!(
        bindings.contains("open SAMPLE_SHIFT: i32 <- 8"),
        "expression macro should be evaluated as open:\n{bindings}"
    );
    assert!(
        bindings.contains("open extern flow sample_copy_rect as loi_bind_sample_sample_copy_rect"),
        "struct pointer conversion should use the shim symbol:\n{bindings}"
    );
    assert!(
        bindings.contains("open mark Mode [\n    MODE_A\n    MODE_B\n]"),
        "a zero-based contiguous C enum should become an open mark:\n{bindings}"
    );
    assert!(
        bindings.contains("open extern flow sample_next_mode as loi_bind_sample_sample_next_mode"),
        "enum marks should use the shim for exact C enum types:\n{bindings}"
    );
    assert!(
        bindings.contains("open SPARSE_B: i32 <- 2") && !bindings.contains("mark SparseMode"),
        "a sparse C enum should keep open integer constants without a mark:\n{bindings}"
    );
    assert!(
        bindings.contains(
            "open extern flow sample_checksum as loi_bind_sample_sample_checksum\n    <- bytes: [u8]\n    <- len: u64\n    => i32\n"
        ),
        "a const scalar pointer should become a borrowed slice through the shim:\n{bindings}"
    );
    assert!(
        bindings.contains(
            "open extern flow sample_sum_i32 as loi_bind_sample_sample_sum_i32\n    <- values: [i32]\n    <- len: u64\n    => i32\n"
        ),
        "a const int pointer should become a borrowed i32 slice:\n{bindings}"
    );
    assert!(
        bindings.contains("open extern flow sample_open\n    <- name: cstr\n    => Opaque\n"),
        "const char * must stay a cstr, not become a slice:\n{bindings}"
    );
    assert!(
        !bindings.contains("__clang__"),
        "compiler predefined macros must not leak into bindings:\n{bindings}"
    );

    let shim = fs::read_to_string(dir.join("sample_shim.c")).expect("shim should be readable");
    assert!(
        shim.contains("#ifndef LOI_SLICE_DEFINED\n#define LOI_SLICE_DEFINED\ntypedef struct {\n    void* data;\n    size_t len;\n} LoiSlice;\n#endif"),
        "the shim must use the compiler's shared LoiSlice layout:\n{shim}"
    );
    assert!(
        shim.contains("sample_checksum((const uint8_t *)bytes.data, len)"),
        "a slice input must reach C through the shim's data pointer:\n{shim}"
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
    assert!(
        skipped.contains("sample_bad_pointer_return: returns unsupported pointer type"),
        "const scalar pointer returns should stay unsupported:\n{skipped}"
    );
    assert!(
        skipped
            .contains("sample_bad_pointer_to_pointer: parameter 'values' is a pointer-to-pointer"),
        "pointer-to-pointer parameters should stay unsupported:\n{skipped}"
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
        .arg("--cc-flag")
        .arg("-std=c11")
        .arg("--cc-flag")
        .arg("-pedantic-errors")
        .arg("--cc-flag")
        .arg("-Wall")
        .arg("--cc-flag")
        .arg("-Wextra")
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
