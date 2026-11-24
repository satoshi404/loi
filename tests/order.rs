use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-order-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_source(dir: &Path, source: &str) -> PathBuf {
    let path = dir.join("main.loi");
    fs::write(&path, source).expect("source should be writable");
    path
}

fn generated_c(source: &str, dir: &Path) -> PathBuf {
    let source_path = write_source(dir, source);
    let c_path = dir.join("out.c");
    let output = Command::new(loi())
        .arg("gen")
        .arg(&source_path)
        .arg("-o")
        .arg(&c_path)
        .output()
        .expect("loi gen should run");
    assert_success("loi gen", &output);
    c_path
}

fn has_tool(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Return the first usable clang candidate, or `None` on machines without one.
/// `LOI_TEST_CLANG` selects an explicit binary; the Termux path is only a
/// candidate and is validated like any other tool.
fn clang() -> Option<String> {
    let candidate = std::env::var("LOI_TEST_CLANG")
        .ok()
        .or_else(|| {
            let bundled = "/data/data/com.termux/files/usr/bin/clang";
            Path::new(bundled).exists().then(|| bundled.to_string())
        })
        .unwrap_or_else(|| "clang".to_string());
    has_tool(&candidate).then_some(candidate)
}

fn require_clang(test_name: &str) -> Option<String> {
    match clang() {
        Some(compiler) => Some(compiler),
        None => {
            eprintln!(
                "skipping {test_name}: clang is unavailable; set LOI_TEST_CLANG to a working clang binary"
            );
            None
        }
    }
}

fn compile_and_run(compiler: &str, c_path: &Path, exe: &Path) -> String {
    let build = Command::new(compiler)
        .args(["-std=c11", "-w"])
        .arg(c_path)
        .arg("-o")
        .arg(exe)
        .output()
        .expect("C compiler should run");
    assert_success(compiler, &build);
    let run = Command::new(exe).output().expect("program should run");
    assert_success("program", &run);
    String::from_utf8(run.stdout).expect("program output should be UTF-8")
}

const ORDER_REPRO: &str = r#"
flow first
    => u64
{
    printf("50")
    => 0
}

flow second
    => i32
{
    printf("07")
    => 7
}

open flow main
    => i32
{
    protect {
        mut xs: [i32]
        alloc(-> xs, i32, 1)
        xs[first()] = second()
    }
    printf("\n")
    => 0
}
"#;

const SIDE_EFFECT_PREFIX: &str = r#"
flow first
    => i32
{
    printf("50")
    => 2
}

flow second
    => i32
{
    printf("07")
    => 40
}

flow sink
    <- a: i32
    <- b: i32
    => i32
{
    => a + b
}
"#;

#[test]
fn repro_prints_5007_with_gcc_and_clang() {
    let Some(clang) = require_clang("repro_prints_5007_with_gcc_and_clang") else {
        return;
    };
    let dir = test_dir("repro");
    let c_path = generated_c(ORDER_REPRO, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("repro-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "5007\n",
            "{label} must follow Loi's left-to-right evaluation order"
        );
    }
}

#[test]
fn binary_operands_evaluate_left_to_right() {
    let source = format!(
        r#"{SIDE_EFFECT_PREFIX}

open flow main
    => i32
{{
    total <- first() + second()
    printf("%d\n", total)
    => 0
}}
"#
    );
    let Some(clang) = require_clang("binary_operands_evaluate_left_to_right") else {
        return;
    };
    let dir = test_dir("binary");
    let c_path = generated_c(&source, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("binary-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "500742\n",
            "{label} evaluated binary operands in the wrong order"
        );
    }
}

#[test]
fn call_arguments_evaluate_left_to_right() {
    let source = format!(
        r#"{SIDE_EFFECT_PREFIX}

open flow main
    => i32
{{
    total <- sink(first(), second())
    printf("%d\n", total)
    => 0
}}
"#
    );
    let Some(clang) = require_clang("call_arguments_evaluate_left_to_right") else {
        return;
    };
    let dir = test_dir("call-args");
    let c_path = generated_c(&source, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("call-args-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "500742\n",
            "{label} evaluated call arguments in the wrong order"
        );
    }
}

#[test]
fn printf_arguments_evaluate_left_to_right() {
    let source = format!(
        r#"{SIDE_EFFECT_PREFIX}

open flow main
    => i32
{{
    printf("%d%d\n", first(), second())
    => 0
}}
"#
    );
    let Some(clang) = require_clang("printf_arguments_evaluate_left_to_right") else {
        return;
    };
    let dir = test_dir("printf-args");
    let c_path = generated_c(&source, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("printf-args-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "5007240\n",
            "{label} evaluated printf arguments in the wrong order"
        );
    }
}

#[test]
fn logical_operators_keep_short_circuiting() {
    let source = r#"
flow yes
    => bool
{
    printf("A")
    => true
}

flow no
    => bool
{
    printf("B")
    => false
}

open flow main
    => i32
{
    if no() and yes() {
        printf("X")
    }
    if yes() or no() {
        printf("Y")
    }
    printf("\n")
    => 0
}
"#;
    let Some(clang) = require_clang("logical_operators_keep_short_circuiting") else {
        return;
    };
    let dir = test_dir("short-circuit");
    let c_path = generated_c(source, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("short-circuit-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "BAY\n",
            "{label} must not evaluate a short-circuited right operand"
        );
    }
}

#[derive(Clone)]
enum Node {
    Int(i32),
    Event(usize),
    Add(Box<Node>, Box<Node>),
    Sub(Box<Node>, Box<Node>),
}

impl Node {
    fn code(&self) -> String {
        match self {
            Node::Int(value) => value.to_string(),
            Node::Event(index) => format!("e{index}()"),
            Node::Add(left, right) => format!("({} + {})", left.code(), right.code()),
            Node::Sub(left, right) => format!("({} - {})", left.code(), right.code()),
        }
    }

    fn events(&self, output: &mut Vec<usize>) {
        match self {
            Node::Int(_) => {}
            Node::Event(index) => output.push(*index),
            Node::Add(left, right) | Node::Sub(left, right) => {
                left.events(output);
                right.events(output);
            }
        }
    }

    fn eval(&self, values: &[i32]) -> i32 {
        match self {
            Node::Int(value) => *value,
            Node::Event(index) => values[*index],
            Node::Add(left, right) => left.eval(values).wrapping_add(right.eval(values)),
            Node::Sub(left, right) => left.eval(values).wrapping_sub(right.eval(values)),
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn make_program(seed: u64) -> (String, String) {
    let mut rng = Rng(seed | 1);
    let event_count = 3 + rng.below(4) as usize;
    let mut values = vec![0; event_count + 1];
    for value in values.iter_mut().skip(1) {
        *value = 1 + rng.below(9) as i32;
    }

    let mut leaves = Vec::new();
    for index in 1..=event_count {
        leaves.push(Node::Event(index));
    }
    for _ in 0..event_count {
        leaves.push(Node::Int(1 + rng.below(9) as i32));
    }
    for index in (1..leaves.len()).rev() {
        let swap = rng.below((index + 1) as u64) as usize;
        leaves.swap(index, swap);
    }
    while leaves.len() > 1 {
        let right = leaves.pop().expect("leaf should exist");
        let left = leaves.pop().expect("leaf should exist");
        let combined = if rng.below(2) == 0 {
            Node::Add(Box::new(left), Box::new(right))
        } else {
            Node::Sub(Box::new(left), Box::new(right))
        };
        leaves.push(combined);
    }
    let expression = leaves.pop().expect("expression should exist");

    let mut source = String::new();
    for (index, value) in values.iter().enumerate() {
        source.push_str(&format!(
            "flow e{index}\n    => i32\n{{\n    printf(\"{index:02}\")\n    => {value}\n}}\n\n"
        ));
    }
    source.push_str(&format!(
        "open flow main\n    => i32\n{{\n    protect {{\n        mut xs: [i32]\n        alloc(-> xs, i32, 1)\n        xs[e0() as u64] = {}\n        value: i32 <- xs[0]\n        printf(\"%d\\n\", value)\n    }}\n    => 0\n}}\n",
        expression.code()
    ));

    let mut expected = "00".to_string();
    let mut events = Vec::new();
    expression.events(&mut events);
    for event in events {
        expected.push_str(&format!("{event:02}"));
    }
    expected.push_str(&format!("{}\n", expression.eval(&values)));
    (source, expected)
}

#[test]
fn three_hundred_generated_programs_match_the_left_to_right_oracle() {
    let Some(compiler) =
        require_clang("three_hundred_generated_programs_match_the_left_to_right_oracle")
    else {
        return;
    };
    let dir = test_dir("differential");
    for seed in 1..=320 {
        let (source, expected) = make_program(seed);
        let case_dir = dir.join(format!("case-{seed}"));
        fs::create_dir_all(&case_dir).expect("case directory should be creatable");
        let c_path = generated_c(&source, &case_dir);
        let exe = case_dir.join("program");
        let actual = compile_and_run(&compiler, &c_path, &exe);
        assert_eq!(
            actual, expected,
            "differential case {seed} did not match the Rust oracle\nsource:\n{source}"
        );
    }
}

#[test]
fn len_keeps_logical_short_circuiting() {
    let Some(clang) = require_clang("len_keeps_logical_short_circuiting") else {
        return;
    };
    let source = r#"
flow no
    => bool
{
    printf("L")
    => false
}

flow touched
    => [u8]
{
    printf("R")
    => "abc"
}

open flow main
    => i32
{
    if no() and (len(touched()) > 0) {
        printf("X")
    }
    printf("\n")
    => 0
}
"#;
    let dir = test_dir("len-short-circuit");
    let c_path = generated_c(source, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("len-short-circuit-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "L\n",
            "{label} evaluated a short-circuited len() operand"
        );
    }
}

#[test]
fn len_preserves_left_to_right_operand_order() {
    let Some(clang) = require_clang("len_preserves_left_to_right_operand_order") else {
        return;
    };
    let source = r#"
flow first
    => i32
{
    printf("L")
    => 2
}

flow touched
    => [u8]
{
    printf("R")
    => "abc"
}

open flow main
    => i32
{
    total <- first() + (len(touched()) as i32)
    printf("\n")
    => 0
}
"#;
    let dir = test_dir("len-order");
    let c_path = generated_c(source, &dir);
    for (label, compiler) in [("gcc", "cc"), ("clang", clang.as_str())] {
        let exe = dir.join(format!("len-order-{label}"));
        assert_eq!(
            compile_and_run(compiler, &c_path, &exe),
            "LR\n",
            "{label} evaluated len() before the earlier operand"
        );
    }
}
