use loi::{format_program, format_program_with_comments, parse, tokenize, tokenize_with_comments};

fn parse_source(source: &str) -> loi::Program {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    parse(tokens).expect("source should parse")
}

/// Formatting must be idempotent even though a re-parse produces fresh spans.
fn assert_format_stable(source: &str) -> String {
    let formatted = format_program(&parse_source(source));
    let reformatted = format_program(&parse_source(&formatted));
    assert_eq!(formatted, reformatted);
    formatted
}

#[test]
fn reference_program_round_trips() {
    let formatted = assert_format_stable(include_str!("../examples/particles.loi"));
    assert!(formatted.contains("open flow world_new"));
    assert!(formatted.contains("shape Particle ["));
    assert!(formatted.contains("w.particles = give(ps)"));
}

#[test]
fn canonical_flow_syntax_round_trips_unchanged() {
    let source = "open flow add\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    mut total: i32 <- a + b\n    => total\n}\n";
    assert_eq!(assert_format_stable(source), source);
}

#[test]
fn formats_a_small_declaration_stably() {
    let program = parse_source(
        "open flow add\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    => a + b\n}\n",
    );

    assert_eq!(
        format_program(&program),
        "open flow add\n    <- a: i32\n    <- b: i32\n    => i32\n{\n    => a + b\n}\n"
    );
}

#[test]
fn preserves_operator_precedence_and_grouping() {
    let grouped = assert_format_stable("flow f {\n    => (1 + 2) * 3\n}\n");
    assert!(grouped.contains("=> (1 + 2) * 3"));

    let ungrouped = assert_format_stable("flow f {\n    => 1 + 2 * 3\n}\n");
    assert!(ungrouped.contains("=> 1 + 2 * 3"));

    let cast =
        assert_format_stable("flow f\n    <- i: u32\n    => f64\n{\n    => (i + 1) as f64\n}\n");
    assert!(cast.contains("=> (i + 1) as f64"));
}

#[test]
fn bracket_literals_need_no_parentheses_in_headers() {
    let formatted =
        assert_format_stable("flow f {\n    if Vec2[ x: 1, y: 2 ].x > 0 {\n        =>\n    }\n}\n");

    assert!(formatted.contains("if Vec2[ x: 1, y: 2 ].x > 0 {"));
}

#[test]
fn formats_alloc_give_clone_and_argument_directions() {
    let formatted = assert_format_stable(
        "flow f\n    <-> w: World\n{\n    alloc(-> xs, i32, 3)\n    w.particles = give(xs)\n    w.bounds = clone(size)\n    step(<-> w, 1.0 / 60)\n}\n",
    );

    assert!(formatted.contains("alloc(-> xs, i32, 3)"));
    assert!(formatted.contains("w.particles = give(xs)"));
    assert!(formatted.contains("w.bounds = clone(size)"));
    assert!(formatted.contains("step(<-> w, 1.0 / 60)"));
}

#[test]
fn formats_opaque_and_extern_flows_stably() {
    let source = r#"opaque Handle

extern flow make
    <- x: i32
    => Handle

extern flow renamed as c_renamed
    <- text: cstr
    => i32
"#;
    let formatted = assert_format_stable(source);
    assert_eq!(formatted, source);
}

#[test]
fn formats_imports_globals_strings_and_slice_literals() {
    let formatted = assert_format_stable(
        "import \"lib/std.loi\"\n\
         GREETING: [u8] <- \"hi\\n\\x01\"\n\
         mut LIMIT: i32 <- 3\n\
         flow f\n\
             => [i32]\n\
         {\n\
             inferred <- [1, 2]\n\
             => [i32: 3, 4]\n\
         }\n",
    );

    assert!(formatted.contains("import \"lib/std.loi\""));
    assert!(formatted.contains("GREETING: [u8] <- \"hi\\n\\x01\""));
    assert!(formatted.contains("mut LIMIT: i32 <- 3"));
    assert!(formatted.contains("inferred <- [1, 2]"));
    assert!(formatted.contains("=> [i32: 3, 4]"));
}

fn format_with_comments(source: &str) -> String {
    let (tokens, comments) = tokenize_with_comments(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    format_program_with_comments(&program, &comments)
}

#[test]
fn preserves_comments_and_formatting_is_idempotent() {
    let source = "\
// header\n\
// second line\n\
\n\
shape Point [ // shape open\n\
    // before x\n\
    x: f64 // x trailing\n\
    y: f64\n\
    // before close\n\
] // shape close\n\
\n\
flow main // header trailing\n\
    => i32\n\
{\n\
    // before let\n\
    mut x: i32 <- 1 // let trailing\n\
    if x > 0 { // if open\n\
        x += 1\n\
    } else { // else open\n\
        x = 0\n\
    } // if close\n\
    while x < 3 {\n\
        x += 1\n\
    }\n\
    protect {\n\
        mut items: [i32]\n\
        alloc(-> items, i32, 1)\n\
        // before protect close\n\
    }\n\
    // before return\n\
    => 0 // return trailing\n\
}\n\
// end\n";

    let formatted = format_with_comments(source);
    for comment in [
        "// header",
        "// second line",
        "// shape open",
        "// before x",
        "// x trailing",
        "// before close",
        "// shape close",
        "// header trailing",
        "// before let",
        "// let trailing",
        "// if open",
        "// else open",
        "// if close",
        "// before protect close",
        "// before return",
        "// return trailing",
        "// end",
    ] {
        assert!(
            formatted.contains(comment),
            "missing {comment:?} in:\n{formatted}"
        );
    }

    // Trailing comments stay on their statement lines.
    assert!(formatted.contains("x: f64 // x trailing"));
    assert!(formatted.contains("mut x: i32 <- 1 // let trailing"));
    assert!(formatted.contains("=> 0 // return trailing"));

    let reformatted = format_with_comments(&formatted);
    assert_eq!(
        formatted, reformatted,
        "comment formatting must be idempotent"
    );
}

#[test]
fn formatting_without_comments_still_discards_them() {
    let formatted = format_program(&parse_source("// gone\nflow main {\n    => 0\n}\n"));
    assert!(!formatted.contains("// gone"));
}
