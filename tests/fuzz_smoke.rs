use loi::{expand, lexer, parser};

const LEXER_CORPUS: &[&str] = &[
    "",
    "flow main => i32 { => 0 }",
    "0x 0b 0xG 0b2",
    "\"unterminated",
    "\"bad\\q\"",
    "\"ok\\n\\t\\0\\\"\"",
    "// comment\nflow f => i32 { => 1 }",
    "/* nested /* comment */ */",
    "'a' '\\n' '\\x'",
    "1_000 3.14 1e9 0xFF_FF",
];

const PARSE_CORPUS: &[&str] = &[
    "flow main => i32 { => 0 }",
    "shape Box<T> { value: T } flow f => i32 { => 0 }",
    "flow f => i32 { if true { => 1 } else { => 2 } }",
    "choice R { Ok(i32), Err([u8]) }",
    "mark M { A, B }",
    "comptime flow f => i32 { => 1 }",
    "flow f => i32 { mut x: i32 <- 1; x += 2; => x }",
    "import \"missing.loi\" flow main => i32 { => 0 }",
    "guard x <- f() { g(<-> x) }",
    "flow f(a: callback(i32) => i32) => i32 { => a(1) }",
    "flow f => i32 { for i in 0..10 { continue } => 0 }",
    "<<< >>> not loi source at all",
];

#[test]
fn lexer_smoke_never_panics() {
    for (i, source) in LEXER_CORPUS.iter().enumerate() {
        let result = std::panic::catch_unwind(|| lexer::tokenize(source, &format!("<fuzz-{i}>")));
        assert!(result.is_ok(), "lexer panicked on corpus entry {i}");
    }
}

#[test]
fn parser_smoke_never_panics() {
    for (i, source) in PARSE_CORPUS.iter().enumerate() {
        let result = std::panic::catch_unwind(|| {
            let tokens = match lexer::tokenize(source, &format!("<fuzz-{i}>")) {
                Ok(t) => t,
                Err(_) => return,
            };
            let mut parser = parser::Parser::new(tokens);
            let _ = parser.parse_program();
        });
        assert!(result.is_ok(), "parser panicked on corpus entry {i}");
    }
}

#[test]
fn checker_smoke_never_panics() {
    for (i, source) in PARSE_CORPUS.iter().enumerate() {
        let result = std::panic::catch_unwind(|| {
            let tokens = match lexer::tokenize(source, &format!("<fuzz-{i}>")) {
                Ok(t) => t,
                Err(_) => return,
            };
            let mut parser = parser::Parser::new(tokens);
            let program = match parser.parse_program() {
                Ok(p) => p,
                Err(_) => return,
            };
            let expanded = match expand::expand_program(&program) {
                Ok(p) => p,
                Err(_) => return,
            };
            let _ = loi::checker::check(&expanded);
        });
        assert!(result.is_ok(), "checker panicked on corpus entry {i}");
    }
}
