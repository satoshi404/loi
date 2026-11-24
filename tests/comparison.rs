use std::fs;
use std::process::Command;

use loi::{check, generate_c, parse, tokenize};

fn diagnostics(source: &str) -> Vec<loi::Diagnostic> {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    check(&program)
}

fn assert_clean(source: &str) {
    let diagnostics = diagnostics(source);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
}

fn assert_message(source: &str, expected: &str) {
    let messages = diagnostics(source)
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "expected a diagnostic containing {expected:?}, got {messages:#?}"
    );
}

const STRUCTURAL_SOURCE: &str = r#"
shape Point [
    x: i32
    y: i32
]

mark Priority [
    Low
    High
]

choice Maybe [
    None: bool
    Some: i32
]

flow eq_points
    <- a: Point
    <- b: Point
    => bool
{
    => a == b
}

flow neq_points
    <- a: Point
    <- b: Point
    => bool
{
    => a != b
}

flow eq_choices
    <- a: Maybe
    <- b: Maybe
    => bool
{
    => a == b
}

flow eq_slices
    <- a: [i32]
    <- b: [i32]
    => bool
{
    => a == b
}

flow ord_points
    <- a: Point
    <- b: Point
    => bool
{
    => a < b
}

flow ord_choices
    <- a: Maybe
    <- b: Maybe
    => bool
{
    => a >= b
}

flow ord_slices
    <- a: [i32]
    <- b: [i32]
    => bool
{
    => a > b
}

flow ord_marks
    <- a: Priority
    <- b: Priority
    => bool
{
    => a <= b
}
"#;

#[test]
fn structural_eq_is_available_for_shapes_choices_and_slices() {
    assert_clean(STRUCTURAL_SOURCE);
}

#[test]
fn ord_supports_numeric_non_byte_slices_marks_shapes_and_choices() {
    let source = format!(
        "{STRUCTURAL_SOURCE}\nflow ord_numbers\n    <- a: f64\n    <- b: f64\n    => bool\n{{\n    => a < b\n}}\n"
    );
    assert_clean(&source);
}

#[test]
fn callbacks_are_neither_comparable_nor_orderable() {
    let source = r#"
extern flow apply_cb
    <- cb: callback
    <- a: i32
    <- b: i32
    => i32

flow add
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}

flow main {
    cb <- callback(add)
    if cb == cb { }
}
"#;
    assert_message(source, "cannot compare callback with callback");

    let source = source.replace("if cb == cb { }", "if cb < cb { }");
    assert_message(&source, "cannot order callback with callback");
}

#[test]
fn opaque_handles_are_comparable_but_not_orderable() {
    let source = r#"
opaque Handle
extern flow make_handle
    => Handle

flow same
    <- a: Handle
    <- b: Handle
    => bool
{
    => a == b
}

flow main {
    a <- make_handle()
    b <- make_handle()
    if same(a, b) { }
}
"#;
    assert_clean(source);

    let source = format!(
        "{source}\nflow ordered\n    <- a: Handle\n    <- b: Handle\n    => bool\n{{\n    => a < b\n}}\n"
    );
    assert_message(&source, "cannot order Handle with Handle");
}

#[test]
fn eq_bound_accepts_shapes_choices_and_non_byte_slices() {
    let source = r#"
shape Point [
    x: i32
]
choice Maybe [
    None: bool
    Some: i32
]

flow same<T: Eq>
    <- a: T
    <- b: T
    => bool
{
    => a == b
}

flow main
    => i32
{
    p <- Point[ x: 1 ]
    q <- Point[ x: 1 ]
    if same<Point>(p, q) { }
    m <- Maybe.Some(2)
    n <- Maybe.Some(2)
    if same<Maybe>(m, n) { }
    if same<[i32]>([i32: 1], [i32: 1]) { }
    => 0
}
"#;
    assert_clean(source);
}

#[test]
fn ord_bound_accepts_shapes_and_slices_but_rejects_opaque() {
    let source = r#"
shape Point [
    x: i32
]
opaque Handle

flow before<T: Ord>
    <- a: T
    <- b: T
    => bool
{
    => a < b
}

flow main
    => i32
{
    p <- Point[ x: 1 ]
    q <- Point[ x: 2 ]
    if before<Point>(p, q) { }
    if before<[i32]>([i32: 1], [i32: 2]) { }
    => 0
}
"#;
    assert_clean(source);

    let source = r#"
opaque Handle
extern flow make_handle
    => Handle

flow before<T: Ord>
    <- a: T
    <- b: T
    => bool
{
    => a < b
}

flow main {
    a <- make_handle()
    b <- make_handle()
    if before<Handle>(a, b) { }
}
"#;
    assert_message(source, "does not satisfy the 'Ord' bound");
}

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn compile_and_run(name: &str, source: &str, harness: &str) {
    if !has_c_compiler() {
        eprintln!("skipping {name}: no C compiler found");
        return;
    }
    let dir = std::env::temp_dir().join(format!("loi-cmp-{}-{name}", std::process::id()));
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
        .arg("-Werror")
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
    assert_eq!(
        run.status.code(),
        Some(0),
        "program for {name} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn compiled_structural_comparison_runs_in_strict_c11() {
    let source = r#"
shape Point [
    x: i32
    y: i32
]

mark Priority [
    Low
    High
]

choice Maybe [
    None: bool
    Some: i32
]

flow eq_points
    <- a: Point
    <- b: Point
    => bool
{
    => a == b
}

flow ord_points
    <- a: Point
    <- b: Point
    => i32
{
    if a < b { => -1 }
    if b < a { => 1 }
    => 0
}

flow eq_choices
    <- a: Maybe
    <- b: Maybe
    => bool
{
    => a == b
}

flow ord_choices
    <- a: Maybe
    <- b: Maybe
    => i32
{
    if a < b { => -1 }
    if b < a { => 1 }
    => 0
}

flow eq_slices
    <- a: [Point]
    <- b: [Point]
    => bool
{
    => a == b
}

flow ord_slices
    <- a: [i32]
    <- b: [i32]
    => i32
{
    if a < b { => -1 }
    if b < a { => 1 }
    => 0
}

flow ord_marks
    <- a: Priority
    <- b: Priority
    => i32
{
    if a < b { => -1 }
    if b < a { => 1 }
    => 0
}
"#;

    let harness = r#"
int main(void) {
    loi_t_Point p1 = { .loi_f_x = 1, .loi_f_y = 2 };
    loi_t_Point p2 = { .loi_f_x = 1, .loi_f_y = 2 };
    loi_t_Point p3 = { .loi_f_x = 1, .loi_f_y = 3 };
    if (!loi_fn_eq_points(p1, p2)) return 1;
    if (loi_fn_eq_points(p1, p3)) return 2;
    if (loi_fn_ord_points(p1, p2) != 0) return 3;
    if (loi_fn_ord_points(p1, p3) >= 0) return 4;
    if (loi_fn_ord_points(p3, p1) <= 0) return 5;

    loi_t_Maybe m1 = { .loi_f_tag = 1u, .loi_u_payload = { .loi_p_Some = 7 } };
    loi_t_Maybe m2 = { .loi_f_tag = 1u, .loi_u_payload = { .loi_p_Some = 7 } };
    loi_t_Maybe m3 = { .loi_f_tag = 0u, .loi_u_payload = { .loi_p_None = false } };
    if (!loi_fn_eq_choices(m1, m2)) return 6;
    if (loi_fn_eq_choices(m1, m3)) return 7;
    if (loi_fn_ord_choices(m1, m3) <= 0) return 8;

    loi_t_Point points[2] = { p1, p3 };
    LoiSlice s1 = { points, 2 };
    LoiSlice s2 = { points, 2 };
    if (!loi_fn_eq_slices(s1, s2)) return 9;

    int32_t xs[3] = { 1, 2, 9 };
    int32_t ys[3] = { 1, 3, 0 };
    LoiSlice a = { xs, 3 };
    LoiSlice b = { ys, 3 };
    if (loi_fn_ord_slices(a, b) >= 0) return 10;

    if (loi_fn_ord_marks(loi_m_Priority_Low, loi_m_Priority_High) >= 0) return 11;
    return 0;
}
"#;

    let c = generate_c_from(source);
    compile_and_run("structural-comparison", &c, harness);
}

fn generate_c_from(source: &str) -> String {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    let diagnostics = check(&program);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
    generate_c(&program, None).expect("source should generate C")
}
