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
fn an_uncalled_visible_allocator_must_protect_its_allocations() {
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nopen flow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
        "function 'fill' can allocate outside a 'protect' block",
    );
}

#[test]
fn an_uncalled_allocator_with_default_visibility_is_an_entry() {
    // A function with no visibility keyword is `hidden` by default (spec
    // question 3). Hidden functions that are not called by any other function
    // are never called, so they can allocate outside a protect block without
    // error. Only `main` and visible functions are entry points.
    assert_clean(
        "shape Particle [\n    mass: f64\n]\n\nflow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
    );

    // A visible function that is not called by any other function IS an entry
    // point and must protect its allocations.
    assert_message(
        "shape Particle [\n    mass: f64\n]\n\nopen flow fill\n    -> ps: [Particle]\n{\n    alloc(-> ps, Particle, 3)\n}\n\nflow main {\n}\n",
        "function 'fill' can allocate outside a 'protect' block",
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
    assert_message(
        "shape Vec2 [\n    x: i32\n    y: i32\n]\n\norigin <- Vec2[ x: 0, y: 0 ]\n",
        "only scalar and [u8] globals are supported",
    );
    assert_message(
        "xs <- [i32: 1, 2]\n",
        "only scalar and [u8] globals are supported",
    );
    assert_message(
        "DEFAULT: i32 <- 1\n\nshape I [\n    x: i32 = DEFAULT\n]\n",
        "unknown variable 'DEFAULT'",
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
