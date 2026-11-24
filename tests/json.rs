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
    let dir = std::env::temp_dir().join(format!("loi-json-{}-{name}", std::process::id()));
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

fn lib_path() -> String {
    repo_root().join("lib").to_string_lossy().into_owned()
}

fn check(path: &Path) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(path)
        .output()
        .expect("loi should run")
}

fn build_and_run(name: &str, body: &str) -> (Output, PathBuf) {
    let dir = test_dir(name);
    let source = format!("import \"{}/json.loi\"\n{body}", lib_path());
    let program = write_program(&dir, source);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let executable = dir.join("program");
    let built = Command::new(loi())
        .arg("build")
        .arg(&program)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("loi build should run");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = Command::new(&executable)
        .output()
        .expect("built program should run");
    (run, dir)
}

fn assert_run(name: &str, body: &str, expected: &[u8]) {
    if !has_c_compiler() {
        eprintln!("skipping {name}: cc not found");
        return;
    }
    let (run, dir) = build_and_run(name, body);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(run.status.code(), Some(0), "{name} exited unexpectedly");
    assert_eq!(
        run.stdout,
        expected,
        "{name} stdout mismatch: {}",
        String::from_utf8_lossy(&run.stdout)
    );
}

#[test]
fn json_library_parses_and_accesses_an_lsp_like_object() {
    assert_run(
        "lsp-object",
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        ok <- json_parse("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"processId\":null,\"capabilities\":{}}}", -> doc)
        if not ok {
            => 1
        }
        root <- json_root(<- doc)
        if json_kind(<- doc, root) != JsonKind.Object {
            => 2
        }
        method <- json_object_get(<- doc, root, "method")
        if method < 0 {
            => 3
        }
        if json_kind(<- doc, method) != JsonKind.String {
            => 4
        }
        print("method=")
        print(json_string(<- doc, method))
        print("\n")
        id <- json_object_get(<- doc, root, "id")
        print("id=")
        print_int(json_number_i64(<- doc, id))
        print("\n")
        params <- json_object_get(<- doc, root, "params")
        process <- json_object_get(<- doc, params, "processId")
        if json_kind(<- doc, process) != JsonKind.Null {
            => 5
        }
        caps <- json_object_get(<- doc, params, "capabilities")
        if json_kind(<- doc, caps) != JsonKind.Object {
            => 6
        }
        print("caps=object\n")
    }
    => 0
}
"#,
        b"method=initialize\nid=1\ncaps=object\n",
    );
}

#[test]
fn json_library_handles_arrays_numbers_bools_and_escapes() {
    assert_run(
        "array-escapes",
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        ok <- json_parse("[1,-2,3.5,true,false,null,\"a\\n\\u00e9\\\"b\"]", -> doc)
        if not ok {
            => 1
        }
        arr <- json_root(<- doc)
        print_int(json_array_len(<- doc, arr) as i32)
        print("\n")
        third <- json_array_get(<- doc, arr, 2)
        print(json_number_text(<- doc, third))
        print("\n")
        text <- json_array_get(<- doc, arr, 6)
        print_int(json_string(<- doc, text).len as i32)
        print("\n")
        print(json_string(<- doc, text))
        print("\n")
        truth <- json_array_get(<- doc, arr, 3)
        if not json_bool(<- doc, truth) {
            => 2
        }
        falsehood <- json_array_get(<- doc, arr, 4)
        if json_bool(<- doc, falsehood) {
            => 3
        }
    }
    => 0
}
"#,
        "7\n3.5\n6\na\né\"b\n".as_bytes(),
    );
}

#[test]
fn json_library_stringifies_a_parsed_document() {
    assert_run(
        "round-trip",
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        ok <- json_parse("{\"b\":[1,2,{\"x\":null}],\"a\":\"x\\/y\"}", -> doc)
        if not ok {
            => 1
        }
        print(json_stringify(<- doc))
        print("\n")
    }
    => 0
}
"#,
        b"{\"b\":[1,2,{\"x\":null}],\"a\":\"x/y\"}\n",
    );
}

#[test]
fn json_library_reports_malformed_documents_without_panicking() {
    assert_run(
        "malformed",
        r#"
open flow main
    => i32
{
    protect {
        mut first: JsonDoc
        first_ok <- json_parse("{\"a\":}", -> first)
        if first_ok {
            => 1
        }
        print("bad1=false ")
    }
    protect {
        mut second: JsonDoc
        second_ok <- json_parse("[1,]", -> second)
        if second_ok {
            => 2
        }
        print("bad2=false ")
    }
    protect {
        mut third: JsonDoc
        third_ok <- json_parse("1 2", -> third)
        if third_ok {
            => 3
        }
        print("bad3=false\n")
    }
    => 0
}
"#,
        b"bad1=false bad2=false bad3=false\n",
    );
}

#[test]
fn json_accessors_handle_invalid_handles_and_missing_keys() {
    assert_run(
        "accessors",
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        ok <- json_parse("{\"n\":-2,\"arr\":[7],\"s\":\"x\"}", -> doc)
        if not ok {
            => 1
        }
        root <- json_root(<- doc)
        n <- json_object_get(<- doc, root, "n")
        if json_number_i64(<- doc, n) != -2 {
            => 2
        }
        arr <- json_object_get(<- doc, root, "arr")
        if json_array_get(<- doc, arr, 3) != -1 {
            => 3
        }
        if json_object_get(<- doc, root, "missing") != -1 {
            => 4
        }
        if json_kind(<- doc, 999) != JsonKind.Null {
            => 5
        }
        if json_bool(<- doc, n) {
            => 6
        }
        if json_string(<- doc, n).len != 0 {
            => 7
        }
        print("accessors=ok\n")
    }
    => 0
}
"#,
        b"accessors=ok\n",
    );
}

#[test]
fn json_parse_outside_protect_is_rejected() {
    let dir = test_dir("outside-protect");
    let program = write_program(
        &dir,
        format!(
            "import \"{}/json.loi\"\n\nopen flow main\n    => i32\n{{\n    mut doc: JsonDoc\n    ok <- json_parse(\"{{}}\", -> doc)\n    print_int(ok as i32)\n    => 0\n}}\n",
            lib_path()
        ),
    );
    let checked = check(&program);
    let stderr = String::from_utf8_lossy(&checked.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);

    assert!(!checked.status.success(), "parse outside protect must fail");
    assert!(
        stderr.contains("protect") || stderr.contains("allocation"),
        "unexpected diagnostic: {stderr}"
    );
}
