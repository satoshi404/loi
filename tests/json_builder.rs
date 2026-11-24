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
    let dir = std::env::temp_dir().join(format!("loi-json-builder-{}-{name}", std::process::id()));
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

fn source(imports: &[&str], body: &str) -> String {
    let mut text = String::new();
    for name in imports {
        text.push_str(&format!(
            "import \"{}\"\n",
            repo_root().join("lib").join(name).display()
        ));
    }
    text.push_str(body);
    text
}

fn assert_run(name: &str, imports: &[&str], body: &str, expected: &[u8]) {
    if !has_c_compiler() {
        eprintln!("skipping {name}: cc not found");
        return;
    }
    let dir = test_dir(name);
    let program = dir.join("main.loi");
    fs::write(&program, source(imports, body)).expect("program should be writable");
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
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(run.status.code(), Some(0), "{name} exited unexpectedly");
    assert_eq!(
        run.stdout,
        expected,
        "{name} stdout mismatch: {}",
        String::from_utf8_lossy(&run.stdout)
    );
}

fn assert_rejected(name: &str, imports: &[&str], body: &str, needle: &str) {
    let dir = test_dir(name);
    let program = dir.join("main.loi");
    fs::write(&program, source(imports, body)).expect("program should be writable");
    let checked = check(&program);
    let stderr = String::from_utf8_lossy(&checked.stderr).to_string();
    let _ = fs::remove_dir_all(&dir);
    assert!(!checked.status.success(), "{name} must be rejected");
    assert!(
        stderr.contains(needle),
        "{name}: expected {needle:?} in: {stderr}"
    );
}

#[test]
fn json_builder_builds_nested_documents() {
    assert_run(
        "nested",
        &["json.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        json_new(-> doc)
        root <- json_new_object(<-> doc)
        items <- json_new_array(<-> doc)
        minus_seven <- json_new_i64(<-> doc, -7)
        big <- json_new_u64(<-> doc, 18446744073709551615)
        text <- json_new_string(<-> doc, "hi\n\"")
        json_array_push(<-> doc, items, minus_seven)
        json_array_push(<-> doc, items, big)
        json_array_push(<-> doc, items, text)
        name <- json_new_string(<-> doc, "loi")
        count <- json_new_i64(<-> doc, 3)
        yes <- json_new_bool(<-> doc, true)
        nothing <- json_new_null(<-> doc)
        json_object_set(<-> doc, root, "name", name)
        json_object_set(<-> doc, root, "count", count)
        json_object_set(<-> doc, root, "items", items)
        json_object_set(<-> doc, root, "ok", yes)
        json_object_set(<-> doc, root, "nothing", nothing)
        json_set_root(<-> doc, root)
        print(json_stringify(<- doc))
        print("\n")
    }
    => 0
}
"#,
        br#"{"name":"loi","count":3,"items":[-7,18446744073709551615,"hi\n\""],"ok":true,"nothing":null}
"#,
    );
}

#[test]
fn json_builder_escapes_strings_and_keys() {
    assert_run(
        "escaping",
        &["json.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        json_new(-> doc)
        root <- json_new_object(<-> doc)
        mut key: [u8]
        alloc(-> key, u8, 3)
        key[0] = 97
        key[1] = 10
        key[2] = 34
        mut value: [u8]
        alloc(-> value, u8, 4)
        value[0] = 8
        value[1] = 9
        value[2] = 1
        value[3] = 92
        escaped <- json_new_string(<-> doc, value)
        json_object_set(<-> doc, root, key, escaped)
        json_set_root(<-> doc, root)
        print(json_stringify(<- doc))
        print("\n")
    }
    => 0
}
"#,
        br#"{"a\n\"":"\b\t\u0001\\"}
"#,
    );
}

#[test]
fn integer_to_bytes_covers_zero_negative_and_limits() {
    assert_run(
        "integer-bytes",
        &["std.loi"],
        r#"
open flow main
    => i32
{
    protect {
        print(u64_to_bytes(0))
        print("|")
        print(u64_to_bytes(18446744073709551615))
        print("|")
        print(i64_to_bytes(0))
        print("|")
        print(i64_to_bytes(-1))
        print("|")
        // A large negative literal would be narrowed to i32; derive i64::MIN
        // from its unsigned magnitude instead.
        mut magnitude: u64 <- 9223372036854775808
        mut lowest: i64 <- magnitude as i64
        print(i64_to_bytes(lowest))
        print("|")
        print(i64_to_bytes(9223372036854775807))
        print("\n")
    }
    => 0
}
"#,
        b"0|18446744073709551615|0|-1|-9223372036854775808|9223372036854775807\n",
    );
}

#[test]
fn built_numbers_work_with_the_parsed_document_accessors() {
    assert_run(
        "number-accessors",
        &["json.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut doc: JsonDoc
        json_new(-> doc)
        root <- json_new_array(<-> doc)
        negative <- json_new_i64(<-> doc, -42)
        scientific <- json_new_number(<-> doc, "1.25e2")
        json_array_push(<-> doc, root, negative)
        json_array_push(<-> doc, root, scientific)
        json_set_root(<-> doc, root)
        first <- json_array_get(<- doc, root, 0)
        second <- json_array_get(<- doc, root, 1)
        print_int(json_number_i64(<- doc, first))
        print("|")
        print(json_number_text(<- doc, second))
        print("|")
        print(json_stringify(<- doc))
        print("\n")
    }
    => 0
}
"#,
        b"-42|1.25e2|[-42,1.25e2]\n",
    );
}

#[test]
fn builder_and_integer_conversion_require_protect() {
    assert_rejected(
        "builder-outside-protect",
        &["json.loi"],
        r#"
open flow main
    => i32
{
    mut doc: JsonDoc
    json_new(-> doc)
    value <- json_new_i64(<-> doc, 7)
    print_int(value)
    => 0
}
"#,
        "protect",
    );
    assert_rejected(
        "conversion-outside-protect",
        &["std.loi"],
        r#"
open flow main
    => i32
{
    mut digits: [u8] <- i64_to_bytes(7)
    print(digits)
    => 0
}
"#,
        "protect",
    );
}
