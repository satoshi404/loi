//! Deterministic property checks without external dependencies.
//!
//! These are deliberately small: they prove properties that must hold for
//! *every* input, not specific language features. Fuzzing targets for each
//! stage live in `tests/fuzz_smoke.rs` and `tests/fuzz_stages.rs`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use loi::{check, const_eval, format_program, parse, tokenize, tokenize_with_comments};

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

const VALID: &[&str] = &[
    "flow main\n    => i32\n{\n    => 0\n}\n",
    "shape Vec2 { x: i32, y: i32 }\nflow sum\n    <- v: Vec2\n    => i32\n{\n    => v.x + v.y\n}\n",
    "mark Color { Red, Green }\nchoice R { Ok: i32, Err: [u8] }\n",
    "flow f\n    <- xs: [i32]\n    => i32\n{\n    => xs.len as i32\n}\n",
    "shape Box<T> { value: T }\nflow unbox\n    <- b: Box<i32>\n    => i32\n{\n    => b.value\n}\n",
    "flow main\n    => i32\n{\n    if true { => 1 }\n    => 0\n}\n",
    "comptime flow three\n    => i32\n{\n    => 1 + 2\n}\n",
];

const INVALID: &[&str] = &[
    "flow main => i32 { => }",
    "shape Vec2 { x: i32, y: i32",
    "\"unterminated",
    "0x 0b2",
    "flow main => i32 { mut x: i32 <- ; => 0 }",
    "flow f(<- a: i32, <- a: i32) => i32 { => a }",
    "flow main => i32 { => 1 + }",
];

fn token_fingerprint(source: &str) -> String {
    format!("{:?}", tokenize(source, "<property>").map(|_| ()))
}

#[test]
fn tokenization_is_deterministic() {
    for source in VALID.iter().chain(INVALID) {
        assert_eq!(
            token_fingerprint(source),
            token_fingerprint(source),
            "tokenization changed between runs for {source:?}"
        );
        let first = format!("{:?}", tokenize(source, "<property>"));
        let second = format!("{:?}", tokenize(source, "<property>"));
        assert_eq!(first, second);
    }
}

#[test]
fn token_and_error_spans_stay_inside_the_source() {
    for source in VALID.iter().chain(INVALID) {
        let lines: Vec<&str> = source.split('\n').collect();
        let line_count = lines.len().max(1);
        if let Ok((tokens, _comments)) = tokenize_with_comments(source, "<property>") {
            for token in tokens {
                assert!(
                    token.line >= 1 && token.line <= line_count,
                    "bad token line"
                );
                let width = lines[token.line - 1].chars().count() + 1;
                assert!(
                    token.column >= 1 && token.column <= width,
                    "bad token column"
                );
            }
        } else if let Err(error) = tokenize_with_comments(source, "<property>") {
            assert!(
                error.line >= 1 && error.line <= line_count,
                "bad error line"
            );
            let width = lines[error.line - 1].chars().count() + 1;
            assert!(
                error.column >= 1 && error.column <= width,
                "bad error column"
            );
            assert!(
                error.end_column >= error.column && error.end_column <= width + 1,
                "bad error end column"
            );
        }
    }
}

#[test]
fn formatting_twice_is_idempotent() {
    for source in VALID {
        let tokens = tokenize(source, "<property>").expect("valid fixture should lex");
        let program = parse(tokens).expect("valid fixture should parse");
        let once = format_program(&program);
        let tokens = tokenize(&once, "<property>").expect("formatted source should lex");
        let program = parse(tokens).expect("formatted source should parse");
        let twice = format_program(&program);
        assert_eq!(once, twice, "formatting is not idempotent for {source:?}");
    }
}

#[test]
fn mutated_valid_programs_never_panic_in_lexer_or_parser() {
    let alphabet = b" \n{}()<>[],;:+-*/\"'0123456789abcxyz_";
    let mut rng = Rng(0x5eed);
    for source in VALID {
        for _ in 0..32 {
            let bytes = source.as_bytes();
            let mut mutant = bytes.to_vec();
            let edits = 1 + rng.below(3) as usize;
            for _ in 0..edits {
                if mutant.is_empty() {
                    break;
                }
                let index = rng.below(mutant.len() as u64) as usize;
                match rng.below(3) {
                    0 => mutant[index] = alphabet[rng.below(alphabet.len() as u64) as usize],
                    1 => {
                        mutant.remove(index);
                    }
                    _ => mutant.insert(index, alphabet[rng.below(alphabet.len() as u64) as usize]),
                }
            }
            let mutant = String::from_utf8_lossy(&mutant).into_owned();
            let result = std::panic::catch_unwind(|| {
                let Ok(tokens) = tokenize(&mutant, "<property>") else {
                    return;
                };
                let _ = parse(tokens);
            });
            assert!(result.is_ok(), "panic in lexer/parser for {mutant:?}");
        }
    }
}

#[test]
fn valid_give_then_use_is_rejected_for_every_field_path() {
    for (field, use_path) in [("a", "a"), ("nested", "nested.x")] {
        let source = format!(
            "shape Q {{ x: [u8] }}\nshape P {{ a: [u8], nested: Q }}\nflow f\n    => i32\n{{\n    mut p: P <- P {{ a: [u8: 1], nested: Q {{ x: [u8: 2] }} }}\n    give(p.{field})\n    => p.{use_path}.len as i32\n}}\n"
        );
        let tokens = tokenize(&source, "<property>").expect("fixture should lex");
        let program = parse(tokens).expect("fixture should parse");
        let errors = check(&program);
        assert!(
            !errors.is_empty(),
            "use after give({field}) was accepted: {errors:#?}"
        );
    }
}

#[test]
fn constant_evaluation_matches_runtime_for_supported_arithmetic() {
    let source = "answer: i32 <- 2 + 3 * 4\nopen flow main\n    => i32\n{\n    => answer\n}\n";
    let tokens = tokenize(source, "<property>").expect("fixture should lex");
    let program = parse(tokens).expect("fixture should parse");
    let global = program
        .items
        .iter()
        .find_map(|item| match item {
            loi::Item::Global(decl) => Some(decl),
            _ => None,
        })
        .expect("fixture has a global");
    let value = const_eval::ConstEvaluator::new(&program)
        .eval_global(global)
        .expect("constant should evaluate")
        .as_int()
        .expect("constant should be integer");
    assert_eq!(value, 14);

    let dir = std::env::temp_dir().join(format!("loi-property-const-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("main.loi");
    let exe = dir.join("program");
    fs::write(&path, source).expect("fixture should be writable");
    let mut build = Command::new(env!("CARGO_BIN_EXE_loi"));
    build.arg("build").arg(&path).arg("-o").arg(&exe);
    let built = build.output().expect("loi build should run");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = Command::new(&exe).output().expect("program should run");
    assert_eq!(run.status.code(), Some(14));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn paths_with_spaces_and_unicode_are_supported() {
    let root = std::env::temp_dir().join(format!(
        "loi property spaces ünicode {}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp dir with spaces and unicode");
    let path = root.join("program with spaces.loi");
    fs::write(&path, "flow main\n    => i32\n{\n    => 7\n}\n")
        .expect("fixture should be writable");
    let output = Command::new(env!("CARGO_BIN_EXE_loi"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("loi check should run");
    assert!(
        output.status.success(),
        "portable path failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&root);
}

#[allow(dead_code)]
fn unused_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
