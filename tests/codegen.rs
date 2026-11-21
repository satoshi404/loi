use std::fs;
use std::path::PathBuf;
use std::process::Command;

use loi::{check, generate_c, generate_header, parse, tokenize};

fn generated(source: &str) -> String {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    let diagnostics = check(&program);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
    generate_c(&program, None).expect("source should generate C")
}

fn generated_header(source: &str) -> String {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    let diagnostics = check(&program);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
    generate_header(&program, None).expect("source should generate header")
}

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Compile the generated translation unit with strict C11 warnings promoted to
/// errors. This is intentionally stronger than the link-and-run helper: it
/// verifies the generated C itself, independently of any test harness.
fn assert_strict_c11(name: &str, source: &str) {
    if !has_c_compiler() {
        eprintln!("skipping {name}: no C compiler found");
        return;
    }

    let dir = PathBuf::from("/tmp/opencode")
        .join(format!("loi-codegen-{}-{name}-strict", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    let path = dir.join("generated.c");
    fs::write(&path, source).expect("generated C should be writable");

    let output = Command::new("cc")
        .arg("-std=c11")
        .arg("-pedantic-errors")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-fsyntax-only")
        .arg(&path)
        .output()
        .expect("cc should run");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "strict C11 rejected {name}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Compile (but do not link) the generated C with ASan and UBSan enabled.
/// ASan cannot run in every container, so unsupported sanitizer flags are
/// recorded as a skip; real generated-code errors still fail the test.
fn assert_sanitizers_compile(name: &str, source: &str) {
    if !has_c_compiler() {
        eprintln!("skipping {name}: no C compiler found");
        return;
    }

    let dir = PathBuf::from("/tmp/opencode")
        .join(format!("loi-codegen-{}-{name}-san", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    let path = dir.join("generated.c");
    let object = dir.join("generated.o");
    fs::write(&path, source).expect("generated C should be writable");

    let output = Command::new("cc")
        .arg("-std=c11")
        .arg("-pedantic-errors")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-fsanitize=address,undefined")
        .arg("-fno-omit-frame-pointer")
        .arg("-c")
        .arg(&path)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("cc should run");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = fs::remove_dir_all(&dir);
    if !output.status.success() {
        let unsupported = stderr.contains("unrecognized command line option")
            || stderr.contains("unsupported option")
            || stderr.contains("unknown argument")
            || stderr.contains("unable to find");
        if unsupported {
            eprintln!("skipping {name}: sanitizer flags unavailable: {stderr}");
            return;
        }
        panic!("sanitizer compile rejected {name}:\n{stderr}");
    }
}

/// Compile the generated C together with a C harness that calls into it.
///
/// The harness `#include`s the generated translation unit so it can reach the
/// otherwise `static` functions. The test is skipped when no C compiler is
/// available.
fn compile_and_run_output(name: &str, source: &str, harness: &str) -> Option<std::process::Output> {
    if !has_c_compiler() {
        eprintln!("skipping {name}: no C compiler found");
        return None;
    }

    let dir =
        PathBuf::from("/tmp/opencode").join(format!("loi-codegen-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");

    let generated_path = dir.join("generated.c");
    let harness_path = dir.join("harness.c");
    let executable = dir.join("program");

    fs::write(&generated_path, source).expect("generated C should be writable");
    fs::write(
        &harness_path,
        format!("#include \"{}\"\n\n{harness}\n", generated_path.display()),
    )
    .expect("harness should be writable");

    let compile = Command::new("cc")
        .arg("-std=c11")
        .arg("-pedantic-errors")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-o")
        .arg(&executable)
        .arg(&harness_path)
        .arg("-lm")
        .output()
        .expect("cc should run");
    assert!(
        compile.status.success(),
        "cc failed for {name}:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(&executable)
        .output()
        .expect("compiled program should run");
    let _ = fs::remove_dir_all(&dir);
    Some(run)
}

fn compile_and_run(name: &str, source: &str, harness: &str) {
    let Some(run) = compile_and_run_output(name, source, harness) else {
        return;
    };
    assert_eq!(
        run.status.code(),
        Some(0),
        "program for {name} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

fn assert_runtime_trap(name: &str, source: &str, harness: &str, message: &str) {
    let Some(run) = compile_and_run_output(name, source, harness) else {
        return;
    };
    assert_eq!(
        run.status.code(),
        Some(1),
        "program for {name} should trap with exit 1:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains(message) && stderr.contains(" at "),
        "expected {message:?} with a source position, got: {stderr}"
    );
}

#[test]
fn reference_program_generates_c() {
    let c = generated(include_str!("../examples/particles.loi"));

    assert!(c.contains("typedef struct loi_t_Vec2 loi_t_Vec2;"));
    assert!(c.contains("struct loi_t_World {"));
    // `world_new` is `visible`, so it is not `static inline`.
    assert!(c.contains("void loi_fn_world_new("));
    assert!(!c.contains("static inline void loi_fn_world_new("));
    // Hidden functions remain `static inline`.
    assert!(c.contains("static inline double loi_fn_clamp("));
    assert!(c.contains("static inline void loi_fn_bounce("));
    assert!(c.contains("int main(void);"));
    assert!(c.contains("loi_index("));
    assert!(c.contains("loi_alloc("));
    assert!(c.contains("loi_alloc_slice("));
    // `world_new` leaves `World.tick` unset, so its default must be applied.
    assert!(c.contains(".loi_f_tick = 0"));
}

#[test]
fn tictactoe_example_compiles_and_runs() {
    let source = include_str!("../examples/tictactoe.loi");
    let c = generated(source);
    // The generated translation unit already defines `main`.
    compile_and_run("tictactoe", &c, "");
}

#[test]
fn marks_example_compiles_and_runs() {
    let c = generated(include_str!("../examples/marks.loi"));
    assert_strict_c11("marks-example", &c);
    assert_sanitizers_compile("marks-example", &c);
    compile_and_run("marks-example", &c, "");
}

#[test]
fn choices_example_compiles_and_runs() {
    let c = generated(include_str!("../examples/choices.loi"));
    assert_strict_c11("choices-example", &c);
    assert_sanitizers_compile("choices-example", &c);
    compile_and_run("choices-example", &c, "");
}

#[test]
fn header_contains_visible_items_only() {
    let h = generated_header(
        "open shape Vec2 [\n    x: f64\n    y: f64\n]\n\nopen shape World [\n    gravity: Vec2\n]\n\nopen flow world_new\n    <- size: Vec2\n    -> w: World\n{\n    w.gravity = Vec2[ x: 0, y: 0 ]\n}\n\nflow secret\n    => i32\n{\n    => 1\n}\n\nflow main\n    => i32\n{\n    => 0\n}\n",
    );

    // Header guard.
    assert!(h.contains("#ifndef LOI_MAIN_H"));
    assert!(h.contains("#define LOI_MAIN_H"));
    assert!(h.contains("#endif"));

    // Visible interfaces are declared.
    assert!(h.contains("typedef struct loi_t_Vec2 loi_t_Vec2;"));
    assert!(h.contains("struct loi_t_Vec2 {"));
    assert!(h.contains("typedef struct loi_t_World loi_t_World;"));
    assert!(h.contains("struct loi_t_World {"));

    // Visible functions are declared (without `static inline`).
    assert!(h.contains("void loi_fn_world_new("));
    assert!(!h.contains("static inline void loi_fn_world_new("));
    assert!(h.contains("int main(void);"));

    // Hidden functions are NOT in the header.
    assert!(!h.contains("loi_fn_secret"));

    // Intrinsics are declared.
    assert!(h.contains("void loi_fn_print("));
    assert!(h.contains("double loi_fn_sqrt("));
}

#[test]
fn header_excludes_hidden_and_unspecified_items() {
    let h = generated_header(
        "shape Secret [\n    value: i32\n]\n\nflow secret_fn\n    <- x: i32\n    => i32\n{\n    => x\n}\n\nopen flow public_fn\n    <- x: i32\n    => i32\n{\n    => x\n}\n\nflow unspecified_fn\n    <- x: i32\n    => i32\n{\n    => x\n}\n\nflow main\n    => i32\n{\n    => 0\n}\n",
    );

    // Unspecified interface is not in the header.
    assert!(!h.contains("loi_t_Secret"));
    // Hidden function is not in the header.
    assert!(!h.contains("secret_fn"));
    // Unspecified function is not in the header.
    assert!(!h.contains("unspecified_fn"));
    // Visible function is in the header.
    assert!(h.contains("public_fn"));
    // main is in the header.
    assert!(h.contains("int main(void);"));
}

#[test]
fn explicit_return_in_main_returns_zero() {
    let c = generated("flow main {\n    =>\n}\n");
    assert!(c.contains("return 0;"), "{c}");
}

#[test]
fn rejects_unsupported_main_signatures() {
    let with_param = tokenize("flow main\n    <- x: i32\n{\n}\n", "<test>").expect("lex");
    let program = parse(with_param).expect("parse");
    let error = generate_c(&program, None).expect_err("parameters should be rejected");
    assert!(error.message.contains("no parameters"));

    let with_composite = tokenize(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow main\n    => Vec2\n{\n    => Vec2[ x: 0, y: 0 ]\n}\n",
        "<test>",
    )
    .expect("lex");
    let program = parse(with_composite).expect("parse");
    let error = generate_c(&program, None).expect_err("composite return should be rejected");
    assert!(error.message.contains("return no value or an integer"));
}

#[test]
fn alloc_sequences_target_count_and_store() {
    let c = generated(
        "shape Item [\n    value: i32\n]\n\nflow fill\n    -> items: [Item]\n    <- n: u32\n{\n    alloc(-> items, Item, n + 1)\n}\n",
    );

    let target = c
        .find("LoiSlice* loi_tmp_alloc_target_")
        .expect("alloc target address should be captured");
    let count = c
        .find("size_t loi_tmp_alloc_count_")
        .expect("alloc count should be captured");
    let store = c
        .find("*loi_tmp_alloc_target_")
        .expect("alloc result should be stored through the captured target");
    assert!(
        target < count && count < store,
        "alloc must evaluate target, then count, then store:\n{c}"
    );
}

#[test]
fn u64_ranges_use_an_unsigned_induction_variable() {
    let c = generated(
        "flow sum_to\n    <- n: u64\n    => u64\n{\n    mut total: u64 <- 0\n    for i in 0..n {\n        total += i\n    }\n    => total\n}\n",
    );

    assert!(c.contains("uint64_t loi_tmp_end_"), "{c}");
    assert!(c.contains("for (uint64_t loi_v_i"), "{c}");
}

#[test]
fn compiled_range_loop_edges() {
    let source = "flow sum_signed\n    <- start: i64\n    <- end: i64\n    => i64\n{\n    mut total: i64 <- 0\n    for i in start..end {\n        total += i\n    }\n    => total\n}\n\nflow sum_unsigned\n    <- start: u64\n    <- end: u64\n    => u64\n{\n    mut total: u64 <- 0\n    for i in start..end {\n        total += i\n    }\n    => total\n}\n\nflow count_unsigned\n    <- start: u64\n    <- end: u64\n    => u64\n{\n    mut count: u64 <- 0\n    for i in start..end {\n        count += 1\n    }\n    => count\n}\n\nflow sum_literals\n    => i32\n{\n    mut total: i32 <- 0\n    for i in -3..3 {\n        total += i\n    }\n    => total\n}\n\nflow count_literal_empty\n    => i32\n{\n    mut count: i32 <- 0\n    for i in 5..5 {\n        count += 1\n    }\n    => count\n}\n";

    let c = generated(source);
    assert!(c.contains("for (int64_t loi_v_i"), "{c}");
    assert!(c.contains("for (uint64_t loi_v_i"), "{c}");

    let harness = r#"
int main(void) {
    if (loi_fn_sum_signed(-3, 3) != -3) return 40;
    if (loi_fn_sum_signed(3, -3) != 0) return 41;
    if (loi_fn_sum_signed(0, 0) != 0) return 42;

    if (loi_fn_sum_unsigned(0, 5) != 10) return 43;
    if (loi_fn_sum_unsigned(5, 0) != 0) return 44;

    // The last iteration is UINT64_MAX-1, so incrementing must not wrap.
    if (loi_fn_count_unsigned(UINT64_MAX - 2, UINT64_MAX) != 2) return 45;
    if (loi_fn_count_unsigned(UINT64_MAX - 1, UINT64_MAX) != 1) return 46;

    if (loi_fn_sum_literals() != -3) return 47;
    if (loi_fn_count_literal_empty() != 0) return 48;
    return 0;
}
"#;

    compile_and_run("range-edges", &c, harness);
}

#[test]
fn interfaces_are_ordered_by_value_dependency() {
    let c = generated(
        "shape Outer [\n    inner: Inner\n]\n\nshape Inner [\n    value: i32\n]\n\nflow make\n    => Outer\n{\n    => Outer[ inner: Inner[ value: 1 ] ]\n}\n",
    );

    let inner = c.find("struct loi_t_Inner {").expect("inner struct");
    let outer = c.find("struct loi_t_Outer {").expect("outer struct");
    assert!(
        inner < outer,
        "dependent interfaces must be declared first:\n{c}"
    );
}

#[test]
fn detects_interface_value_cycles() {
    // The checker accepts this shape; the C backend cannot lay it out by value.
    let tokens = tokenize(
        "shape A [\n    b: B\n]\n\nshape B [\n    a: A\n]\n",
        "<test>",
    )
    .expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    let error = generate_c(&program, None).expect_err("cycle should be rejected");
    assert!(error.message.contains("contains itself"));
}

#[test]
fn compiled_slices_mutation_and_out_defaults() {
    let source = "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nshape World [\n    ticks: u64 = 7\n    gravity: Vec2\n]\n\nshape Item [\n    value: i32\n]\n\nflow fill\n    -> items: [Item]\n    <- n: u32\n{\n    alloc(-> items, Item, n)\n    for i in 0..n {\n        items[i] = Item[ value: i as i32 ]\n    }\n}\n\nflow sum\n    <- items: [Item]\n    => i32\n{\n    mut total: i32 <- 0\n    for <- item in items {\n        total += item.value\n    }\n    => total\n}\n\nflow bump\n    <-> value: i32\n    <- amount: i32\n{\n    value += amount\n}\n\nflow new_world\n    -> w: World\n{\n    w.gravity = Vec2[ x: 1, y: 2 ]\n}\n";

    let harness = r#"
int main(void) {
    size_t save = loi_arena_top;

    LoiSlice items = (LoiSlice){0};
    loi_fn_fill(&items, 100);
    if (items.len != 100) return 10;
    if (loi_fn_sum(items) != 4950) return 11;

    int32_t value = 10;
    loi_fn_bump(&value, 5);
    if (value != 15) return 12;

    loi_t_World world;
    loi_fn_new_world(&world);
    if (world.loi_f_ticks != 7) return 13;
    if (world.loi_f_gravity.loi_f_x != 1 || world.loi_f_gravity.loi_f_y != 2) return 14;

    loi_arena_top = save;
    return 0;
}
"#;

    compile_and_run("slices", &generated(source), harness);
}

#[test]
fn compiled_control_flow_and_calls() {
    let source = "flow classify\n    <- value: i32\n    => i32\n{\n    if value < 0 {\n        => -1\n    } else if value == 0 {\n        => 0\n    } else {\n        => 1\n    }\n}\n\nflow count_positive\n    <- values: [i32]\n    => i32\n{\n    mut count: i32 <- 0\n    for <- value in values {\n        if value > 0 {\n            count += 1\n        } else {\n            continue\n        }\n    }\n    => count\n}\n";

    let harness = r#"
int main(void) {
    if (loi_fn_classify(-5) != -1) return 20;
    if (loi_fn_classify(0) != 0) return 21;
    if (loi_fn_classify(9) != 1) return 22;

    size_t save = loi_arena_top;
    int32_t values[] = { -2, 0, 3, 7, -1, 4 };
    LoiSlice slice = { values, 6 };
    int32_t count = loi_fn_count_positive(slice);
    loi_arena_top = save;
    if (count != 3) return 23;
    return 0;
}
"#;

    compile_and_run("control-flow", &generated(source), harness);
}

#[test]
fn compiled_protect_restores_the_arena() {
    let source = "shape Item [\n    value: i32\n]\n\nflow spend\n    <- n: u32\n{\n    protect {\n        mut items: [Item]\n        alloc(-> items, Item, n)\n        for i in 0..n {\n            items[i] = Item[ value: i as i32 ]\n        }\n    }\n}\n\nflow pick\n    <- n: u32\n    => i32\n{\n    protect {\n        mut items: [Item]\n        alloc(-> items, Item, n)\n        for i in 0..n {\n            items[i] = Item[ value: i as i32 * 2 ]\n        }\n        => 42\n    }\n}\n";

    let harness = r#"
int main(void) {
    size_t save = loi_arena_top;
    loi_fn_spend(1000);
    if (loi_arena_top != save) return 30;
    if (loi_fn_pick(10) != 42) return 31;
    if (loi_arena_top != save) return 32;
    return 0;
}
"#;

    compile_and_run("protect", &generated(source), harness);
}

#[test]
fn compiled_intrinsic_sqrt() {
    let source = "flow root\n    <- x: f64\n    => f64\n{\n    => sqrt(x)\n}\n";

    let harness = r#"
int main(void) {
    if (loi_fn_root(16.0) != 4.0) return 50;
    if (loi_fn_root(2.0) != 1.4142135623730951) return 51;
    return 0;
}
"#;

    compile_and_run("intrinsic-sqrt", &generated(source), harness);
}

#[test]
fn compiled_intrinsic_print_generates_fwrite() {
    let source = "flow greet\n    -> buf: [u8]\n    <- n: u32\n{\n    alloc(-> buf, u8, n)\n    buf[0] = 72\n    print(buf)\n}\n";

    let c = generated(source);
    assert!(c.contains("fwrite("), "print should lower to fwrite:\n{c}");
    assert!(c.contains(".data"), "fwrite should use slice data:\n{c}");
    assert!(c.contains(".len"), "fwrite should use slice len:\n{c}");
}

#[test]
fn generated_break_emits_break_statement() {
    let c = generated(
        "flow find_first\n    <- items: [i32]\n    => i32\n{\n    for <- item in items {\n        if item == 42 {\n            break\n        }\n    }\n    => 0\n}\n",
    );
    assert!(c.contains("break;"), "generated C should emit break:\n{c}");
}

#[test]
fn generated_hex_and_binary_literals() {
    let c = generated(
        "flow f\n    => u32\n{\n    => 0xFF\n}\n\nflow g\n    => u32\n{\n    => 0b10101010\n}\n",
    );
    assert!(c.contains("0xFF"), "hex literal should be preserved:\n{c}");
    assert!(
        c.contains("0b10101010"),
        "binary literal should be preserved:\n{c}"
    );
}

#[test]
fn generated_source_map_contains_entries() {
    let source =
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow main\n    => i32\n{\n    => 0\n}\n";
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    let dir = std::env::temp_dir().join(format!("loi-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir should be creatable");
    let map_path = dir.join("test.map");
    let _c = generate_c(&program, Some(map_path.as_path())).expect("source should generate C");
    let map_content = std::fs::read_to_string(&map_path).expect("source map should exist");
    let lines: Vec<&str> = map_content.lines().filter(|l| !l.is_empty()).collect();
    assert!(!lines.is_empty(), "source map should have entries");
    // Check that interface entries are present
    let has_interface = lines.iter().any(|l| l.contains(r#""kind":"interface""#));
    assert!(has_interface, "should have interface entries");
    // Check that function entries are present
    let has_function = lines.iter().any(|l| l.contains(r#""kind":"function""#));
    assert!(has_function, "should have function entries");
    // Check that required fields are present
    for line in &lines {
        assert!(
            line.contains(r#""c_line""#),
            "line should have c_line: {line}"
        );
        assert!(
            line.contains(r#""c_column""#),
            "line should have c_column: {line}"
        );
        assert!(
            line.contains(r#""loi_line""#),
            "line should have loi_line: {line}"
        );
        assert!(
            line.contains(r#""loi_column""#),
            "line should have loi_column: {line}"
        );
        assert!(line.contains(r#""kind""#), "line should have kind: {line}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn generated_strings_slices_len_and_globals() {
    let c = generated(
        "GREETING: [u8] <- \"hi\"\n\nLIMIT: i32 <- 3\n\nflow greet\n    => u64\n{\n    => GREETING.len\n}\n\nflow same\n    <- a: [u8]\n    <- b: [u8]\n    => bool\n{\n    => a == b\n}\n\nflow values\n    => i32\n{\n    xs <- [1, 2, 3]\n    => len(xs) as i32 + xs.len as i32\n}\n",
    );

    assert!(
        c.contains("static unsigned char loi_str_0[] = { 104, 105, 0 };"),
        "string literals should become NUL-terminated static byte arrays:\n{c}"
    );
    assert!(
        c.contains("LoiSlice loi_g_GREETING"),
        "globals should be emitted as C globals:\n{c}"
    );
    assert!(
        c.contains("int32_t loi_g_LIMIT = 3;"),
        "scalar globals should keep their initializer:\n{c}"
    );
    assert!(
        c.contains("loi_slice_eq("),
        "byte-slice equality should use the runtime helper:\n{c}"
    );
    assert!(
        c.contains("((uint64_t)(loi_g_GREETING).len)"),
        "member .len should read the slice length:\n{c}"
    );
    assert!(
        c.contains("(int32_t[]){ 1, 2, 3 }"),
        "slice literals should lower to C compound literals:\n{c}"
    );
}

#[test]
fn compiled_strings_slices_len_globals_and_equality() {
    let source = "LIMIT: i32 <- 3\n\nGREETING: [u8] <- \"loi\\n\"\n\nflow total\n    <- xs: [i32]\n    => i32\n{\n    mut sum: i32 <- 0\n    for i in 0..len(xs) {\n        sum += xs[i]\n    }\n    => sum\n}\n\nflow make_sum\n    => i32\n{\n    xs <- [1, 2, 3, 4]\n    => total(xs)\n}\n\nflow greeting_len\n    => u64\n{\n    => GREETING.len\n}\n\nflow same\n    <- a: [u8]\n    <- b: [u8]\n    => bool\n{\n    => a == b\n}\n\nflow limit\n    => i32\n{\n    => LIMIT\n}\n";

    let harness = r#"
int main(void) {
    int32_t values[] = { 10, 20, 30 };
    LoiSlice slice = { values, 3 };
    if (loi_fn_total(slice) != 60) return 30;
    if (loi_fn_make_sum() != 10) return 31;
    if (loi_fn_greeting_len() != 4) return 32;

    LoiSlice greeting = loi_g_GREETING;
    if (!loi_fn_same(greeting, greeting)) return 33;
    LoiSlice other = { (void*)"loi\n", 4 };
    if (!loi_fn_same(greeting, other)) return 34;
    LoiSlice different = { (void*)"loi!", 4 };
    if (loi_fn_same(greeting, different)) return 35;

    if (loi_fn_limit() != 3) return 36;
    return 0;
}
"#;

    compile_and_run("strings-slices-globals", &generated(source), harness);
}

#[test]
fn extern_flows_use_c_symbols_and_cstr_literals_are_nul_terminated() {
    let c = generated(
        "opaque Handle\n\n\
         extern flow make\n\
             <- x: i32\n\
             => Handle\n\n\
         extern flow release\n\
             <- h: Handle\n\n\
         extern flow title as c_title\n\
             <- text: cstr\n\
             => i32\n\n\
         flow main\n\
             => i32\n\
         {\n\
             mut h: Handle\n\
             h = make(7)\n\
             release(h)\n\
             => title(\"loi\")\n\
         }\n",
    );

    assert!(c.contains("void * make(int32_t loi_v_x);"), "{c}");
    assert!(c.contains("void release(void * loi_v_h);"), "{c}");
    assert!(
        c.contains("int32_t c_title(const char * loi_v_text);"),
        "{c}"
    );
    assert!(c.contains("make(7)"), "{c}");
    assert!(c.contains("release(loi_v_h)"), "{c}");
    assert!(c.contains("c_title(((const char*)loi_str_0))"), "{c}");
    assert!(
        c.contains("static unsigned char loi_str_0[] = { 108, 111, 105, 0 };"),
        "{c}"
    );
    assert!(!c.contains("loi_fn_make"), "{c}");
}

#[test]
fn generated_math_intrinsics_and_eprint() {
    let c = generated(
        "flow math\n    <- x: f64\n    => f64\n{\n    => floor(x) + ceil(x) + fabs(x) + pow(x, 2.0) + fmod(x, 3.0)\n}\n\nflow warn\n    <- msg: [u8]\n{\n    eprint(msg)\n}\n",
    );

    for call in ["floor(", "ceil(", "fabs(", "pow(", "fmod("] {
        assert!(c.contains(call), "expected {call} in generated C:\n{c}");
    }
    assert!(
        c.contains("fwrite(") && c.contains("stderr"),
        "eprint should write to stderr:\n{c}"
    );
}

const ARITHMETIC_SOURCE: &str = "flow add_i32\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    => a + b\n}\n\nflow sub_i8\n    <- a: i8\n    <- b: i8\n    => i8\n{\n    => a - b\n}\n\nflow mul_i32\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    => a * b\n}\n\nflow neg_i32\n    <- a: i32\n    => i32\n{\n    => -a\n}\n\nflow div_i32\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    => a / b\n}\n\nflow rem_i32\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    => a % b\n}\n\nflow add_u8\n    <- a: u8\n    <- b: u8\n    => u8\n{\n    => a + b\n}\n\nflow sub_u8\n    <- a: u8\n    <- b: u8\n    => u8\n{\n    => a - b\n}\n\nflow mul_u8\n    <- a: u8\n    <- b: u8\n    => u8\n{\n    => a * b\n}\n\nflow add_u32\n    <- a: u32\n    <- b: u32\n    => u32\n{\n    => a + b\n}\n\nflow sub_u64\n    <- a: u64\n    <- b: u64\n    => u64\n{\n    => a - b\n}\n\nflow div_u64\n    <- a: u64\n    <- b: u64\n    => u64\n{\n    => a / b\n}\n\nflow rem_u64\n    <- a: u64\n    <- b: u64\n    => u64\n{\n    => a % b\n}\n";

#[test]
fn signed_arithmetic_uses_checked_helpers() {
    let c = generated(ARITHMETIC_SOURCE);
    for helper in [
        "loi_add_i32(",
        "loi_sub_i8(",
        "loi_mul_i32(",
        "loi_neg_i32(",
        "loi_div_i32(",
        "loi_rem_i32(",
    ] {
        assert!(c.contains(helper), "missing {helper}:\n{c}");
    }
    assert!(c.contains("loi_div_u64("), "{c}");
    assert!(c.contains("loi_rem_u64("), "{c}");
}

#[test]
fn signed_arithmetic_traps_on_overflow() {
    let cases = [
        (
            "add-i32",
            "int main(void) { return (int)loi_fn_add_i32(INT32_MAX, 1); }",
        ),
        (
            "sub-i8",
            "int main(void) { return (int)loi_fn_sub_i8(INT8_MIN, 1); }",
        ),
        (
            "mul-i32",
            "int main(void) { return (int)loi_fn_mul_i32(100000, 100000); }",
        ),
        (
            "neg-i32",
            "int main(void) { return (int)loi_fn_neg_i32(INT32_MIN); }",
        ),
    ];
    for (name, harness) in cases {
        assert_runtime_trap(
            name,
            &generated(ARITHMETIC_SOURCE),
            harness,
            "integer overflow",
        );
    }
}

#[test]
fn division_and_remainder_by_zero_trap_with_a_position() {
    assert_runtime_trap(
        "div-zero",
        &generated(ARITHMETIC_SOURCE),
        "int main(void) { return (int)loi_fn_div_i32(10, 0); }",
        "division by zero",
    );
    assert_runtime_trap(
        "rem-zero",
        &generated(ARITHMETIC_SOURCE),
        "int main(void) { return (int)loi_fn_rem_i32(10, 0); }",
        "remainder by zero",
    );
}

#[test]
fn signed_min_divided_by_negative_one_traps_but_remainder_is_zero() {
    assert_runtime_trap(
        "min-div-minus-one",
        &generated(ARITHMETIC_SOURCE),
        "int main(void) { return (int)loi_fn_div_i32(INT32_MIN, -1); }",
        "integer overflow",
    );

    let harness = r#"
int main(void) {
    if (loi_fn_rem_i32(INT32_MIN, -1) != 0) return 1;
    if (loi_fn_div_i32(INT32_MIN, 1) != INT32_MIN) return 2;
    if (loi_fn_div_i32(-7, 2) != -3) return 3;
    if (loi_fn_rem_i32(-7, 2) != -1) return 4;
    return 0;
}
"#;
    compile_and_run("min-rem-minus-one", &generated(ARITHMETIC_SOURCE), harness);
}

#[test]
fn unsigned_arithmetic_wraps_at_its_declared_width() {
    let harness = r#"
int main(void) {
    if (loi_fn_add_u8(255, 1) != 0) return 1;
    if (loi_fn_sub_u8(0, 1) != 255) return 2;
    if (loi_fn_mul_u8(200, 2) != 144) return 3;
    if (loi_fn_add_u32(UINT32_MAX, 1) != 0) return 4;
    if (loi_fn_sub_u64(0, 1) != UINT64_MAX) return 5;
    if (loi_fn_div_u64(100, 3) != 33) return 6;
    if (loi_fn_rem_u64(100, 3) != 1) return 7;
    return 0;
}
"#;
    compile_and_run("unsigned-wrap", &generated(ARITHMETIC_SOURCE), harness);
}

#[test]
fn arena_grows_in_chunks_and_reuses_space_after_reset() {
    let source = "flow fill\n    <- n: u32\n    => u64\n{\n    protect {\n        mut total: u64 <- 0\n        for i in 0..n {\n            mut items: [i64]\n            alloc(-> items, i64, 64)\n            items[0] = i as i64\n            total += items[0] as u64\n        }\n        => total\n    }\n}\n\nflow one_big\n    => i64\n{\n    protect {\n        mut items: [i64]\n        alloc(-> items, i64, 10000)\n        items[9999] = 7\n        => items[9999]\n    }\n}\n";
    // Keep the first chunk tiny so the test does not need to allocate 64 MiB.
    let c = format!("#define LOI_ARENA_SIZE 1024\n{}", generated(source));
    let harness = r#"
int main(void) {
    size_t save = loi_arena_top;
    if (loi_fn_fill(10) != 45) return 1;
    if (loi_arena_capacity <= 1024) return 2;
    if (loi_arena_top != save) return 3;
    if (loi_fn_fill(10) != 45) return 4;
    if (loi_fn_one_big() != 7) return 5;
    if (loi_arena_capacity <= 80000) return 6;
    loi_arena_top = save;
    if (loi_fn_fill(1) != 0) return 7;
    return 0;
}
"#;
    compile_and_run("arena-grow", &c, harness);
}

#[test]
fn compiled_bitwise_shifts_and_compound_operators() {
    let source = "flow combine\n    <- a: u32\n    <- b: u32\n    => u32\n{\n    => (a | b) ^ (a & b)\n}\n\nflow left\n    <- x: i32\n    <- n: i32\n    => i32\n{\n    => x << n\n}\n\nflow arithmetic\n    <- x: i32\n    => i32\n{\n    => x >> 1\n}\n\nflow invert\n    <- x: u8\n    => u8\n{\n    => ~x\n}\n\nflow compound\n    => i32\n{\n    mut x: i32 <- 0x0F\n    x <<= 4\n    x |= 3\n    x &= 0xF3\n    x ^= 1\n    x %= 7\n    => x\n}\n";
    let harness = r#"
int main(void) {
    if (loi_fn_combine(0xF0, 0x0F) != 0xFF) return 1;
    if (loi_fn_combine(0xAA, 0xAA) != 0) return 2;
    if (loi_fn_left(1, 4) != 16) return 3;
    if (loi_fn_left(0x8000, 1) != 0x10000) return 4;
    if (loi_fn_arithmetic(-8) != -4) return 5;
    if (loi_fn_arithmetic(7) != 3) return 6;
    if (loi_fn_invert(0) != 0xFF) return 7;
    if (loi_fn_compound() != 4) return 8;
    return 0;
}
"#;
    compile_and_run("bitwise", &generated(source), harness);
}

#[test]
fn compiled_bitwise_shift_count_traps_with_a_position() {
    let source = "flow shift\n    <- x: i32\n    <- n: i32\n    => i32\n{\n    => x << n\n}\n";
    let harness = r#"
int main(void) {
    return loi_fn_shift(1, 32);
}
"#;
    assert_runtime_trap(
        "shift-count",
        &generated(source),
        harness,
        "shift count out of range",
    );
}

#[test]
fn compiled_shift_count_is_checked_at_its_own_width() {
    // The count is i64 while the shifted value is i32. Narrowing the count
    // before the range check would turn 2^32 into 0 and silently shift by 0.
    let source = "flow shift_wide\n    <- x: i32\n    <- n: i64\n    => i32\n{\n    => x << n\n}\n";
    let harness = r#"
int main(void) {
    return loi_fn_shift_wide(1, 4294967296LL);
}
"#;
    assert_runtime_trap(
        "shift-count-width",
        &generated(source),
        harness,
        "shift count out of range",
    );
}

#[test]
fn compiled_reads_and_writes_files() {
    let path = format!("/tmp/opencode/loi-codegen-io-{}.dat", std::process::id());
    let source = "flow roundtrip\n    <- path: [u8]\n    => i32\n{\n    protect {\n        mut data: [u8] <- \"hello\"\n        if not write_file(<- path, <- data) {\n            => 1\n        }\n        mut read_back: [u8] <- read_file(<- path)\n        if read_back.len != 5 {\n            => 2\n        }\n        if read_back[0] != 104 {\n            => 3\n        }\n        if read_back[4] != 111 {\n            => 4\n        }\n        => 0\n    }\n}\n";
    let harness = format!(
        r#"
int main(void) {{
    const char* name = "{path}";
    LoiSlice path = {{ (void*)name, strlen(name) }};
    int code = loi_fn_roundtrip(path);
    remove(name);
    return code;
}}
"#
    );
    compile_and_run("file-io", &generated(source), &harness);
}

#[test]
fn compiled_reads_stdin_lines_and_the_remainder() {
    let input_path = format!("/tmp/opencode/loi-codegen-stdin-{}.txt", std::process::id());
    fs::write(&input_path, "hello\nrest\n").expect("stdin fixture should be writable");

    let source = "flow line_len\n    => i32\n{\n    protect {\n        mut line: [u8] <- read_line()\n        => line.len as i32\n    }\n}\n\nflow rest_len\n    => i32\n{\n    protect {\n        mut rest: [u8] <- read_stdin()\n        => rest.len as i32\n    }\n}\n";
    let harness = format!(
        r#"
int main(void) {{
    if (!freopen("{input_path}", "r", stdin)) return 1;
    if (loi_fn_line_len() != 5) return 2;
    if (loi_fn_rest_len() != 5) return 3;
    return 0;
}}
"#
    );
    let result = compile_and_run_output("stdin-io", &generated(source), &harness);
    let _ = fs::remove_file(&input_path);
    let Some(run) = result else {
        return;
    };
    assert_eq!(
        run.status.code(),
        Some(0),
        "stdin test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn compiled_numeric_printing_handles_widths_and_floats() {
    let source = "flow show {\n    print_int(255 as u8)\n    print(<- \" \")\n    print_int(-7 as i16)\n    print(<- \" \")\n    print_int(4294967295 as u32)\n    print(<- \" \")\n    print_float(0.1)\n    print(<- \" \")\n    print_float(1.0)\n    print(<- \"\\n\")\n}\n";
    let harness = r#"
int main(void) {
    loi_fn_show();
    return 0;
}
"#;
    let Some(run) = compile_and_run_output("numeric-print", &generated(source), harness) else {
        return;
    };
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "255 -7 4294967295 0.1 1\n"
    );
}

#[test]
fn generated_generic_instantiations_are_mangled() {
    let c = generated(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow sum<T>\n    <- xs: Vec<T>\n    => i32\n{\n    mut total: i32 <- 0\n    for i in 0..xs.len {\n        total += xs.data[i] as i32\n    }\n    => total\n}\n\nflow use\n    => i32\n{\n    xs <- Vec<i32>[ data: [i32: 1, 2, 3], len: 3 ]\n    => sum<i32>(xs)\n}\n",
    );

    assert!(
        c.contains("struct loi_t_Vec__i32"),
        "generic shape should be monomorphized:\n{c}"
    );
    assert!(
        c.contains("loi_fn_sum__i32"),
        "generic flow should be monomorphized:\n{c}"
    );
    assert!(
        !c.contains("loi_t_Vec<"),
        "template shapes must not reach C:\n{c}"
    );
}

#[test]
fn compiled_generic_shapes_and_flows_run() {
    let source = "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow sum<T>\n    <- xs: Vec<T>\n    => i32\n{\n    mut total: i32 <- 0\n    for i in 0..xs.len {\n        total += xs.data[i] as i32\n    }\n    => total\n}\n\nflow set_at<T>\n    <-> xs: Vec<T>\n    <- index: u64\n    <- value: T\n{\n    xs.data[index] = value\n}\n\nflow use\n    => i32\n{\n    mut v <- Vec<i32>[ data: [i32: 1], len: 1 ]\n    set_at<i32>(<-> v, 0, 2)\n    => sum<i32>(v)\n}\n";
    let harness = r#"
int main(void) {
    int32_t values[] = { 1, 2, 3 };
    loi_t_Vec__i32 v = { .loi_f_data = { values, 3 }, .loi_f_len = 3 };
    if (loi_fn_sum__i32(v) != 6) return 40;
    loi_fn_set_at__i32(&v, 1, 42);
    if (loi_fn_sum__i32(v) != 46) return 41;
    return 0;
}
"#;
    compile_and_run("generic-shapes-flows", &generated(source), harness);
}

#[test]
fn composite_globals_use_static_initializers() {
    let c = generated(
        "shape Color [\n    r: u8\n    g: u8\n    b: u8\n    a: u8\n]\n\nshape Scene [\n    color: Color\n    points: [i32]\n    name: [u8]\n]\n\nGREEN: Color <- Color[ r: 0, g: 255, b: 0, a: 255 ]\n\nSCENE: Scene <- Scene[ color: Color[ r: 1, g: 2, b: 3, a: 4 ], points: [i32: 7, 8], name: \"scene\" ]\n",
    );

    assert!(
        c.contains("loi_t_Color loi_g_GREEN = { .loi_f_r = 0, .loi_f_g = 255, .loi_f_b = 0, .loi_f_a = 255 };"),
        "shape globals should use designated initializers without a compound literal:\n{c}"
    );
    assert!(
        c.contains("static int32_t loi_gs_0[] = { 7, 8 };"),
        "slice data should be a named static array:\n{c}"
    );
    assert!(
        c.contains("loi_t_Scene loi_g_SCENE = { .loi_f_color = { .loi_f_r = 1, .loi_f_g = 2, .loi_f_b = 3, .loi_f_a = 4 }, .loi_f_points = { (void*)loi_gs_0, 2 }, .loi_f_name = { (void*)loi_str_0, 5 } };"),
        "nested shape globals should use plain designated initializers:\n{c}"
    );
}

#[test]
fn compiled_composite_globals_and_comptime_values() {
    let source = "shape Color [\n    r: u8\n    g: u8\n    b: u8\n    a: u8\n]\n\nGREEN: Color <- Color[ r: 0, g: 255, b: 0, a: 255 ]\n\nPOINTS: [i32] <- [i32: 4, 5, 6]\n\ncomptime flow triple\n    <- x: i32\n    => i32\n{\n    => x * 3\n}\n\nLIMIT: i32 <- comptime triple(7)\n\nflow next\n    => i32\n{\n    => comptime triple(4)\n}\n";
    let harness = r#"
int main(void) {
    if (loi_g_GREEN.loi_f_g != 255) return 50;
    if (loi_g_GREEN.loi_f_r != 0) return 51;
    if (loi_g_POINTS.len != 3) return 52;
    if (((int32_t*)loi_g_POINTS.data)[2] != 6) return 53;
    if (loi_g_LIMIT != 21) return 54;
    if (loi_fn_next() != 12) return 55;
    return 0;
}
"#;
    compile_and_run("composite-globals-comptime", &generated(source), harness);
}

#[test]
fn composite_globals_compile_as_strict_c11() {
    let source = "shape Color [\n    r: u8\n    g: u8\n    b: u8\n    a: u8\n]\n\nshape Scene [\n    color: Color\n    points: [i32]\n    name: [u8]\n]\n\nGREEN: Color <- Color[ r: 0, g: 255, b: 0, a: 255 ]\nPOINTS: [i32] <- [i32: 4, 5, 6]\nSCENE: Scene <- Scene[ color: Color[ r: 1, g: 2, b: 3, a: 4 ], points: [i32: 7, 8], name: \"scene\" ]\n";
    let c = generated(source);
    if !has_c_compiler() {
        eprintln!("skipping strict c11 test: no C compiler found");
        return;
    }

    let dir = PathBuf::from("/tmp/opencode")
        .join(format!("loi-codegen-{}-strict-c11", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    let path = dir.join("generated.c");
    fs::write(&path, &c).expect("generated C should be writable");
    let output = Command::new("cc")
        .arg("-std=c11")
        .arg("-pedantic-errors")
        .arg("-fsyntax-only")
        .arg(&path)
        .output()
        .expect("cc should run");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "strict C11 rejected the generated code:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn compiled_constant_globals_with_expressions_and_negation() {
    let source = "SIZE: i32 <- 2 + 3\nNEG: i32 <- -7\n\nflow main\n    => i32\n{\n    if SIZE == 5 and NEG == -7 {\n        => 0\n    }\n    => 1\n}\n";
    let c = generated(source);
    assert!(c.contains("int32_t loi_g_SIZE = 5;"), "unexpected C:\n{c}");
    assert!(c.contains("int32_t loi_g_NEG = -7;"), "unexpected C:\n{c}");
    compile_and_run("constant-globals", &c, "");
}

#[test]
fn comptime_f32_rounds_like_runtime() {
    let source = "comptime flow third\n    => f32\n{\n    => 1.0 / 3.0\n}\n\nX: f32 <- comptime third()\n\nflow same\n    => bool\n{\n    y: f32 <- 1.0 / 3.0\n    => X == y\n}\n\nflow main\n    => i32\n{\n    if same() {\n        => 0\n    }\n    => 1\n}\n";
    let c = generated(source);
    assert!(
        c.contains("float loi_g_X = 0.3333333432674408;"),
        "comptime f32 should be rounded to f32 precision:\n{c}"
    );
    compile_and_run("comptime-f32", &c, "");
}

#[test]
fn compiled_printf_lowers_to_typed_varargs() {
    let source = "flow show {\n    n: i32 <- 42\n    big: i64 <- 9000000000\n    u: u64 <- 7\n    f: f64 <- 0.1\n    printf(\"n=%d big=%lld u=%llu f=%.2f s=%s pct=100%%\\n\", n, big, u, f, \"ok\")\n}\n";
    let c = generated(source);
    assert!(
        c.contains("printf(((const char*)loi_str_0)"),
        "printf should use the literal format string:\n{c}"
    );
    assert!(
        c.contains("((int)( loi_v_n ))"),
        "varargs should be cast to the conversion type:\n{c}"
    );
    assert!(
        c.contains("((long long)( loi_v_big ))"),
        "length modifiers should select the C cast:\n{c}"
    );
    assert!(
        c.contains("((const char*)loi_str_1)"),
        "%s should reuse the literal's NUL terminator:\n{c}"
    );

    let harness = r#"
int main(void) {
    loi_fn_show();
    return 0;
}
"#;
    let Some(run) = compile_and_run_output("printf", &c, harness) else {
        return;
    };
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "n=42 big=9000000000 u=7 f=0.10 s=ok pct=100%\n"
    );
}

const MARK_CHOICE_SOURCE: &str = "mark Color [ Red, Green, Blue ]\n\nchoice Result [ Ok: i32, Err: i64 ]\n\nflow classify\n    <- c: Color\n    <- r: Result\n    => i32\n{\n    if c == Color.Green and r is Result.Ok {\n        => r.Ok\n    }\n    => -1\n}\n\nflow wrong\n    => i64\n{\n    r <- Result.Ok(7)\n    => r.Err\n}\n";

#[test]
fn mark_and_choice_generate_c_representations() {
    let c = generated(MARK_CHOICE_SOURCE);

    // A mark is a scalar tag set with one macro per variant.
    assert!(c.contains("typedef uint32_t loi_t_Color;"), "{c}");
    assert!(
        c.contains("#define loi_m_Color_Green ((loi_t_Color)1u)"),
        "{c}"
    );

    // A choice is a tagged union with checked readers.
    assert!(
        c.contains("typedef struct loi_t_Result loi_t_Result;"),
        "{c}"
    );
    assert!(c.contains("struct loi_t_Result {"), "{c}");
    assert!(c.contains("uint32_t loi_f_tag;"), "{c}");
    assert!(c.contains("int32_t loi_p_Ok;"), "{c}");
    assert!(c.contains("int64_t loi_p_Err;"), "{c}");
    assert!(
        c.contains("static inline int32_t loi_get_Result_Ok(loi_t_Result value, size_t line, size_t column)"),
        "{c}"
    );
    assert!(
        c.contains("loi_runtime_fail(\"choice 'Result' does not hold 'Ok'\", line, column);"),
        "{c}"
    );

    // Constructors, `is` and payload reads lower to plain C.
    assert!(
        c.contains("((loi_t_Result){ 0u, { .loi_p_Ok = (int32_t)(7) } })"),
        "{c}"
    );
    assert!(c.contains("((loi_v_r).loi_f_tag == 0u)"), "{c}");
    assert!(c.contains("loi_get_Result_Ok(loi_v_r, 11, 12)"), "{c}");
    assert!(c.contains("loi_get_Result_Err(loi_v_r, 20, 8)"), "{c}");
}

#[test]
fn compiled_marks_and_choices_run() {
    let harness = r#"
int main(void) {
    loi_t_Result ok = { 0u, { .loi_p_Ok = 42 } };
    loi_t_Result err = { 1u, { .loi_p_Err = 9 } };

    if (loi_fn_classify(loi_m_Color_Green, ok) != 42) return 1;
    if (loi_fn_classify(loi_m_Color_Red, ok) != -1) return 2;
    if (loi_fn_classify(loi_m_Color_Green, err) != -1) return 3;
    return 0;
}
"#;
    compile_and_run("mark-choice", &generated(MARK_CHOICE_SOURCE), harness);
}

#[test]
fn mark_and_choice_generated_c_is_strict_c11_and_sanitizer_clean() {
    let c = generated(MARK_CHOICE_SOURCE);
    assert_strict_c11("mark-choice", &c);
    assert_sanitizers_compile("mark-choice", &c);
}

#[test]
fn wrong_choice_payload_aborts_with_declaration_and_position() {
    let Some(run) = compile_and_run_output(
        "choice-payload-abort",
        &generated(MARK_CHOICE_SOURCE),
        "int main(void) { return (int)loi_fn_wrong(); }",
    ) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(
        run.status.code(),
        Some(1),
        "wrong payload should abort with exit 1:\nstdout: {}\nstderr: {stderr}",
        String::from_utf8_lossy(&run.stdout)
    );
    assert!(
        stderr.contains("loi: choice 'Result' does not hold 'Err' at 20:8"),
        "expected the declaration, variant and source position in the abort message, got: {stderr}"
    );
}

#[test]
fn headers_expose_visible_marks_and_choices_only() {
    let h = generated_header(
        "open mark Color [ Red, Green ]\n\nopen choice Result [ Ok: i32, Err: i64 ]\n\nmark HiddenMark [ Only ]\n\nchoice HiddenChoice [ H: bool ]\n\nflow main\n    => i32\n{\n    => 0\n}\n",
    );

    assert!(h.contains("typedef uint32_t loi_t_Color;"), "{h}");
    assert!(
        h.contains("#define loi_m_Color_Red ((loi_t_Color)0u)"),
        "{h}"
    );
    assert!(
        h.contains("typedef struct loi_t_Result loi_t_Result;"),
        "{h}"
    );
    assert!(h.contains("struct loi_t_Result {"), "{h}");
    assert!(h.contains("uint32_t loi_f_tag;"), "{h}");
    assert!(h.contains("int32_t loi_p_Ok;"), "{h}");
    assert!(h.contains("int64_t loi_p_Err;"), "{h}");

    assert!(!h.contains("loi_t_HiddenMark"), "{h}");
    assert!(!h.contains("loi_t_HiddenChoice"), "{h}");
}

#[test]
fn choices_and_shapes_share_the_by_value_layout_order() {
    let c = generated(
        "shape Wrapper [\n    inner: Result\n]\n\nchoice Result [\n    Ok: i32,\n    Err: i64\n]\n",
    );

    let choice = c
        .find("struct loi_t_Result {")
        .expect("choice struct should be emitted");
    let wrapper = c
        .find("struct loi_t_Wrapper {")
        .expect("shape struct should be emitted");
    assert!(
        choice < wrapper,
        "the shape embeds the choice, so the choice layout must come first:\n{c}"
    );
}

#[test]
fn partial_moves_compile_and_swap_runtime_elements() {
    let source = r#"flow swap
    <-> v: [u64]
    <- i: u64
    <- j: u64
{
    tmp: u64 <- give(v[i])
    v[i] = give(v[j])
    v[j] = give(tmp)
}

flow main
    => i32
{
    protect {
        mut xs: [u64]
        alloc(-> xs, u64, 3)
        xs[0] = 1
        xs[1] = 2
        xs[2] = 3
        swap(<-> xs, 0, 2)
        print_int(xs[0])
        print_int(xs[1])
        print_int(xs[2])
    }
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("partial-move-swap", &c);
    assert_sanitizers_compile("partial-move-swap", &c);
    let Some(run) = compile_and_run_output("partial-move-swap", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "321");
}

// ---------------------------------------------------------------------------
// Block A2: recursive clone with slices
// ---------------------------------------------------------------------------

#[test]
fn recursive_clone_of_a_shape_creates_independent_storage() {
    let source = r#"shape World [
    particles: [i32]
]

flow main
    => i32
{
    protect {
        mut w: World
        alloc(-> w.particles, i32, 2)
        w.particles[0] = 1
        w.particles[1] = 2
        mut copy: World <- clone(w)
        copy.particles[0] = 9
        print_int(w.particles[0])
        print_int(copy.particles[0])
        print_int(w.particles[1])
        print_int(copy.particles[1])
    }
    => 0
}
"#;
    let c = generated(source);
    assert!(
        c.contains("loi_clone_world"),
        "expected a generated shape clone helper:\n{c}"
    );
    assert!(
        c.contains("loi_clone_slice_i32"),
        "expected a generated slice clone helper:\n{c}"
    );
    assert_strict_c11("recursive-clone-shape", &c);
    assert_sanitizers_compile("recursive-clone-shape", &c);
    let Some(run) = compile_and_run_output("recursive-clone-shape", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "1922");
}

#[test]
fn recursive_clone_of_nested_shapes_creates_independent_storage() {
    let source = r#"shape Inner [
    data: [u64]
]

shape Outer [
    left: Inner
    right: Inner
]

flow main
    => i32
{
    protect {
        mut o: Outer
        alloc(-> o.left.data, u64, 1)
        alloc(-> o.right.data, u64, 1)
        o.left.data[0] = 3
        o.right.data[0] = 4
        mut copy: Outer <- clone(o)
        copy.left.data[0] = 7
        copy.right.data[0] = 8
        print_int(o.left.data[0])
        print_int(copy.left.data[0])
        print_int(o.right.data[0])
        print_int(copy.right.data[0])
    }
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("recursive-clone-nested", &c);
    assert_sanitizers_compile("recursive-clone-nested", &c);
    let Some(run) = compile_and_run_output("recursive-clone-nested", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "3748");
}

#[test]
fn recursive_clone_of_a_slice_copies_elements() {
    let source = r#"flow main
    => i32
{
    protect {
        xs: [u8] <- "abc"
        mut ys: [u8] <- clone(xs)
        ys[0] = 90
        print_int(xs[0])
        print_int(ys[0])
        print_int(xs.len)
        print_int(ys.len)
    }
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("recursive-clone-slice", &c);
    assert_sanitizers_compile("recursive-clone-slice", &c);
    let Some(run) = compile_and_run_output("recursive-clone-slice", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "979033");
}

#[test]
fn recursive_clone_of_nested_slices_creates_independent_rows() {
    let source = r#"flow main
    => i32
{
    protect {
        mut rows: [[u8]]
        alloc(-> rows, [u8], 2)
        alloc(-> rows[0], u8, 1)
        alloc(-> rows[1], u8, 1)
        rows[0][0] = 10
        rows[1][0] = 20
        mut copy: [[u8]] <- clone(rows)
        copy[0][0] = 99
        print_int(rows[0][0])
        print_int(copy[0][0])
        print_int(rows[1][0])
        print_int(copy[1][0])
    }
    => 0
}
"#;
    let c = generated(source);
    assert!(
        c.contains("loi_clone_slice_slice_u8"),
        "expected a nested-slice clone helper:\n{c}"
    );
    assert_strict_c11("recursive-clone-nested-slices", &c);
    assert_sanitizers_compile("recursive-clone-nested-slices", &c);
    let Some(run) = compile_and_run_output("recursive-clone-nested-slices", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "10992020");
}

#[test]
fn clone_of_an_empty_slice_is_safe() {
    let source = r#"flow main
    => i32
{
    protect {
        xs: [u64] <- [u64:]
        mut ys: [u64] <- clone(xs)
        print_int(ys.len)
    }
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("recursive-clone-empty", &c);
    assert_sanitizers_compile("recursive-clone-empty", &c);
    let Some(run) = compile_and_run_output("recursive-clone-empty", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "0");
}

// ---------------------------------------------------------------------------
// Block A3: lexical read views of partially consumed composites
// ---------------------------------------------------------------------------

#[test]
fn field_limited_read_view_compiles_and_runs() {
    let source = r#"shape Cell [
    value: u64
]

shape Vec [
    data: [Cell]
    len: u64
]

flow vec_len
    <- v: Vec
    => u64
{
    => v.len
}

flow main
    => i32
{
    protect {
        mut v: Vec
        alloc(-> v.data, Cell, 2)
        v.data[0].value = 7
        v.len = 2
        taken <- give(v.data[0])
        print_int(vec_len(v))
        print_int(taken.value)
    }
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("read-view-vec-len", &c);
    assert_sanitizers_compile("read-view-vec-len", &c);
    let Some(run) = compile_and_run_output("read-view-vec-len", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "27");
}

// ---------------------------------------------------------------------------
// Block A4: empty slice literals and writable slice length
// ---------------------------------------------------------------------------

#[test]
fn empty_slice_literal_out_param_compiles_and_runs() {
    let source = r#"shape Vec [
    data: [u64]
    len: u64
    cap: u64
]

flow vec_new
    -> v: Vec
{
    v.data = [u64:]
    v.len = 0
    v.cap = 0
}

flow vec_len
    <- v: Vec
    => u64
{
    => v.len
}

flow main
    => i32
{
    mut v: Vec
    vec_new(-> v)
    print_int(vec_len(v))
    print_int(v.cap)
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("empty-slice-out-param", &c);
    assert_sanitizers_compile("empty-slice-out-param", &c);
    let Some(run) = compile_and_run_output("empty-slice-out-param", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "00");
}

#[test]
fn bounded_generic_flow_keeps_the_same_mangling_and_runs() {
    let source = r#"flow find<T: Clone + Eq>
    <- xs: [T]
    <- needle: T
    => i64
{
    for i in 0..xs.len {
        if xs[i] == needle {
            => i as i64
        }
    }
    => -1
}

flow main
    => i32
{
    protect {
        mut xs: [i32]
        alloc(-> xs, i32, 3)
        xs[0] = 4
        xs[1] = 5
        xs[2] = 6
        print_int(find<i32>(xs, 5))
    }
    => 0
}
"#;
    let c = generated(source);
    assert!(
        c.contains("find__i32"),
        "bounds must not change the generated name"
    );
    assert_strict_c11("bounded-generic", &c);
    assert_sanitizers_compile("bounded-generic", &c);
    let Some(run) = compile_and_run_output("bounded-generic", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "1");
}

#[test]
fn slice_len_write_compiles_and_runs() {
    let source = r#"flow set_len
    <-> s: [u64]
    <- n: u64
{
    s.len = n
}

flow main
    => i32
{
    protect {
        mut xs: [u64]
        alloc(-> xs, u64, 4)
        xs[0] = 11
        xs[1] = 22
        set_len(<-> xs, 2)
        print_int(xs.len)
        print_int(xs[0])
        print_int(xs[1])
    }
    => 0
}
"#;
    let c = generated(source);
    assert_strict_c11("slice-len-write", &c);
    assert_sanitizers_compile("slice-len-write", &c);
    let Some(run) = compile_and_run_output("slice-len-write", &c, "") else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "21122");
}

const CALLBACK_SOURCE: &str = r#"open extern flow apply_cb
    <- a: i32
    <- b: i32
    <- cb: callback
    => i32

open shape Handler [
    cb: callback
]

open flow add
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}

open flow pick
    => callback
{
    => callback(add)
}

open flow consume
    <- cb: callback
    => i32
{
    => apply_cb(1, 2, cb)
}

open flow main
    => i32
{
    cb <- callback(add)
    if apply_cb(2, 3, cb) != 5 { => 1 }
    h <- Handler[ cb: cb ]
    if apply_cb(4, 5, h.cb) != 9 { => 2 }
    p <- pick()
    if apply_cb(6, 7, p) != 13 { => 3 }
    cbs: [callback] <- [callback: cb, callback(add)]
    if apply_cb(1, 1, cbs[0]) != 2 { => 4 }
    => 0
}
"#;

#[test]
fn callbacks_pass_loi_flows_to_c_without_a_thunk() {
    let c = generated(CALLBACK_SOURCE);

    assert!(
        c.contains("typedef int32_t (*LoiCallback)(int32_t, int32_t);"),
        "missing callback typedef:\n{c}"
    );
    assert!(
        c.contains("int32_t apply_cb(int32_t loi_v_a, int32_t loi_v_b, LoiCallback loi_v_cb);"),
        "callback parameter should use the fixed C ABI:\n{c}"
    );
    assert!(
        c.contains("LoiCallback loi_v_cb = loi_fn_add;"),
        "callback(flow) should emit the flow symbol directly:\n{c}"
    );
    assert!(
        c.contains("LoiCallback loi_tmp_ret_"),
        "returned callbacks must not be emitted as void:\n{c}"
    );
    assert!(
        c.contains("LoiCallback loi_f_cb;"),
        "shape fields must keep the callback type:\n{c}"
    );
    assert!(
        c.contains("(LoiCallback[])"),
        "callback slices must have a non-void element type:\n{c}"
    );
    assert!(
        !c.contains("void loi_v_cb"),
        "callback emitted as void:\n{c}"
    );

    assert_strict_c11("callbacks", &c);
    assert_sanitizers_compile("callbacks", &c);

    // The generated translation unit already defines `main`; the harness only
    // supplies the C side of the FFI call.
    compile_and_run(
        "callbacks",
        &c,
        r#"int32_t apply_cb(int32_t a, int32_t b, LoiCallback cb) {
    return cb(a, b);
}
"#,
    );
}

#[test]
fn callbacks_example_compiles_and_runs() {
    let c = generated(include_str!("../examples/callbacks.loi"));
    assert_strict_c11("callbacks-example", &c);
    let Some(run) = compile_and_run_output(
        "callbacks-example",
        &c,
        include_str!("../examples/callbacks_shim.c"),
    ) else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "104",
        "reduce_range should apply both callbacks"
    );
}

#[test]
fn callback_signatures_example_compiles_and_runs() {
    let c = generated(include_str!("../examples/callback_signatures.loi"));
    assert_strict_c11("callback-signatures-example", &c);
    let Some(run) = compile_and_run_output(
        "callback-signatures-example",
        &c,
        include_str!("../examples/callback_signatures_shim.c"),
    ) else {
        return;
    };
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "30\n0\n1\n2\n",
        "callback signatures should drive both C helpers"
    );
}

#[test]
fn headers_expose_the_callback_abi() {
    let h = generated_header(CALLBACK_SOURCE);
    assert!(
        h.contains("typedef int32_t (*LoiCallback)(int32_t, int32_t);"),
        "header must define the callback ABI:\n{h}"
    );
    assert!(
        h.contains("int32_t loi_fn_consume(LoiCallback loi_v_cb);"),
        "header must declare callback parameters:\n{h}"
    );
    assert!(
        h.contains("LoiCallback loi_fn_pick(void);"),
        "header must declare callback results:\n{h}"
    );
    assert!(
        h.contains("LoiCallback loi_f_cb;"),
        "header must declare callback shape fields:\n{h}"
    );
}
