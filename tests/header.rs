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
    let dir =
        PathBuf::from("/tmp/opencode").join(format!("loi-header-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn run(command: &mut Command) -> Output {
    command.output().expect("command should run")
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn has_tool(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn library_program(extra: &str) -> String {
    format!(
        r#"import "{}"
import "{}"
import "{}"

open mark Sig [ Red, Green ]
open choice R [ Ok: i32, Err: i32 ]

open flow all_parts
    <- v: Vec<i32>
    <- m: IntMap<i32>
    <- s: StrMap<i32>
    <-> r: Rng
    => i32
{{
    mut total: i32 <- map_int_get_or<i32>(m, 7, -1)
    total += map_str_get_or<i32>(s, "ab", -1)
    total += vec_len<i32>(v) as i32
    total += rng_next_u64(<-> r) as i32
    => total
}}
{extra}"#,
        repo_root().join("lib/vec.loi").display(),
        repo_root().join("lib/map.loi").display(),
        repo_root().join("lib/random.loi").display(),
    )
}

fn reference_body() -> &'static str {
    r#"
open flow main
    => i32
{
    vals <- [i32: 3, 5, 7]
    v <- Vec<i32>[ data: give(vals), len: 3, cap: 3 ]
    int_keys <- [u64: 7]
    int_values <- [i32: 11]
    m <- IntMap<i32>[ keys: give(int_keys), values: give(int_values), len: 1, cap: 1 ]
    str_keys: [u8] <- "ab"
    str_starts <- [u64: 0]
    str_lengths <- [u64: 2]
    str_values <- [i32: 4]
    s <- StrMap<i32>[
        keys: give(str_keys),
        starts: give(str_starts),
        lengths: give(str_lengths),
        values: give(str_values),
        len: 1,
        cap: 1,
        key_bytes: 2,
        key_cap: 2
    ]
    mut rng <- rng_new(1)
    draw <- rng_next_u64(<-> rng)
    len <- vec_len<i32>(v)
    int_value <- map_int_get_or<i32>(m, 7, -1)
    str_value <- map_str_get_or<i32>(s, "ab", -1)
    printf("%llu %d %d %llu\n", len, int_value, str_value, draw)
    mut rng2 <- rng_new(1)
    total <- all_parts(v, m, s, <-> rng2)
    printf("%d\n", total)
    => 0
}
"#
}

fn generate(dir: &Path, name: &str, source: &str) -> (PathBuf, PathBuf) {
    let program = dir.join(format!("{name}.loi"));
    let c = dir.join(format!("{name}.c"));
    let header = dir.join(format!("{name}.h"));
    fs::write(&program, source).expect("source should be writable");
    let generated = run(Command::new(loi())
        .arg("gen")
        .arg(&program)
        .arg("-o")
        .arg(&c)
        .arg("--emit-h")
        .arg(&header));
    assert_success("loi gen --emit-h", &generated);
    (c, header)
}

#[test]
fn header_compiles_alone_in_strict_c11_and_cpp17() {
    if !has_tool("cc") {
        eprintln!("skipping header_compiles_alone_in_strict_c11_and_cpp17: no C compiler");
        return;
    }
    let dir = test_dir("standalone");
    let (c, header) = generate(&dir, "prog", &library_program(""));

    let c_probe = dir.join("header_probe.c");
    fs::write(
        &c_probe,
        "#include \"prog.h\"\nint probe(void) { return 0; }\n",
    )
    .expect("C probe should be writable");
    let c_output = run(Command::new("cc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(&dir)
        .arg(&c_probe));
    assert_success("strict C11 header compile", &c_output);

    if has_tool("c++") {
        let cpp_probe = dir.join("header_probe.cpp");
        fs::write(
            &cpp_probe,
            "#include \"prog.h\"\nint probe_cpp() { return 0; }\n",
        )
        .expect("C++ probe should be writable");
        let cpp_output = run(Command::new("c++")
            .args([
                "-std=c++17",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-fsyntax-only",
                "-I",
            ])
            .arg(&dir)
            .arg(&cpp_probe));
        assert_success("C++17 header compile", &cpp_output);
    } else {
        eprintln!("skipping C++17 header compile: no C++ compiler found");
    }

    let text = fs::read_to_string(&header).expect("header should be readable");
    assert!(
        text.contains("#ifndef PROG_H"),
        "header guard should come from the output file name:\n{text}"
    );
    assert!(
        text.contains("#ifdef __cplusplus"),
        "missing C++ guards:\n{text}"
    );

    let c_text = fs::read_to_string(&c).expect("generated C should be readable");
    for definition in [
        "int32_t loi_fn_all_parts(",
        "uint64_t loi_fn_vec_len__i32(",
        "int32_t loi_fn_map_int_get_or__i32(",
    ] {
        assert!(
            c_text.contains(definition),
            "header prototype {definition:?} has no external definition:\n{c_text}"
        );
        assert!(
            !c_text.contains(&format!("static inline {definition}")),
            "{definition:?} should not have internal linkage:\n{c_text}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn linked_c_caller_matches_the_loi_computation() {
    if !has_tool("cc") {
        eprintln!("skipping linked_c_caller_matches_the_loi_computation: no C compiler");
        return;
    }
    let dir = test_dir("link");
    let lib_source = library_program("");
    let (c, header) = generate(&dir, "prog", &lib_source);

    let reference_source = dir.join("reference.loi");
    fs::write(&reference_source, library_program(reference_body()))
        .expect("reference source should be writable");
    let reference_exe = dir.join("reference");
    let reference_build = run(Command::new(loi())
        .arg("build")
        .arg(&reference_source)
        .arg("-o")
        .arg(&reference_exe));
    assert_success("reference build", &reference_build);
    let reference_run = run(&mut Command::new(&reference_exe));
    assert_success("reference run", &reference_run);

    let harness = dir.join("caller.c");
    fs::write(
        &harness,
        r#"#include <stdio.h>
#include "prog.h"

static int32_t vec_values[] = { 3, 5, 7 };
static uint64_t int_keys[] = { 7 };
static int32_t int_values[] = { 11 };
static uint8_t str_keys[] = { 'a', 'b' };
static uint64_t str_starts[] = { 0 };
static uint64_t str_lengths[] = { 2 };
static int32_t str_values[] = { 4 };

int main(void) {
    LoiSlice vec_slice = { (void*)vec_values, 3 };
    loi_t_Vec__i32 v = { vec_slice, 3, 3 };
    LoiSlice int_key_slice = { (void*)int_keys, 1 };
    LoiSlice int_value_slice = { (void*)int_values, 1 };
    loi_t_IntMap__i32 m = { int_key_slice, int_value_slice, 1, 1 };
    LoiSlice str_key_slice = { (void*)str_keys, 2 };
    LoiSlice str_start_slice = { (void*)str_starts, 1 };
    LoiSlice str_length_slice = { (void*)str_lengths, 1 };
    LoiSlice str_value_slice = { (void*)str_values, 1 };
    loi_t_StrMap__i32 s = {
        str_key_slice,
        str_start_slice,
        str_length_slice,
        str_value_slice,
        1,
        1,
        2,
        2
    };
    LoiSlice key = { (void*)"ab", 2 };
    loi_t_Rng rng = { 1 };
    loi_t_Rng rng2 = { 1 };

    uint64_t len = loi_fn_vec_len__i32(v);
    int32_t int_value = loi_fn_map_int_get_or__i32(m, 7, -1);
    int32_t str_value = loi_fn_map_str_get_or__i32(s, key, -1);
    uint64_t draw = loi_fn_rng_next_u64(&rng);
    int32_t total = loi_fn_all_parts(v, m, s, &rng2);
    printf("%llu %d %d %llu\n", (unsigned long long)len, int_value, str_value, (unsigned long long)draw);
    printf("%d\n", total);
    return 0;
}
"#,
    )
    .expect("C harness should be writable");

    assert!(
        fs::read_to_string(&header)
            .expect("header should be readable")
            .contains("loi_fn_map_str_get_or__i32"),
        "header should declare the StrMap lookup"
    );

    let caller_exe = dir.join("caller");
    let built = run(Command::new("cc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
        ])
        .arg(&dir)
        .arg(&harness)
        .arg(&c)
        .arg("-o")
        .arg(&caller_exe));
    assert_success("C harness build", &built);
    let caller_run = run(&mut Command::new(&caller_exe));
    assert_success("C harness run", &caller_run);
    assert_eq!(caller_run.stdout, reference_run.stdout);
    assert_eq!(
        String::from_utf8_lossy(&caller_run.stdout),
        "3 11 4 5180492295206395165\n-1989354193\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn headers_can_be_included_twice_and_together() {
    if !has_tool("cc") {
        eprintln!("skipping headers_can_be_included_twice_and_together: no C compiler");
        return;
    }
    let dir = test_dir("multiple");
    generate(&dir, "first", &library_program(""));
    let second_source = r#"
open mark OtherMark [ One, Two ]
open choice OtherChoice [ Value: i32 ]
open shape OtherShape [ value: i32 ]

open flow other_value
    <- value: OtherShape
    => i32
{
    => value.value
}
"#;
    generate(&dir, "second", second_source);

    let probe = dir.join("multiple.c");
    fs::write(
        &probe,
        r#"#include "first.h"
#include "first.h"
#include "second.h"
#include "second.h"

int main(void) {
    loi_t_OtherShape value = { 7 };
    return loi_fn_other_value(value) == 7 ? 0 : 1;
}
"#,
    )
    .expect("multi-header probe should be writable");
    let built = run(Command::new("cc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
        ])
        .arg(&dir)
        .arg(&probe)
        .arg(dir.join("first.c"))
        .arg(dir.join("second.c"))
        .arg("-o")
        .arg(dir.join("multiple")));
    assert_success("two-header build", &built);
    let run_output = run(&mut Command::new(dir.join("multiple")));
    assert_success("two-header run", &run_output);

    for (name, guard) in [("first.h", "FIRST_H"), ("second.h", "SECOND_H")] {
        let text = fs::read_to_string(dir.join(name)).expect("header should be readable");
        assert!(
            text.contains(&format!("#ifndef {guard}")),
            "{name} should use a filename-derived guard:\n{text}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn headers_never_contain_sealed_flows() {
    let dir = test_dir("sealed");
    let (_, header) = generate(&dir, "prog", &library_program(""));
    let text = fs::read_to_string(header).expect("header should be readable");
    for sealed in [
        "loi_fn_vec_decimal_len",
        "loi_fn_vec_write_bytes",
        "loi_fn_vec_fail_range",
        "loi_fn_map_int_find__i32",
        "loi_fn_map_int_grow__i32",
        "loi_fn_map_str_find__i32",
        "loi_fn_map_str_grow__i32",
        "loi_fn_rng_swap__i32",
    ] {
        assert!(
            !text.contains(sealed),
            "header leaked sealed flow {sealed}:\n{text}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}
