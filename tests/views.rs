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

#[test]
fn view_of_borrowed_slice_can_be_returned() {
    assert_clean(
        "flow head2\n    <- xs: [u8]\n    => [u8]\n{\n    => view(xs, 0, 2)\n}\n\nflow main\n    => i32\n{\n    protect {\n        data: [u8] <- [u8: 1, 2, 3]\n        part <- head2(data)\n        if part.len != 2 { => 1 }\n    }\n    => 0\n}\n",
    );
}

#[test]
fn borrowed_view_can_be_forwarded_through_two_flows() {
    assert_clean(
        "flow inner\n    <- xs: [u8]\n    => [u8]\n{\n    => view(xs, 1, 3)\n}\n\nflow outer\n    <- xs: [u8]\n    => [u8]\n{\n    => inner(xs)\n}\n\nflow main\n    => i32\n{\n    protect {\n        data: [u8] <- [u8: 1, 2, 3, 4]\n        part <- outer(data)\n        if part.len != 2 { => 1 }\n    }\n    => 0\n}\n",
    );
}

#[test]
fn view_of_a_view_is_still_borrowed_from_the_original_source() {
    assert_clean(
        "flow inner\n    <- xs: [u8]\n    => [u8]\n{\n    => view(xs, 1, 4)\n}\n\nflow outer\n    <- xs: [u8]\n    => [u8]\n{\n    part <- inner(xs)\n    => view(part, 0, 2)\n}\n",
    );
}

#[test]
fn view_result_keeps_the_storage_region_of_its_source() {
    assert_message(
        "flow main\n    => i32\n{\n    mut escaped: [u8]\n    if true {\n        data: [u8] <- [u8: 1, 2, 3]\n        escaped = view(data, 0, 2)\n    }\n    => escaped.len as i32\n}\n",
        "escapes the block",
    );
}

#[test]
fn writing_to_the_source_while_a_view_is_live_is_rejected() {
    assert_message(
        "flow main {\n    mut data: [u8] <- [u8: 1, 2, 3]\n    part <- view(data, 0, 2)\n    data[0] = 9\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn moving_the_source_while_a_view_is_live_is_rejected() {
    assert_message(
        "flow main {\n    data: [u8] <- [u8: 1, 2, 3]\n    part <- view(data, 0, 2)\n    moved <- give(data)\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn writing_to_the_source_of_a_returned_view_is_rejected_at_the_caller() {
    assert_message(
        "flow head2\n    <- xs: [u8]\n    => [u8]\n{\n    => view(xs, 0, 2)\n}\n\nflow main {\n    mut data: [u8] <- [u8: 1, 2, 3]\n    part <- head2(data)\n    data[0] = 9\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn a_view_loan_ends_when_its_variable_goes_out_of_scope() {
    assert_clean(
        "flow main {\n    mut data: [u8] <- [u8: 1, 2, 3]\n    if true {\n        part <- view(data, 0, 2)\n        if part.len != 2 { => }\n    }\n    data[0] = 9\n}\n",
    );
}

#[test]
fn cloning_does_not_borrow_the_source() {
    assert_clean(
        "flow main {\n    protect {\n        mut data: [u8] <- [u8: 1, 2, 3]\n        part <- clone(data)\n        data[0] = 9\n    }\n}\n",
    );
}

#[test]
fn returning_a_borrowed_composite_is_still_rejected() {
    assert_message(
        "shape S [\n    data: [u8]\n]\n\nflow bad\n    <- s: S\n    => S\n{\n    => s\n}\n",
        "cannot return a value borrowed from '<-' parameter",
    );
}

#[test]
fn view_checks_its_arguments() {
    assert_message(
        "flow main {\n    xs: [u8] <- \"abc\"\n    part <- view(xs, 0)\n}\n",
        "view expects 3 argument(s)",
    );
    assert_message(
        "flow main {\n    n <- 1\n    part <- view(n, 0, 1)\n}\n",
        "view expects a slice",
    );
}

#[test]
fn a_loan_in_one_field_survives_writing_a_sibling_field() {
    assert_message(
        "shape S [\n    a: [u8]\n    b: [u8]\n]\n\nflow main {\n    protect {\n        mut x: [u8] <- [u8: 1, 2, 3]\n        mut y: [u8] <- [u8: 4, 5, 6]\n        mut s: S\n        s.a = view(x, 0, 2)\n        s.b = view(y, 0, 2)\n        x[0] = 9\n    }\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn overwriting_a_slot_releases_its_old_loan() {
    assert_clean(
        "shape Inner [\n    data: [u8]\n]\n\nshape S [\n    b: [u8]\n]\n\nflow main {\n    protect {\n        mut x: Inner <- Inner[ data: [u8: 1, 2, 3] ]\n        mut y: [u8] <- [u8: 4, 5, 6]\n        mut s: S\n        s.b = view(x.data, 0, 2)\n        s.b = view(y, 0, 2)\n        x.data[0] = 9\n    }\n}\n",
    );
}

#[test]
fn a_loan_survives_taking_an_element_of_a_nested_view() {
    assert_message(
        "flow main {\n    protect {\n        mut outer: [[[u8]]]\n        alloc(-> outer, [[u8]], 1)\n        alloc(-> outer[0], [u8], 2)\n        alloc(-> outer[0][0], u8, 1)\n        alloc(-> outer[0][1], u8, 1)\n        outer[0][0][0] = 1\n        outer[0][1][0] = 2\n        mut elem: [u8]\n        if true {\n            part <- view(outer[0], 0, 2)\n            elem = give(part[1])\n        }\n        outer[0][1][0] = 9\n    }\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn a_view_inside_a_returned_composite_keeps_the_source_borrowed() {
    assert_message(
        "shape S [\n    data: [u8]\n]\n\nflow wrap\n    <- xs: [u8]\n    => S\n{\n    mut s: S\n    s.data = view(xs, 0, 2)\n    => s\n}\n\nflow main {\n    protect {\n        mut x: [u8] <- [u8: 1, 2, 3]\n        w <- wrap(x)\n        x[0] = 9\n    }\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn a_view_returned_through_an_out_parameter_keeps_the_source_borrowed() {
    assert_message(
        "flow fill\n    <- xs: [u8]\n    -> out: [u8]\n{\n    out = view(xs, 0, 2)\n}\n\nflow main {\n    protect {\n        mut x: [u8] <- [u8: 1, 2, 3]\n        mut r: [u8]\n        fill(x, -> r)\n        x[0] = 9\n    }\n}\n",
        "borrowed by a slice view",
    );
}

#[test]
fn view_lowers_to_a_checked_helper() {
    let tokens = tokenize(
        "flow main\n    => i32\n{\n    xs: [u8] <- \"abc\"\n    part <- view(xs, 1, 2)\n    => part.len as i32\n}\n",
        "<test>",
    )
    .expect("source should lex");
    let program = parse(tokens).expect("source should parse");
    assert!(check(&program).is_empty());
    let c = generate_c(&program, None).expect("source should generate C");
    assert!(c.contains("loi_view("), "missing view helper call in:\n{c}");
}
