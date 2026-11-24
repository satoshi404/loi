//! Separate fuzz-smoke targets for stages not covered by `fuzz_smoke.rs`.
//! Deterministic and dependency-free; each stage has its own test so a panic
//! identifies the responsible phase.

use std::fs;
use std::panic::catch_unwind;
use std::process::Command;

use loi::{check, expand_program, format_program, parse, tokenize};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("loi-fuzz-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[test]
fn formatter_never_panics_on_mutated_valid_programs() {
    let seeds = [
        "flow main\n    => i32\n{\n    => 0\n}\n",
        "shape P { a: i32, b: [u8] }\nflow f <- p: P => i32 { => p.a }\n",
        "mark M { A, B }\nchoice R { Ok(i32), Err }\n",
        "comptime flow c => i32 { quote { => 1 } }\n",
    ];
    let mut rng = Rng(0xf0a7);
    for seed in seeds {
        for _ in 0..24 {
            let mut source = seed.as_bytes().to_vec();
            let index = (rng.next() as usize) % source.len().max(1);
            if index < source.len() {
                source[index] = b" {}[](),;:<>=-+\n\""[(rng.next() as usize) % 17];
            }
            let source = String::from_utf8_lossy(&source).into_owned();
            let result = catch_unwind(|| {
                let Ok(tokens) = tokenize(&source, "<fuzz-fmt>") else {
                    return;
                };
                let Ok(program) = parse(tokens) else {
                    return;
                };
                let _ = format_program(&program);
            });
            assert!(result.is_ok(), "formatter panicked for {source:?}");
        }
    }
}

#[test]
fn module_and_import_stage_never_panics() {
    let dir = temp_dir("modules");
    let names = ["a", "b", "c", "d"];
    for (index, name) in names.iter().enumerate() {
        let next = names[(index + 1) % names.len()];
        let source = format!(
            "import \"{}\"\nopen flow f{index}\n    => i32\n{{\n    => {index}\n}}\n",
            dir.join(format!("{next}.loi")).display()
        );
        fs::write(dir.join(format!("{name}.loi")), source).expect("module fixture");
    }
    let result = catch_unwind(|| {
        for name in names {
            let output = Command::new(env!("CARGO_BIN_EXE_loi"))
                .arg("check")
                .arg(dir.join(format!("{name}.loi")))
                .output()
                .expect("loi check should run");
            assert!(!output.status.success(), "cyclic imports must be diagnosed");
            assert!(!output.stderr.is_empty(), "cycle must produce a diagnostic");
        }
    });
    let _ = fs::remove_dir_all(&dir);
    assert!(result.is_ok(), "module stage panicked");
}

#[test]
fn template_expansion_stage_never_panics() {
    let seeds = [
        "comptime flow make\n    => code\n{\n    quote { => 1 }\n}\nflow main\n    => i32\n{\n    splice make()\n}\n",
        "flow main\n    => i32\n{\n    quote { => 1 }\n}\n",
        "global g: i32 <- quote { => 1 }\n",
        "comptime flow make\n    => code\n{\n    quote { unquote(1) }\n}\n",
    ];
    for source in seeds {
        let result = catch_unwind(|| {
            let Ok(tokens) = tokenize(source, "<fuzz-template>") else {
                return;
            };
            let Ok(program) = parse(tokens) else {
                return;
            };
            let _ = expand_program(&program);
        });
        assert!(result.is_ok(), "template expansion panicked for {source:?}");
    }
}

#[test]
fn constant_evaluator_never_panics_on_mutated_globals() {
    let expressions = [
        "1 + 2 * 3",
        "-9223372036854775808",
        "1 / 0",
        "not true",
        "1 << 200",
        "\"abc\".len",
        "0x",
        "0b2",
    ];
    for expression in expressions {
        let source = format!("global g: i32 <- {expression}\n");
        let result = catch_unwind(|| {
            let Ok(tokens) = tokenize(&source, "<fuzz-const>") else {
                return;
            };
            let Ok(program) = parse(tokens) else {
                return;
            };
            let _ = check(&program);
        });
        assert!(
            result.is_ok(),
            "const evaluator panicked for {expression:?}"
        );
    }
}
