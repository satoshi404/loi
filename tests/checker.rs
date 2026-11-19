use loi::{check, parse, tokenize};

fn diagnostics(source: &str) -> Vec<loi::Diagnostic> {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    check(&program)
}

fn messages(source: &str) -> Vec<String> {
    diagnostics(source)
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn assert_clean(source: &str) {
    let diagnostics = diagnostics(source);
    assert!(
        diagnostics.is_empty(),
        "expected no diagnostics, got {diagnostics:#?}"
    );
}

fn assert_message(source: &str, expected: &str) {
    let messages = messages(source);
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "expected a diagnostic containing {expected:?}, got {messages:#?}"
    );
}

#[test]
fn reference_program_type_checks() {
    assert_clean(include_str!("../examples/particles.loi"));
}

#[test]
fn tictactoe_example_type_checks() {
    assert_clean(include_str!("../examples/tictactoe.loi"));
}

#[test]
fn systems_example_type_checks() {
    assert_clean(include_str!("../examples/systems.loi"));
}

#[test]
fn marks_example_type_checks() {
    assert_clean(include_str!("../examples/marks.loi"));
}

#[test]
fn choices_example_type_checks() {
    assert_clean(include_str!("../examples/choices.loi"));
}

#[test]
fn unsuffixed_literals_coerce_to_numeric_types() {
    assert_clean(
        "flow whole\n    => f64\n{\n    => 1\n}\n\nflow add\n    <- x: u32\n    => u32\n{\n    => x + 1\n}\n\nflow div\n    => f64\n{\n    => 1.0 / 60\n}\n",
    );
}

#[test]
fn rejects_integer_literals_out_of_range() {
    assert_message(
        "flow f\n    => u8\n{\n    => 256\n}\n",
        "out of range for u8",
    );
    assert_message("flow f {\n    x: i8 <- -129\n}\n", "out of range for i8");
    assert_message(
        "flow f\n    <- x: u8\n{\n}\n\nflow main {\n    f(300)\n}\n",
        "out of range for u8",
    );
    assert_message("shape I [\n    x: u16 = 70000\n]\n", "out of range for u16");
    assert_message(
        "flow f {\n    mut x: u8 <- 0\n    x = 300\n}\n",
        "out of range for u8",
    );
    assert_message("flow f {\n    x <- 5000000000\n}\n", "out of range for i32");
    assert_message(
        "flow f {\n    x <- 340282366920938463463374607431768211456\n}\n",
        "out of range for i32",
    );
    assert_message(
        "flow f\n    <- n: u64\n{\n    if n > -1 {\n    }\n}\n",
        "out of range for u64",
    );
    assert_message(
        "flow f\n    <- n: u8\n    => u8\n{\n    => n + 300\n}\n",
        "out of range for u8",
    );
}

#[test]
fn accepts_boundary_integer_literals() {
    assert_clean(
        "flow a\n    => u8\n{\n    => 255\n}\n\nflow b\n    => i8\n{\n    => -128\n}\n\nflow c\n    => u64\n{\n    => 18446744073709551615\n}\n\nflow d\n    => i32\n{\n    => -2147483648\n}\n\nflow e\n    <- n: u8\n    => bool\n{\n    => n == 255\n}\n\nflow g\n    <- n: u64\n    => bool\n{\n    => n > 0\n}\n",
    );
}

#[test]
fn rejects_negative_range_start_with_unsigned_bound() {
    assert_message(
        "flow f\n    <- n: u64\n{\n    for i in -1..n {\n    }\n}\n",
        "out of range for u64",
    );
}

#[test]
fn rejects_unknown_types() {
    assert_message(
        "shape Vec2 [\n    x: Missing\n]\n",
        "unknown type 'Missing'",
    );
    assert_message(
        "flow f\n    <- x: Nope\n    => Nada\n{\n    => x\n}\n",
        "unknown type 'Nope'",
    );
}

#[test]
fn checks_parameter_directions_at_call_sites() {
    assert_message(
        "shape World [\n    tick: u64\n]\n\nflow init\n    -> w: World\n{\n    w.tick = 0\n}\n\nflow main {\n    mut w: World\n    init(w)\n}\n",
        "expects direction '->', found '<-'",
    );

    assert_message(
        "shape World [\n    tick: u64\n]\n\nflow step\n    <-> w: World\n{\n    w.tick += 1\n}\n\nflow main {\n    mut w: World\n    step(<-> w)\n    step(w)\n}\n",
        "expects direction '<->', found '<-'",
    );
}

#[test]
fn rejects_bad_member_and_index_access() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <- v: Vec2\n    => f64\n{\n    => v.nope\n}\n",
        "shape 'Vec2' has no field 'nope'",
    );

    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    => x[0]\n}\n",
        "cannot index type i32",
    );

    assert_message(
        "flow f\n    <- xs: [i32]\n    => i32\n{\n    => xs[true]\n}\n",
        "index must be an integer",
    );
}

#[test]
fn rejects_interface_value_cycles() {
    assert_message(
        "shape A [\n    b: B\n]\n\nshape B [\n    a: A\n]\n",
        "contains itself by value through a cycle",
    );
    assert_message(
        "shape A [\n    a: A\n]\n",
        "contains itself by value through a cycle",
    );
    assert_message(
        "shape A [\n    b: B\n]\n\nshape B [\n    c: C\n]\n\nshape C [\n    a: A\n]\n",
        "contains itself by value through a cycle",
    );
    // Slice fields do not create a value dependency.
    assert_clean("shape A [\n    b: [B]\n]\n\nshape B [\n    a: A\n]\n");
    // Acyclic chains are fine.
    assert_clean("shape A [\n    b: B\n]\n\nshape B [\n    value: i32\n]\n");
}

#[test]
fn accepts_builtin_intrinsics() {
    assert_clean(
        "flow use_sqrt\n    <- x: f64\n    => f64\n{\n    => sqrt(x)\n}\n\nflow use_print\n    <- buf: [u8]\n{\n    print(buf)\n}\n",
    );
}

#[test]
fn rejects_intrinsic_argument_type_mismatch() {
    // `sqrt(1)` is fine: literal `1` fits in f64.
    assert_clean("flow f\n    => f64\n{\n    => sqrt(1)\n}\n");
    assert_message(
        "flow f\n    => i32\n{\n    => sqrt(1.0)\n}\n",
        "returns f64, expected i32",
    );
    assert_message(
        "flow f\n    => i32\n{\n    => print(1)\n}\n",
        "expected [u8]",
    );
    // Too many arguments.
    assert_message(
        "flow f {\n    sqrt(1.0, 2.0)\n}\n",
        "expects 1 argument(s), found 2",
    );
}

#[test]
fn hex_and_binary_integer_literals() {
    // Hexadecimal literals in typed positions.
    assert_clean(
        "flow f\n    => u8\n{\n    => 0xFF\n}\n\nflow g\n    => u16\n{\n    => 0xABCD\n}\n\nflow h\n    => i32\n{\n    => 0x7FFFFFFF\n}\n",
    );
    assert_clean(
        "flow f\n    => u8\n{\n    => 0b11111111\n}\n\nflow g\n    => u16\n{\n    => 0b1010101010101010\n}\n\nflow h\n    => i32\n{\n    => 0b01111111111111111111111111111111\n}\n",
    );
    // Hex/binary with underscores.
    assert_clean(
        "flow f\n    => u32\n{\n    => 0xFF_FF\n}\n\nflow g\n    => u8\n{\n    => 0b1111_0000\n}\n",
    );
    // Hex/binary out of range.
    assert_message(
        "flow f\n    => u8\n{\n    => 0x100\n}\n",
        "out of range for u8",
    );
    assert_message(
        "flow f\n    => u8\n{\n    => 0b1_0000_0000\n}\n",
        "out of range for u8",
    );
    // Hex/binary in binary expressions with constant folding.
    assert_clean(
        "flow f\n    => u8\n{\n    => 0x80 + 0x7F\n}\n\nflow g\n    => u8\n{\n    => 0b10000000 - 1\n}\n\nflow h\n    => u8\n{\n    => 0x0A * 0x03\n}\n",
    );
    assert_message(
        "flow f\n    => u8\n{\n    => 0x80 + 0x80\n}\n",
        "out of range for u8",
    );
    // Hex/binary division by zero.
    assert_message(
        "flow f\n    => i32\n{\n    => 1 / 0x0\n}\n",
        "division by zero",
    );
}

#[test]
fn folds_constant_expressions_for_range_checks() {
    assert_message(
        "flow f {\n    x: u8 <- 200 + 100\n}\n",
        "out of range for u8",
    );
    assert_message("flow f {\n    x: u8 <- 512 / 2\n}\n", "out of range for u8");
    assert_message("flow f {\n    x: u8 <- 16 * 16\n}\n", "out of range for u8");
    assert_message("flow f {\n    x: u8 <- 0 - 1\n}\n", "out of range for u8");
    assert_message(
        "flow f {\n    x: i8 <- -128 - 1\n}\n",
        "out of range for i8",
    );
    assert_message(
        "flow f {\n    x: u8 <- (100 + 100) * 2\n}\n",
        "out of range for u8",
    );
    assert_message(
        "flow f\n    => u64\n{\n    => 18446744073709551615 + 1\n}\n",
        "out of range for u64",
    );

    assert_clean(
        "flow a {\n    x: u8 <- 127 + 128\n}\n\nflow b {\n    x: i8 <- -64 - 64\n}\n\nflow c {\n    x: u8 <- 1000 % 256\n}\n",
    );
}

#[test]
fn rejects_constant_division_by_zero() {
    assert_message(
        "flow f\n    => i32\n{\n    => 1 / 0\n}\n",
        "division by zero",
    );
    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    => x / (2 - 2)\n}\n",
        "division by zero",
    );
    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    => x % (1 - 1)\n}\n",
        "remainder by zero",
    );
}

#[test]
fn rejects_negative_constant_slice_indexes() {
    assert_message(
        "flow f\n    <- xs: [i32]\n    => i32\n{\n    => xs[-1]\n}\n",
        "slice index cannot be negative",
    );
    assert_message(
        "flow f\n    <- xs: [i32]\n{\n    xs[-42] = 0\n}\n",
        "slice index cannot be negative",
    );
    assert_message(
        "flow f\n    <- xs: [i32]\n    => i32\n{\n    => xs[0 - 1]\n}\n",
        "slice index cannot be negative",
    );
    // `-0` has the value zero and double negation is positive.
    assert_clean("flow f\n    <- xs: [i32]\n    => i32\n{\n    => xs[-0] + xs[--1]\n}\n");
}

#[test]
fn conditions_must_be_bool() {
    assert_message(
        "flow f\n    <- x: i32\n{\n    if x {\n        =>\n    }\n}\n",
        "condition must be bool",
    );
}

#[test]
fn ordering_operators_require_numeric_operands() {
    assert_message(
        "flow f\n    <- a: bool\n    <- b: bool\n{\n    if a < b {\n        =>\n    }\n}\n",
        "operator requires numeric operands, found bool and bool",
    );
}

#[test]
fn rejects_unknown_variables_and_functions() {
    assert_message(
        "flow f\n    => i32\n{\n    => missing\n}\n",
        "unknown variable 'missing'",
    );
    assert_message("flow f {\n    nope()\n}\n", "unknown function 'nope'");
}

#[test]
fn checks_interface_literal_fields() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    => Vec2\n{\n    => Vec2[ x: 0, z: 1 ]\n}\n",
        "unknown field 'z'",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    => Vec2\n{\n    => Vec2[ x: 0, z: 1 ]\n}\n",
        "missing field 'y'",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    => Vec2\n{\n    => Vec2[ x: 0, x: 1, y: 2 ]\n}\n",
        "duplicate field 'x'",
    );
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow f\n    => Particle\n{\n    => Particle[ mass: true ]\n}\n",
        "field 'mass' has type bool",
    );
}

#[test]
fn enforces_let_immutability_and_read_only_in_parameters() {
    assert_message(
        "flow f {\n    x: i32 <- 0\n    x = 1\n}\n",
        "cannot assign to immutable variable 'x'",
    );
    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    x = 1\n    => give(x)\n}\n",
        "cannot write to '<-' parameter 'x'",
    );
    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    x = 1\n    => give(x)\n}\n",
        "cannot give from '<-' parameter 'x'",
    );
}

#[test]
fn checks_alloc_targets_and_element_types() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow f {\n    mut xs: [i32]\n    alloc(-> xs, Particle, 3)\n}\n",
        "alloc element type Particle does not match slice element type i32",
    );
    assert_message(
        "flow f {\n    mut x: i32\n    alloc(-> x, i32, 1)\n}\n",
        "alloc target must be a slice",
    );
}

#[test]
fn rejects_copy_of_values_containing_slices() {
    assert_message(
        "shape World [\n    particles: [i32]\n]\n\nflow f\n    <- w: World\n    => World\n{\n    => clone(w)\n}\n",
        "cannot copy a value of type World because it contains a slice",
    );
}

#[test]
fn scalar_copies_are_implicit_and_copy_is_allowed() {
    assert_clean("flow f\n    <- a: i32\n    => i32\n{\n    mut b: i32 <- a\n    => clone(b)\n}\n");
}

#[test]
fn checks_return_types_and_loop_ranges() {
    assert_message(
        "flow f\n    => i32\n{\n    => true\n}\n",
        "function 'f' returns bool, expected i32",
    );
    assert_message(
        "flow f\n    => i32\n{\n    =>\n}\n",
        "function 'f' must return a value of type i32",
    );
    assert_message(
        "flow f {\n    for i in 0.5..2 {\n    }\n}\n",
        "range start must be an integer",
    );
}

#[test]
fn inout_iteration_requires_a_writable_slice() {
    assert_message(
        "shape World [\n    particles: [i32]\n]\n\nflow f\n    <- w: World\n{\n    for <-> p in w.particles {\n        p = 1\n    }\n}\n",
        "cannot write to '<-' parameter 'w'",
    );
}

#[test]
fn diagnostics_carry_source_positions() {
    let diagnostics = diagnostics("flow f\n    <- x: i32\n{\n    if x {\n        =>\n    }\n}\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].span.line, 4);
    assert_eq!(diagnostics[0].span.column, 8);
}

// ---------------------------------------------------------------------------
// Phase 2: ownership and definite assignment
// ---------------------------------------------------------------------------

#[test]
fn move_kills_the_source_variable() {
    assert_message(
        "flow f\n    => i32\n{\n    mut a: i32 <- 1\n    mut b: i32 <- give(a)\n    => a + b\n}\n",
        "use of moved value 'a'",
    );
    assert_message(
        "flow f\n    => i32\n{\n    mut a: i32 <- 1\n    mut b: i32 <- give(a)\n    mut c: i32 <- give(a)\n    => b + c\n}\n",
        "use of moved value 'a'",
    );
}

#[test]
fn move_is_path_sensitive() {
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nshape Particle [\n    pos: Vec2\n    mass: f64\n]\n\nflow f\n    <-> p: Particle\n    => f64\n{\n    mut v: Vec2 <- give(p.pos)\n    p.pos = Vec2[ x: 1, y: 2 ]\n    => p.mass + p.pos.x + v.x\n}\n",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nshape Particle [\n    pos: Vec2\n    mass: f64\n]\n\nflow f\n    <-> p: Particle\n    => f64\n{\n    mut v: Vec2 <- give(p.pos)\n    => p.pos.x + v.x\n}\n",
        "use of moved value 'p'",
    );
}

#[test]
fn partial_write_into_a_moved_value_is_rejected() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <-> v: Vec2\n    => f64\n{\n    mut w: Vec2 <- give(v)\n    v.x = 1\n    => w.x\n}\n",
        "cannot write to moved value 'v'",
    );
}

#[test]
fn whole_variable_reinitialization_after_move_is_allowed() {
    assert_clean(
        "flow f\n    => i32\n{\n    mut a: i32 <- 1\n    mut b: i32 <- give(a)\n    a = 2\n    => a + b\n}\n",
    );
}

#[test]
fn branch_merge_requires_definite_initialization() {
    assert_message(
        "flow f\n    <- c: bool\n    => i32\n{\n    mut x: i32\n    if c {\n        x = 1\n    }\n    => x\n}\n",
        "use of uninitialized variable 'x'",
    );
    assert_clean(
        "flow f\n    <- c: bool\n    => i32\n{\n    mut x: i32\n    if c {\n        x = 1\n    } else {\n        x = 2\n    }\n    => x\n}\n",
    );
    assert_clean(
        "flow f\n    <- c: bool\n    => i32\n{\n    mut x: i32\n    if c {\n        x = 1\n    } else if c {\n        x = 2\n    } else {\n        x = 3\n    }\n    => x\n}\n",
    );
    assert_clean(
        "flow f\n    <- c: bool\n    => i32\n{\n    mut x: i32\n    if c {\n        => 0\n    } else {\n        x = 1\n    }\n    => x\n}\n",
    );
    assert_clean(
        "flow f\n    <- c: bool\n    => i32\n{\n    mut a: i32 <- 1\n    if c {\n        mut b: i32 <- give(a)\n        => b\n    }\n    => a\n}\n",
    );
}

#[test]
fn loops_do_not_make_initialization_definite() {
    assert_message(
        "flow f\n    <- n: i32\n    => i32\n{\n    mut x: i32\n    for i in 0..n {\n        x = i\n    }\n    => x\n}\n",
        "use of uninitialized variable 'x'",
    );
    assert_message(
        "flow f\n    <- c: bool\n    => i32\n{\n    mut x: i32\n    while c {\n        x = 1\n    }\n    => x\n}\n",
        "use of uninitialized variable 'x'",
    );
    assert_clean(
        "flow f\n    <- n: i32\n    => i32\n{\n    mut x: i32 <- 0\n    for i in 0..n {\n        x = i\n    }\n    => x\n}\n",
    );
}

#[test]
fn exclusive_arguments_cannot_alias_other_arguments() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow swap\n    <-> a: Vec2\n    <-> b: Vec2\n{\n}\n\nflow f\n    <-> v: Vec2\n{\n    swap(<-> v, <-> v)\n}\n",
        "cannot pass 'v' as '<->'",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow both\n    -> a: Vec2\n    <- b: Vec2\n{\n}\n\nflow f\n    <-> v: Vec2\n{\n    both(-> v, v)\n}\n",
        "cannot pass 'v' as '->'",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow mix\n    <-> x: f64\n    <- y: f64\n{\n}\n\nflow f\n    <-> v: Vec2\n{\n    mix(<-> v.x, v.x)\n}\n",
        "cannot pass 'v' as '<->'",
    );
}

#[test]
fn duplicate_in_arguments_and_disjoint_fields_are_allowed() {
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow read\n    <- a: Vec2\n    <- b: Vec2\n{\n}\n\nflow f\n    <- v: Vec2\n{\n    read(v, v)\n}\n",
    );
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow mix\n    <-> x: f64\n    <- y: f64\n{\n}\n\nflow f\n    <-> v: Vec2\n{\n    mix(<-> v.x, v.y)\n}\n",
    );
}

#[test]
fn implicit_composite_copies_are_rejected() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <- v: Vec2\n    => Vec2\n{\n    mut w: Vec2 <- v\n    => w\n}\n",
        "cannot copy Vec2 implicitly",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <- v: Vec2\n    -> w: Vec2\n{\n    w = v\n}\n",
        "cannot copy Vec2 implicitly",
    );
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nshape Pair [\n    a: Vec2\n    b: Vec2\n]\n\nflow f\n    <- v: Vec2\n    => Pair\n{\n    => Pair[ a: v, b: Vec2[ x: 0, y: 0 ] ]\n}\n",
        "cannot copy Vec2 implicitly",
    );
}

#[test]
fn explicit_move_and_copy_allow_composite_transfers() {
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <- v: Vec2\n    => Vec2\n{\n    => clone(v)\n}\n",
    );
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    => Vec2\n{\n    mut v: Vec2 <- Vec2[ x: 1, y: 2 ]\n    => give(v)\n}\n",
    );
}

#[test]
fn returning_a_value_borrowed_from_an_in_parameter_is_rejected() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <- v: Vec2\n    => Vec2\n{\n    => v\n}\n",
        "cannot return a value borrowed from '<-' parameter 'v'",
    );
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow f\n    <- v: Vec2\n    => Vec2\n{\n    => clone(v)\n}\n",
    );
}

// ---------------------------------------------------------------------------
// Phase 2: definite return
// ---------------------------------------------------------------------------

#[test]
fn non_void_functions_must_return_on_all_paths() {
    assert_message(
        "flow f\n    <- c: bool\n    => i32\n{\n    if c {\n        => 1\n    }\n}\n",
        "function 'f' must return a value of type i32 on all paths",
    );
    assert_message(
        "flow f\n    <- c: bool\n    => i32\n{\n    if c {\n        => 1\n    } else {\n        mut x: i32 <- 2\n    }\n}\n",
        "function 'f' must return a value of type i32 on all paths",
    );
}

#[test]
fn all_branches_returning_satisfies_definite_return() {
    assert_clean(
        "flow f\n    <- c: bool\n    => i32\n{\n    if c {\n        => 1\n    } else {\n        => 2\n    }\n}\n",
    );
    assert_clean(
        "flow f\n    <- c: bool\n    => i32\n{\n    if c {\n        => 1\n    } else if c {\n        => 2\n    } else {\n        => 3\n    }\n}\n",
    );
    assert_clean("flow f\n    => i32\n{\n    => 1\n}\n");
}

#[test]
fn while_true_with_return_satisfies_definite_return() {
    // `while true` always executes, so a body that always returns satisfies
    // definite return.
    assert_clean("flow f\n    => i32\n{\n    while true {\n        => 1\n    }\n}\n");
    assert_clean(
        "flow f\n    => i32\n{\n    while true {\n        => 1\n    }\n    while true {\n        => 2\n    }\n}\n",
    );
    assert_clean("flow f {\n    while true {\n        =>\n    }\n}\n");
}

#[test]
fn while_true_without_return_is_rejected() {
    assert_message(
        "flow f\n    => i32\n{\n    while true {\n    }\n}\n",
        "function 'f' must return a value of type i32 on all paths",
    );
    assert_message(
        "flow f\n    <- c: bool\n    => i32\n{\n    while c {\n        => 1\n    }\n}\n",
        "function 'f' must return a value of type i32 on all paths",
    );
    assert_message(
        "flow f\n    => i32\n{\n    while false {\n        => 1\n    }\n}\n",
        "function 'f' must return a value of type i32 on all paths",
    );
}

#[test]
fn break_and_continue_inside_loops() {
    assert_clean("flow f\n    => i32\n{\n    while true {\n        break\n    }\n    => 0\n}\n");
    assert_clean(
        "flow f\n    => i32\n{\n    for i in 0..10 {\n        if i == 5 {\n            break\n        }\n    }\n    => 0\n}\n",
    );
    assert_clean(
        "flow f {\n    protect {\n        mut items: [i32]\n        alloc(-> items, i32, 5)\n        for <- item in items {\n            continue\n        }\n    }\n}\n",
    );
}

#[test]
fn break_outside_loop_is_rejected() {
    assert_message(
        "flow f {\n    break\n}\n",
        "'break' is only allowed inside a loop",
    );
    assert_message(
        "flow f {\n    continue\n}\n",
        "'continue' is only allowed inside a loop",
    );
}

#[test]
fn void_functions_may_fall_through() {
    assert_clean("flow f\n    <- c: bool\n{\n    if c {\n        =>\n    }\n}\n");
}

// ---------------------------------------------------------------------------
// Phase 2: cross-function ownership
// ---------------------------------------------------------------------------

#[test]
fn moving_from_an_inout_parameter_consumes_the_callers_value() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow take\n    <-> v: Vec2\n    => Vec2\n{\n    => give(v)\n}\n\nflow main {\n    mut v: Vec2 <- Vec2[ x: 1, y: 2 ]\n    mut w: Vec2 <- take(<-> v)\n    mut z: Vec2 <- give(v)\n}\n",
        "use of moved value 'v'",
    );
}

#[test]
fn reading_and_writing_an_inout_parameter_does_not_consume_it() {
    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow bump\n    <-> v: Vec2\n{\n    v.x += 1\n}\n\nflow main {\n    mut v: Vec2 <- Vec2[ x: 1, y: 2 ]\n    bump(<-> v)\n    mut w: Vec2 <- give(v)\n}\n",
    );
}

#[test]
fn parameter_consumption_propagates_through_calls() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow take\n    <-> v: Vec2\n    => Vec2\n{\n    => give(v)\n}\n\nflow relay\n    <-> v: Vec2\n    => Vec2\n{\n    => take(<-> v)\n}\n\nflow main {\n    mut v: Vec2 <- Vec2[ x: 1, y: 2 ]\n    mut w: Vec2 <- relay(<-> v)\n    mut z: Vec2 <- give(v)\n}\n",
        "use of moved value 'v'",
    );
}

#[test]
fn moving_from_an_out_parameter_consumes_the_callers_value() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow make_then_steal\n    -> v: Vec2\n    => Vec2\n{\n    v = Vec2[ x: 1, y: 2 ]\n    => give(v)\n}\n\nflow main {\n    mut v: Vec2\n    mut w: Vec2 <- make_then_steal(-> v)\n    mut z: Vec2 <- give(v)\n}\n",
        "use of moved value 'v'",
    );
}

#[test]
fn returning_a_composite_place_transfers_ownership() {
    assert_message(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow take\n    <-> v: Vec2\n    => Vec2\n{\n    => v\n}\n\nflow main {\n    mut v: Vec2 <- Vec2[ x: 1, y: 2 ]\n    mut w: Vec2 <- take(<-> v)\n    mut z: Vec2 <- give(v)\n}\n",
        "use of moved value 'v'",
    );

    assert_clean(
        "shape Vec2 [\n    x: f64\n    y: f64\n]\n\nflow make\n    => Vec2\n{\n    mut v: Vec2 <- Vec2[ x: 1, y: 2 ]\n    => v\n}\n\nflow main {\n    mut w: Vec2 <- make()\n}\n",
    );
}

#[test]
fn allocation_taints_follow_a_consumed_inout_argument_into_an_out_argument() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow forward\n    <-> src: [Particle]\n    -> dst: [Particle]\n{\n    dst = give(src)\n}\n\nflow main {\n    mut b: [Particle]\n    protect {\n        mut a: [Particle]\n        alloc(-> a, Particle, 1)\n        forward(<-> a, -> b)\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );

    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow forward\n    <-> src: [Particle]\n    -> dst: [Particle]\n{\n    dst = give(src)\n}\n\nflow main {\n    protect {\n        mut a: [Particle]\n        mut b: [Particle]\n        alloc(-> a, Particle, 1)\n        forward(<-> a, -> b)\n    }\n}\n",
    );
}

#[test]
fn allocation_taints_follow_a_consumed_inout_argument_into_a_composite_result() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow take\n    <-> src: [Particle]\n    => [Particle]\n{\n    => src\n}\n\nflow main {\n    mut b: [Particle]\n    protect {\n        mut a: [Particle]\n        alloc(-> a, Particle, 1)\n        b = take(<-> a)\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );

    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow take\n    <-> src: [Particle]\n    => [Particle]\n{\n    => src\n}\n\nflow main {\n    protect {\n        mut a: [Particle]\n        mut b: [Particle]\n        alloc(-> a, Particle, 1)\n        b = take(<-> a)\n    }\n}\n",
    );
}

// ---------------------------------------------------------------------------
// Block A1: partial moves out of composites
// ---------------------------------------------------------------------------

const A1_PARTIAL_MOVE_SOURCE: &str = r#"shape Pair [
    left: [u64]
    right: u64
]

flow main
    => i32
{
    protect {
        mut p: Pair
        alloc(-> p.left, u64, 1)
        p.left[0] = 7
        p.right = 9
        mut taken: [u64]
        taken = give(p.left)
        print_int(taken[0])
        print_int(p.right)
        alloc(-> p.left, u64, 1)
        p.left[0] = 8
        print_int(p.left[0])
    }
    => 0
}
"#;

#[test]
fn partial_move_of_a_composite_field_leaves_siblings_alive() {
    assert_clean(A1_PARTIAL_MOVE_SOURCE);
}

#[test]
fn moved_composite_field_cannot_be_read_before_reinitialization() {
    let source = r#"shape Pair [
    left: [u64]
    right: u64
]

flow f
    <-> p: Pair
{
    mut taken: [u64] <- give(p.left)
    print_int(p.left.len)
    print_int(taken.len)
    alloc(-> p.left, u64, 1)
}
"#;
    assert_message(source, "use of moved value 'p'");
}

const A1_SWAP_SOURCE: &str = r#"flow swap
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

#[test]
fn giving_scalar_elements_with_runtime_indices_does_not_consume_the_slice() {
    assert_clean(A1_SWAP_SOURCE);
}

#[test]
fn giving_a_composite_element_with_a_runtime_index_poisons_unknown_elements() {
    let source = r#"shape Cell [
    data: [u64]
]

flow steal
    <-> cells: [Cell]
    <- i: u64
{
    mut out: [u64]
    out = give(cells[i].data)
    print_int(out.len)
}

flow main
    => i32
{
    protect {
        mut cells: [Cell]
        alloc(-> cells, Cell, 2)
        alloc(-> cells[0].data, u64, 1)
        alloc(-> cells[1].data, u64, 1)
        steal(<-> cells, 1)
        print_int(cells[0].data.len)
    }
    => 0
}
"#;
    let messages = messages(source);
    assert_eq!(
        messages.len(),
        1,
        "the runtime index should poison every element, got {messages:#?}"
    );
    assert!(messages[0].contains("use of moved value 'cells'"));
}

#[test]
fn giving_a_scalar_field_does_not_kill_the_owner() {
    let source = r#"shape Counter [
    label: [u8]
    value: u64
]

flow take_value
    <-> c: Counter
    => u64
{
    => give(c.value)
}

flow main
    => i32
{
    mut c <- Counter[ label: "x", value: 7 ]
    print_int(take_value(<-> c))
    print_int(c.value)
    => 0
}
"#;
    assert_clean(source);
}

#[test]
fn partial_consumption_through_a_call_only_kills_the_consumed_field() {
    let source = r#"shape Pair [
    left: [u64]
    right: [u64]
]

flow steal
    <-> p: Pair
    -> out: [u64]
{
    out = give(p.left)
}

flow main
    => i32
{
    protect {
        mut p: Pair
        alloc(-> p.left, u64, 1)
        alloc(-> p.right, u64, 1)
        p.right[0] = 5
        mut out: [u64]
        steal(<-> p, -> out)
        print_int(p.right[0])
        print_int(p.left[0])
    }
    => 0
}
"#;
    let messages = messages(source);
    assert_eq!(
        messages.len(),
        1,
        "only the consumed field should be dead, got {messages:#?}"
    );
    assert!(messages[0].contains("use of moved value 'p'"));
}

// ---------------------------------------------------------------------------
// Phase 2: protect regions and alloc escape
// ---------------------------------------------------------------------------

#[test]
fn alloc_inside_protect_is_allowed() {
    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow main {\n    protect {\n        mut ps: [Particle]\n        alloc(-> ps, Particle, 3)\n        ps[0].mass = 1.0\n    }\n}\n",
    );
}

#[test]
fn alloc_outside_protect_in_an_entry_function_is_rejected() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow main {\n    mut ps: [Particle]\n    alloc(-> ps, Particle, 3)\n}\n",
        "function 'main' can allocate outside a 'protect' block",
    );
}

#[test]
fn an_uncalled_hidden_allocator_may_rely_on_a_caller_protect() {
    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
    );
}

#[test]
fn an_uncalled_visible_allocator_may_rely_on_a_caller_protect() {
    // Exported library allocators are called from Loi under a caller's
    // `protect`. An uncalled flow cannot run without such a caller, so it is
    // not an entry point regardless of visibility.
    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nopen flow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
    );
}

#[test]
fn an_uncalled_allocator_with_default_visibility_is_not_an_entry() {
    // A function with no visibility keyword is `sealed` by default (spec
    // question 3), but visibility no longer decides entry status. An uncalled
    // allocator may rely on a caller's `protect`; `main` cannot.
    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
    );

    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nopen flow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
    );
}

#[test]
fn calling_an_open_allocator_without_a_protect_is_rejected() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nopen flow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n    mut ps: [Particle]\n    fill(-> ps)\n}\n",
        "function 'main' can allocate outside a 'protect' block",
    );
}

#[test]
fn protect_local_allocation_cannot_escape_to_an_outer_variable() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow main {\n    mut ps: [Particle]\n    protect {\n        alloc(-> ps, Particle, 3)\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );
}

#[test]
fn protect_local_allocation_cannot_be_returned() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow make\n    => [Particle]\n{\n    protect {\n        mut ps: [Particle]\n        alloc(-> ps, Particle, 3)\n        => ps\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );
}

#[test]
fn a_callee_may_allocate_under_the_callers_protect() {
    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow fill\n    -> ps: [Particle]\n    <- n: i32\n{\n    alloc(-> ps, Particle, n)\n}\n\nflow main {\n    protect {\n        mut ps: [Particle]\n        fill(-> ps, 3)\n        ps[0].mass = 1.0\n    }\n}\n",
    );
}

#[test]
fn calling_an_allocator_without_a_protect_is_rejected() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n    mut ps: [Particle]\n    fill(-> ps)\n}\n",
        "function 'main' can allocate outside a 'protect' block",
    );
}

#[test]
fn an_allocator_called_under_protect_cannot_write_to_outer_storage() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nflow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n    mut ps: [Particle]\n    protect {\n        fill(-> ps)\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );
}

#[test]
fn nested_protect_regions_cannot_escape_outward() {
    assert_message(
        "flow main {\n    protect {\n        mut outer: [i32]\n        alloc(-> outer, i32, 1)\n        protect {\n            mut inner: [i32]\n            alloc(-> inner, i32, 1)\n            outer = give(inner)\n        }\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );

    assert_clean(
        "flow main {\n    protect {\n        mut outer: [i32]\n        alloc(-> outer, i32, 1)\n        protect {\n            mut inner: [i32]\n            inner = give(outer)\n        }\n    }\n}\n",
    );
}

#[test]
fn len_and_member_len_are_u64() {
    assert_clean(
        "flow f\n    <- xs: [i32]\n    => u64\n{\n    => len(xs)\n}\n\nflow g\n    <- xs: [i32]\n    => u64\n{\n    => xs.len\n}\n\nflow h\n    <- text: [u8]\n    => u64\n{\n    => text.len\n}\n",
    );

    assert_message(
        "flow f\n    => u64\n{\n    => len(1)\n}\n",
        "len expects a slice",
    );
    assert_message(
        "flow f\n    => u64\n{\n    => len()\n}\n",
        "len expects 1 argument",
    );
    assert_message(
        "flow f\n    => u64\n{\n    => 1.len\n}\n",
        "cannot access field 'len' on type integer literal",
    );
}

#[test]
fn intrinsic_names_are_reserved() {
    assert_message(
        "flow len\n    <- xs: [i32]\n    => u64\n{\n    => 0\n}\n",
        "intrinsic 'len' cannot be redeclared",
    );
    assert_message(
        "flow print\n    <- text: [u8]\n{\n}\n",
        "intrinsic 'print' cannot be redeclared",
    );
}

#[test]
fn string_literals_are_byte_slices() {
    assert_clean(
        "flow f\n    => [u8]\n{\n    => \"hello\"\n}\n\nflow g\n    <- text: [u8]\n    => u64\n{\n    => text.len\n}\n",
    );
}

#[test]
fn slice_literals_infer_and_check_element_types() {
    assert_clean(
        "flow f\n    => i32\n{\n    explicit: [i32] <- [i32: 1, 2, 3]\n    inferred <- [10, 20, 30]\n    => explicit[0] + inferred[2]\n}\n",
    );

    assert_message("flow f {\n    xs <- [1, true]\n}\n", "incompatible types");
    assert_clean("flow f\n    => u64\n{\n    xs: [i32] <- [i32:]\n    => xs.len\n}\n");
    assert_message(
        "flow f {\n    xs: [i32] <- [u8: 1]\n}\n",
        "cannot initialize",
    );
}

#[test]
fn returning_a_slice_literal_is_rejected() {
    assert_message(
        "flow f\n    => [i32]\n{\n    => [1, 2]\n}\n",
        "cannot return a slice literal",
    );
}

#[test]
fn globals_are_checked_and_must_be_constant() {
    assert_clean(
        "LIMIT: i32 <- 3\n\nGREETING: [u8] <- \"hi\"\n\nmut counter: u64 <- 0\n\nflow limit\n    => i32\n{\n    => LIMIT\n}\n",
    );

    assert_message("A <- 1\n\nA <- 2\n", "duplicate global 'A'");
    assert_message(
        "flow A\n    => i32\n{\n    => 1\n}\n\nA <- 2\n",
        "conflicts with a function",
    );
    assert_message(
        "A <- 2\n\nflow A\n    => i32\n{\n    => 1\n}\n",
        "conflicts with a global",
    );
    assert_message(
        "flow value\n    => i32\n{\n    => 1\n}\n\nA <- value()\n",
        "must be a compile-time constant",
    );
    assert_clean("shape Vec2 [\n    x: i32\n    y: i32\n]\n\norigin <- Vec2[ x: 0, y: 0 ]\n");
    assert_clean("xs <- [i32: 1, 2]\n");
    assert_message(
        "DEFAULT: i32 <- 1\n\nshape I [\n    x: i32 = DEFAULT\n]\n",
        "unknown variable 'DEFAULT'",
    );
}

#[test]
fn give_of_a_global_is_rejected() {
    let messages = messages(
        "SAMPLE: [i32] <- [i32: 1, 2, 3]\n\nflow reader\n    => i32\n{\n    mut i: u64 <- 0\n    => SAMPLE[i]\n}\n\nflow poke\n    <- n: i32\n    => i32\n{\n    mut s <- give(SAMPLE)\n    s[0] = 99\n    => 0\n}\n\nopen flow main\n    => i32\n{\n    mut ignore <- poke(1)\n    => reader()\n}\n",
    );
    assert!(
        messages.iter().any(|message| {
            message.contains("cannot give from global 'SAMPLE'") && message.contains("clone")
        }),
        "expected a global-give diagnostic suggesting clone, got {messages:#?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| message.contains("use of moved value 'SAMPLE'")),
        "global move state must not leak between functions, got {messages:#?}"
    );
}

#[test]
fn give_of_a_string_global_is_rejected() {
    assert_message(
        "NAME: [u8] <- \"hi\"\n\nflow main\n    => i32\n{\n    mut name <- give(NAME)\n    => name.len as i32\n}\n",
        "cannot give from global 'NAME'; use clone(...)",
    );
}

#[test]
fn give_of_a_global_field_or_element_is_rejected() {
    assert_message(
        "shape Vec2 [\n    x: i32\n    y: i32\n]\n\nPOINT: Vec2 <- Vec2[ x: 1, y: 2 ]\n\nflow main\n    => i32\n{\n    mut x <- give(POINT.x)\n    => x\n}\n",
        "cannot give from global 'POINT'",
    );
    assert_message(
        "SAMPLE: [i32] <- [i32: 1, 2, 3]\n\nflow main\n    => i32\n{\n    mut x <- give(SAMPLE[0])\n    => x\n}\n",
        "cannot give from global 'SAMPLE'",
    );
    assert_message(
        "NAME: [u8] <- \"hi\"\n\nflow main\n    => i32\n{\n    mut name <- give(NAME)\n    => name.len as i32\n}\n",
        "cannot give from global 'NAME'",
    );
}

#[test]
fn global_moved_state_is_per_function_body_and_order_independent() {
    let poke_before_reader = "SAMPLE: [i32] <- [i32: 1, 2, 3]\n\nflow poke\n    => i32\n{\n    mut s <- give(SAMPLE)\n    s[0] = 99\n    => 0\n}\n\nflow reader\n    => i32\n{\n    mut i: u64 <- 0\n    => SAMPLE[i]\n}\n\nflow main\n    => i32\n{\n    mut ignore <- poke()\n    => reader()\n}\n";
    let reader_before_poke = "SAMPLE: [i32] <- [i32: 1, 2, 3]\n\nflow reader\n    => i32\n{\n    mut i: u64 <- 0\n    => SAMPLE[i]\n}\n\nflow poke\n    => i32\n{\n    mut s <- give(SAMPLE)\n    s[0] = 99\n    => 0\n}\n\nflow main\n    => i32\n{\n    mut ignore <- poke()\n    => reader()\n}\n";

    for source in [poke_before_reader, reader_before_poke] {
        let messages = messages(source);
        assert!(
            messages
                .iter()
                .any(|message| message.contains("cannot give from global 'SAMPLE'")),
            "expected the global-give error regardless of declaration order, got {messages:#?}"
        );
        assert!(
            !messages
                .iter()
                .any(|message| message.contains("use of moved value 'SAMPLE'")),
            "a move in one function must not affect another function, got {messages:#?}"
        );
    }
}

#[test]
fn clone_and_reads_of_globals_still_work() {
    assert_clean(
        "shape Vec2 [\n    x: i32\n    y: i32\n]\n\nORIGIN: Vec2 <- Vec2[ x: 0, y: 0 ]\nLIMIT: i32 <- 3\nNAME: [u8] <- \"hi\"\n\nflow main\n    => i32\n{\n    mut p <- clone(ORIGIN)\n    => p.x + LIMIT + NAME.len as i32\n}\n",
    );
}

#[test]
fn accepts_opaque_handles_extern_flows_and_cstr_literals() {
    assert_clean(
        "opaque Handle\n\n\
         extern flow make\n\
             <- x: i32\n\
             => Handle\n\n\
         extern flow take\n\
             <- h: Handle\n\n\
         extern flow title\n\
             <- text: cstr\n\
             => i32\n\n\
         flow use\n\
             => i32\n\
         {\n\
             mut h: Handle\n\
             h = make(1)\n\
             take(h)\n\
             => title(\"hello\")\n\
         }\n",
    );
}

#[test]
fn cstr_requires_a_string_literal() {
    assert_message(
        "extern flow title\n\
             <- text: cstr\n\
             => i32\n\n\
         flow use\n\
             <- text: [u8]\n\
             => i32\n\
         {\n\
             => title(text)\n\
         }\n",
        "argument 'text' has type [u8], expected cstr",
    );
}

#[test]
fn opaque_names_cannot_collide_with_other_declarations() {
    assert_message("opaque Same\nopaque Same\n", "duplicate type 'Same'");
    assert_message(
        "opaque Same\nshape Same [ x: i32 ]\n",
        "duplicate type 'Same'",
    );
    assert_message(
        "opaque Same\nflow Same {\n}\n",
        "conflicts with an opaque type",
    );
}

#[test]
fn equality_is_limited_to_scalars_and_byte_slices() {
    assert_clean(
        "shape Vec2 [\n    x: i32\n    y: i32\n]\n\nflow same\n    <- a: [u8]\n    <- b: [u8]\n    => bool\n{\n    => a == b\n}\n\nflow different\n    <- a: [u8]\n    <- b: [u8]\n    => bool\n{\n    => a != b\n}\n\nflow scalar\n    <- a: i32\n    <- b: i32\n    => bool\n{\n    => a == b\n}\n",
    );

    assert_message(
        "shape Vec2 [\n    x: i32\n    y: i32\n]\n\nflow same\n    <- a: Vec2\n    <- b: Vec2\n    => bool\n{\n    => a == b\n}\n",
        "only scalars and [u8] values support == and !=",
    );
    assert_message(
        "flow same\n    <- a: [i32]\n    <- b: [i32]\n    => bool\n{\n    => a == b\n}\n",
        "only scalars and [u8] values support == and !=",
    );
}

#[test]
fn disjoint_constant_indices_do_not_alias() {
    assert_clean(
        "flow pair\n    -> a: i32\n    -> b: i32\n{\n    a = 1\n    b = 2\n}\n\nflow f\n    -> xs: [i32]\n{\n    alloc(-> xs, i32, 2)\n    pair(-> xs[0], -> xs[1])\n}\n",
    );

    assert_message(
        "flow pair\n    -> a: i32\n    -> b: i32\n{\n    a = 1\n    b = 2\n}\n\nflow f\n    -> xs: [i32]\n{\n    alloc(-> xs, i32, 2)\n    pair(-> xs[0], -> xs[0])\n}\n",
        "cannot pass 'xs' as '->'",
    );
}

#[test]
fn slice_literal_cannot_escape_the_block_that_created_it() {
    assert_message(
        "open flow main => i32 {\n    mut outer: [i32]\n    if true {\n        outer = [7, 8, 9]\n    } else {\n        outer = [1, 2, 3]\n    }\n    mut i: u64 <- 1\n    => outer[i]\n}\n",
        "slice literal storage escapes the block",
    );

    assert_message(
        "shape Bag [\n    items: [i32]\n]\n\nflow main {\n    mut bag: Bag\n    if true {\n        bag = Bag[ items: [1, 2] ]\n    } else {\n        bag = Bag[ items: [3, 4] ]\n    }\n}\n",
        "slice literal storage escapes the block",
    );
}

#[test]
fn returning_a_slice_literal_through_a_local_is_rejected() {
    assert_message(
        "flow f\n    => [i32]\n{\n    xs <- [1, 2]\n    => xs\n}\n",
        "cannot return a slice literal",
    );
}

#[test]
fn slice_literals_are_safe_within_their_block() {
    assert_clean(
        "flow first\n    <- xs: [i32]\n    => i32\n{\n    => xs[0]\n}\n\nflow main\n    => i32\n{\n    xs <- [1, 2, 3]\n    shape_local <- Bag[ items: [4, 5] ]\n    => first(xs) + shape_local.items[1]\n}\n\nshape Bag [\n    items: [i32]\n]\n",
    );

    assert_clean(
        "flow take\n    <- xs: [i32]\n    => i32\n{\n    => xs[0]\n}\n\nflow main\n    => i32\n{\n    => take([1, 2, 3])\n}\n",
    );
}

#[test]
fn bitwise_and_shift_operators_type_check() {
    assert_clean(
        "flow hash\n    <- x: u32\n    => u32\n{\n    => ((x << 5) ^ (x >> 3)) & 0xFFFF | 1\n}\n\nflow flags\n    <- a: u8\n    => u8\n{\n    => ~a & 0x0F\n}\n\nflow main {\n    mut x: i32 <- 3\n    x <<= 2\n    x |= 1\n    x &= 7\n    x ^= 2\n    x %= 5\n}\n",
    );

    assert_message(
        "flow f\n    <- x: f64\n    => i32\n{\n    => x & 1\n}\n",
        "bitwise operator requires integer operands",
    );
    assert_message(
        "flow f\n    => f64\n{\n    => 1.0 << 2\n}\n",
        "shift requires integer operands",
    );
    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    => x << 32\n}\n",
        "shift count 32 is out of range for i32",
    );
    assert_message(
        "flow f\n    <- x: i32\n    => i32\n{\n    => x >> -1\n}\n",
        "shift count -1 is out of range for i32",
    );
    assert_message(
        "flow f\n    <- x: f64\n    => f64\n{\n    => ~x\n}\n",
        "'~' requires an integer operand",
    );
    assert_message(
        "flow f {\n    mut x: f64 <- 1.0\n    x %= 2.0\n}\n",
        "compound assignment requires an integer target",
    );
}

#[test]
fn file_and_stdin_intrinsics_have_checked_signatures() {
    assert_clean(
        "flow load\n    <- path: [u8]\n    => i32\n{\n    protect {\n        mut data: [u8] <- read_file(<- path)\n        => data.len as i32\n    }\n}\n\nflow save\n    <- path: [u8]\n    <- data: [u8]\n    => bool\n{\n    => write_file(<- path, <- data)\n}\n\nflow main {\n    protect {\n        mut input: [u8] <- read_stdin()\n        mut line: [u8] <- read_line()\n        if line.len > 0 {\n            input = give(line)\n        }\n        let_saved: bool <- save(<- \"out.bin\", <- input)\n    }\n    print_int(255 as u64)\n    print_float(1.5)\n}\n",
    );

    assert_message(
        "flow main {\n    mut data: [u8] <- read_file(<- \"x\")\n}\n",
        "function 'main' can allocate outside a 'protect' block",
    );
    assert_message(
        "flow main {\n    mut data: [u8]\n    protect {\n        data = read_file(<- \"x\")\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );
    assert_message(
        "flow main {\n    mut line: [u8]\n    protect {\n        line = read_line()\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );
    assert_message(
        "flow main {\n    print_int(1.0)\n}\n",
        "print_int expects an integer",
    );
    assert_message(
        "flow main {\n    print_float(1)\n}\n",
        "print_float expects a float",
    );
    assert_message(
        "flow main {\n    mut ok: i32 <- write_file(<- \"x\", <- \"y\")\n}\n",
        "cannot initialize 'ok' of type i32 with bool",
    );
    assert_message(
        "flow main {\n    mut input: [u8]\n    protect {\n        input = read_stdin()\n    }\n}\n",
        "value allocated inside 'protect' escapes the block",
    );
}

#[test]
fn generics_are_monomorphized_before_checking() {
    assert_clean(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow set_at<T>\n    <-> xs: Vec<T>\n    <- index: u64\n    <- value: T\n{\n    xs.data[index] = value\n}\n\nflow main\n    => i32\n{\n    mut v <- Vec<i32>[ data: [i32: 1, 2], len: 2 ]\n    set_at<i32>(<-> v, 0, 9)\n    => 0\n}\n",
    );
    assert_clean(
        "shape Pair<T, U> [\n    first: T\n    second: U\n]\n\nflow main\n    => i32\n{\n    p <- Pair<i32, bool>[ first: 1, second: true ]\n    => p.first\n}\n",
    );
}

#[test]
fn generic_instantiation_requires_explicit_type_arguments() {
    assert_message(
        "shape Vec<T> [\n    data: [T]\n]\n\nG: Vec <- Vec[]\n",
        "requires explicit type arguments",
    );
    assert_message(
        "flow identity<T>\n    <- x: T\n    => T\n{\n    => x\n}\n\nflow main\n    => i32\n{\n    => identity(1)\n}\n",
        "requires explicit type arguments",
    );
}

#[test]
fn generic_type_argument_counts_are_checked() {
    assert_message(
        "shape Pair<T, U> [\n    first: T\n    second: U\n]\n\nflow main\n    => i32\n{\n    p <- Pair<i32>[ first: 1, second: 2 ]\n    => p.first\n}\n",
        "expects 2 type argument(s), found 1",
    );
    assert_message(
        "flow identity<T>\n    <- x: T\n    => T\n{\n    => x\n}\n\nflow main\n    => i32\n{\n    => identity<i32, bool>(1)\n}\n",
        "expects 1 type argument(s), found 2",
    );
}

#[test]
fn comptime_flows_are_pure() {
    assert_clean(
        "comptime flow double\n    <- x: i32\n    => i32\n{\n    => x * 2\n}\n\nflow main\n    => i32\n{\n    => comptime double(21)\n}\n",
    );
    assert_message(
        "comptime flow bad\n    => i32\n{\n    mut xs: [i32]\n    alloc(-> xs, i32, 3)\n    => 0\n}\n",
        "'alloc' is not allowed in a comptime flow",
    );
    assert_message(
        "comptime flow bad\n    => i32\n{\n    protect {\n        => 1\n    }\n    => 0\n}\n",
        "'protect' is not allowed in a comptime flow",
    );
    assert_message(
        "flow helper\n    => i32\n{\n    => 1\n}\n\ncomptime flow bad\n    => i32\n{\n    => helper()\n}\n",
        "flow 'helper' is not declared 'comptime'",
    );
    assert_message(
        "mut G: i32 <- 0\n\ncomptime flow bad\n    => i32\n{\n    G = 1\n    => G\n}\n",
        "a comptime flow cannot mutate a global",
    );
}

#[test]
fn comptime_expressions_must_be_constant() {
    assert_message(
        "G: i32 <- comptime read_file(\"x\")\n",
        "I/O is not allowed in a comptime expression",
    );
    assert_message(
        "flow main\n    => i32\n{\n    x <- 1\n    => comptime x\n}\n",
        "not a compile-time constant",
    );
}

#[test]
fn globals_accept_any_constant_shape() {
    assert_clean(
        "shape Color [\n    r: u8\n    g: u8\n    b: u8\n    a: u8\n]\n\nshape Vec2 [\n    x: f64\n    y: f64\n]\n\nshape Scene [\n    color: Color\n    points: [Vec2]\n    name: [u8]\n]\n\nGREEN: Color <- Color[ r: 0, g: 255, b: 0, a: 255 ]\n\nORIGIN <- Vec2[ x: 0.0, y: 0.0 ]\n\nSCENE: Scene <- Scene[ color: Color[ r: 0, g: 255, b: 0, a: 255 ], points: [Vec2: Vec2[ x: 0.0, y: 0.0 ]], name: \"scene\" ]\n",
    );
}

#[test]
fn comptime_flows_can_initialize_globals() {
    assert_clean(
        "comptime flow triple\n    <- x: i32\n    => i32\n{\n    => x * 3\n}\n\nLIMIT: i32 <- comptime triple(7)\n",
    );
    assert_message(
        "flow triple\n    <- x: i32\n    => i32\n{\n    => x * 3\n}\n\nLIMIT: i32 <- triple(7)\n",
        "must be a compile-time constant",
    );
}

#[test]
fn comptime_overflow_is_diagnosed_like_a_runtime_trap() {
    assert_message(
        "LIMIT: i32 <- comptime 2147483647 + 1\n\nflow main\n    => i32\n{\n    => LIMIT\n}\n",
        "integer overflow in comptime expression",
    );
    assert_message(
        "comptime flow bump\n    <- x: i32\n    => i32\n{\n    => x + 1\n}\n\nLIMIT: i32 <- comptime bump(2147483647)\n\nflow main\n    => i32\n{\n    => LIMIT\n}\n",
        "integer overflow in comptime expression",
    );
    assert_message(
        "LIMIT: i8 <- comptime -127 - 2\n\nflow main\n    => i32\n{\n    => LIMIT as i32\n}\n",
        "integer overflow in comptime expression",
    );
}

#[test]
fn comptime_explicit_casts_still_truncate() {
    assert_clean(
        "TINY: i8 <- comptime 300 as i8\n\nflow main\n    => i32\n{\n    => TINY as i32\n}\n",
    );
}

#[test]
fn comptime_unsigned_arithmetic_wraps_at_its_width() {
    assert_clean(
        "U: u8 <- comptime 250 + 10\nV: u8 <- comptime 255 - 255\nW: u8 <- comptime 200 as u8\n\nflow main\n    => i32\n{\n    => U as i32 + V as i32 + W as i32\n}\n",
    );
    assert_message(
        "SHIFT: u8 <- comptime 1 << 8\n\nflow main\n    => i32\n{\n    => SHIFT as i32\n}\n",
        "shift count is out of range",
    );
}

#[test]
fn recursive_generic_instantiation_reports_a_depth_limit() {
    let messages = messages(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow f<T>\n    <- x: T\n    => i32\n{\n    empty <- Vec<T>[ data: [T:], len: 0 ]\n    => f<Vec<T>>(empty)\n}\n\nflow main\n    => i32\n{\n    => f<i32>(1)\n}\n",
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("depth limit")),
        "expected a depth-limit diagnostic, got {messages:#?}"
    );
    assert!(
        messages.len() == 1,
        "depth-limit errors should not cascade, got {messages:#?}"
    );
}

#[test]
fn generated_names_colliding_with_user_shapes_are_reported_clearly() {
    let messages = messages(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nshape Vec__i32 [\n    x: i32\n]\n\nflow main\n    => i32\n{\n    v <- Vec<i32>[ data: [i32: 1], len: 1 ]\n    w <- Vec__i32[ x: 2 ]\n    => v.len as i32 + w.x\n}\n",
    );
    assert!(
        messages.iter().any(|message| {
            message.contains("generated name 'Vec__i32'")
                && message.contains("Vec<i32>")
                && message.contains("collides")
        }),
        "expected a clear collision diagnostic, got {messages:#?}"
    );
}

#[test]
fn unused_generic_templates_are_still_checked() {
    assert_message(
        "flow broken<T>\n    <- x: T\n    => i32\n{\n    => missing_name + x\n}\n\nflow main\n    => i32\n{\n    => 0\n}\n",
        "unknown variable 'missing_name'",
    );
    assert_clean(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow unused_sum<T>\n    <- xs: Vec<T>\n    => i32\n{\n    mut total: i32 <- 0\n    for i in 0..xs.len {\n        total += xs.data[i] as i32\n    }\n    => total\n}\n\nflow main\n    => i32\n{\n    => 0\n}\n",
    );
}

#[test]
fn suggestions_stay_inside_the_name_namespace() {
    let variable = diagnostics(
        "shape Counter [\n    value: i32\n]\n\nmut counter: i32 <- 0\n\nflow main {\n    mut x: i32 <- cuonter\n}\n",
    );
    assert_eq!(variable.len(), 1, "{variable:#?}");
    assert_eq!(variable[0].message, "unknown variable 'cuonter'");
    assert_eq!(variable[0].help.as_deref(), Some("did you mean 'counter'?"));

    let function = diagnostics(
        "shape Count [\n    value: i32\n]\n\nflow count\n    => i32\n{\n    => 0\n}\n\nflow main {\n    cnout()\n}\n",
    );
    assert_eq!(function.len(), 1, "{function:#?}");
    assert_eq!(function[0].message, "unknown function 'cnout'");
    assert_eq!(function[0].help.as_deref(), Some("did you mean 'count'?"));

    let ty = diagnostics(
        "shape Counter [\n    value: i32\n]\n\nmut counter: i32 <- 0\n\nflow main {\n    x: Cuonter <- Counter[ value: 0 ]\n}\n",
    );
    assert_eq!(ty.len(), 1, "{ty:#?}");
    assert_eq!(ty[0].message, "unknown type 'Cuonter'");
    assert_eq!(ty[0].help.as_deref(), Some("did you mean 'Counter'?"));
}

#[test]
fn unknown_variable_does_not_cascade_into_assignment_errors() {
    let found = diagnostics(
        "shape Counter [\n    value: i32\n]\n\nflow main {\n    mut counter <- Counter[ value: 0 ]\n    cuonter.value += 1\n}\n",
    );
    assert_eq!(
        found.len(),
        1,
        "one root cause should produce one error: {found:#?}"
    );
    assert_eq!(found[0].message, "unknown variable 'cuonter'");
    assert_eq!(found[0].help.as_deref(), Some("did you mean 'counter'?"));
}

#[test]
fn diagnostics_use_source_spelling_for_generic_instantiations() {
    let messages = messages(
        "shape Vec<T> [\n    data: [T]\n]\n\nflow main\n    => i32\n{\n    x: Vec<i32> <- Vec<i64>[ data: [i64: 1] ]\n    => 0\n}\n",
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Vec<i32>") && message.contains("Vec<i64>")),
        "expected source spellings in the diagnostic, got {messages:#?}"
    );
    assert!(
        !messages.iter().any(|message| message.contains("__")),
        "diagnostics must not expose mangled names, got {messages:#?}"
    );
}

#[test]
fn printf_format_arguments_are_checked() {
    assert_clean("flow main {\n    printf(\"n=%d f=%.2f s=%s pct=%%\\n\", 1, 2.5, \"ok\")\n}\n");
    assert_clean("flow main {\n    title: cstr <- \"hi\"\n    printf(\"%s\\n\", title)\n}\n");

    assert_message(
        "flow main {\n    printf(\"%d %d\", 1)\n}\n",
        "printf format expects 2 argument(s), found 1",
    );
    assert_message(
        "flow main {\n    printf(\"%d\", 1, 2)\n}\n",
        "printf format expects 1 argument(s), found 2",
    );
    assert_message(
        "flow main {\n    f: f64 <- 1.0\n    printf(\"%d\", f)\n}\n",
        "format '%d' expects an integer, found f64",
    );
    assert_message(
        "flow main {\n    printf(\"%q\", 1)\n}\n",
        "invalid format string: unsupported conversion '%q'",
    );
    assert_message(
        "flow main {\n    printf(\"%*d\", 1, 2)\n}\n",
        "invalid format string: dynamic '*' width/precision is not supported",
    );
    assert_message(
        "flow main {\n    fmt <- \"%d\"\n    printf(fmt, 1)\n}\n",
        "printf format must be a string literal",
    );
}

#[test]
fn mark_and_choice_program_type_checks() {
    assert_clean(
        "mark Color [ Red, Green, Blue ]\nchoice Result [ Ok: i32, Err: i64 ]\nflow main {\n    c <- Color.Green\n    r <- Result.Ok(42)\n    if c == Color.Green and r is Result.Ok {\n        print_int(r.Ok)\n    }\n    if r is Result.Err {\n        print_int(r.Err)\n    }\n}\n",
    );
}

#[test]
fn mark_variant_names_are_checked() {
    assert_message(
        "mark Color [ Red ]\nflow main {\n    c <- Color.Blue\n}\n",
        "mark 'Color' has no variant 'Blue'",
    );
    assert_message(
        "mark Color [ Red, Red ]\n",
        "duplicate variant 'Red' in mark 'Color'",
    );
}

#[test]
fn marks_are_only_comparable_to_the_same_mark() {
    assert_clean("mark A [ X ]\nmark B [ Y ]\nflow main {\n    a <- A.X\n    if a == A.X { }\n}\n");
    assert_message(
        "mark A [ X ]\nmark B [ Y ]\nflow main {\n    a <- A.X\n    b <- B.Y\n    if a == b { }\n}\n",
        "cannot compare A with B",
    );
    assert_message(
        "mark A [ X ]\nflow main {\n    a <- A.X\n    if a < A.X { }\n}\n",
        "operator requires numeric operands",
    );
}

#[test]
fn choice_constructor_arguments_are_checked() {
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Ok(\"hi\")\n}\n",
        "argument of 'R.Ok' has type [u8], expected i32",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Ok()\n}\n",
        "choice variant 'R.Ok' expects 1 argument, found 0",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    mut x: i32\n    r <- R.Ok(-> x)\n}\n",
        "expects a '<-' argument",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Missing(1)\n}\n",
        "choice 'R' has no variant 'Missing'",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    v <- R.Ok\n}\n",
        "must be constructed with a value",
    );
    assert_message(
        "choice R [ Ok: i32, Ok: i64 ]\n",
        "duplicate variant 'Ok' in choice 'R'",
    );
}

#[test]
fn is_requires_a_choice_and_a_matching_variant() {
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    n: i32 <- 1\n    if n is R.Ok { }\n}\n",
        "'is' requires a choice value on the left, found i32",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nchoice S [ Yes: bool ]\nflow main {\n    r <- R.Ok(1)\n    if r is S.Yes { }\n}\n",
        "variant 'S.Yes' does not belong to choice 'R'",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Ok(1)\n    if r is R.Nope { }\n}\n",
        "choice 'R' has no variant 'Nope'",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Ok(1)\n    if r is 1 { }\n}\n",
        "'is' expects a choice variant on the right",
    );
}

#[test]
fn choice_payload_reads_are_checked_but_not_statically_narrowed() {
    assert_clean(
        "choice R [ Ok: i32, Err: i64 ]\nflow main {\n    r <- R.Ok(1)\n    print_int(r.Err)\n}\n",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Ok(1)\n    print_int(r.Nope)\n}\n",
        "choice 'R' has no variant 'Nope'",
    );
    assert_message(
        "choice R [ Ok: i32 ]\nflow main {\n    r <- R.Ok(1)\n    r.Ok = 2\n}\n",
        "cannot assign to a choice payload",
    );
}

#[test]
fn choice_payload_types_are_restricted() {
    assert_clean("shape Point [ x: i32, y: i32 ]\nchoice R [ P: Point ]\n");

    assert_message(
        "choice R [ Bytes: [u8] ]\n",
        "choice 'R' variant 'Bytes' cannot hold [u8]",
    );
    assert_message(
        "choice R [ Nested: S ]\nchoice S [ Yes: bool ]\n",
        "choice 'R' variant 'Nested' cannot hold S",
    );
    assert_message(
        "shape WithSlice [ data: [u8] ]\nchoice R [ P: WithSlice ]\n",
        "cannot hold WithSlice",
    );
}

#[test]
fn mark_and_choice_names_cannot_collide() {
    assert_message("mark A [ X ]\nshape A []\n", "duplicate type 'A'");
    assert_message("choice A [ X: i32 ]\nmark A [ Y ]\n", "duplicate type 'A'");
    assert_message("mark A [ X ]\nflow A { }\n", "conflicts with a type");
}

#[test]
fn choices_can_flow_through_flows_and_shapes() {
    assert_clean(
        "choice R [ Ok: i32, Err: i64 ]\nflow take\n    <- r: R\n    => i32\n{\n    => r.Ok\n}\nflow main {\n    r <- R.Ok(3)\n    print_int(take(r))\n}\n",
    );
    assert_clean(
        "choice R [ Ok: i32, Err: i64 ]\nshape Box [ value: R ]\nflow main {\n    r <- R.Ok(3)\n    b: Box <- Box[ value: give(r) ]\n    if b.value is R.Ok {\n        print_int(b.value.Ok)\n    }\n}\n",
    );
}
