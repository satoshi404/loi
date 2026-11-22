use loi::ast::*;
use loi::{format_program, parse, tokenize};

fn parse_source(source: &str) -> Program {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    parse(tokens).expect("source should parse")
}

fn reference_program() -> Program {
    parse_source(include_str!("../examples/particles.loi"))
}

#[test]
fn spans_carry_the_token_extent() {
    let program = parse_source("flow main => i32 {\n    => missing + 1\n}\n");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected a function");
    };

    assert_eq!((function.span.line, function.span.column), (1, 1));
    assert_eq!(function.span.end_column, Some(5), "the span covers 'flow'");

    let StmtKind::Return(Some(value)) = &function.body.statements[0].kind else {
        panic!("expected a return statement");
    };
    assert_eq!((value.span.line, value.span.column), (2, 8));
    assert_eq!(
        value.span.end_column,
        Some(19),
        "the span covers the whole 'missing + 1' expression"
    );
}

#[test]
fn expression_spans_cover_whole_expressions() {
    let program = parse_source("flow main {\n    xs <- [1, 2, 3]\n    thing().field\n}\n");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected a function");
    };

    let StmtKind::Let {
        value: Some(value), ..
    } = &function.body.statements[0].kind
    else {
        panic!("expected a binding with an initializer");
    };
    assert_eq!((value.span.line, value.span.column), (2, 11));
    assert_eq!(value.span.end_line(), 2);
    assert_eq!(value.span.end_column, Some(20), "covers '[1, 2, 3]'");

    let StmtKind::Expr(expr) = &function.body.statements[1].kind else {
        panic!("expected an expression statement");
    };
    assert_eq!((expr.span.line, expr.span.column), (3, 5));
    assert_eq!(expr.span.end_column, Some(18), "covers 'thing().field'");
}

#[test]
fn multi_line_expression_spans_record_the_end_line() {
    let program = parse_source("flow main {\n    xs <- [1,\n           2]\n}\n");
    let Item::Function(function) = &program.items[0] else {
        panic!("expected a function");
    };
    let StmtKind::Let {
        value: Some(value), ..
    } = &function.body.statements[0].kind
    else {
        panic!("expected a binding with an initializer");
    };
    assert_eq!((value.span.line, value.span.column), (2, 11));
    assert_eq!(value.span.end_line, Some(3));
    assert_eq!(value.span.end_column, Some(14));
}

fn function<'a>(program: &'a Program, name: &str) -> &'a FunctionDecl {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == name => Some(function),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing function {name}"))
}

fn interface<'a>(program: &'a Program, name: &str) -> &'a InterfaceDecl {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Interface(interface) if interface.name == name => Some(interface),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing interface {name}"))
}

fn kind(expr: &Expr) -> &ExprKind {
    &expr.kind
}

#[test]
fn parses_the_reference_program() {
    let program = reference_program();
    assert_eq!(program.items.len(), 9);

    let names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Interface(interface) => Some(interface.name.as_str()),
            Item::Mark(mark) => Some(mark.name.as_str()),
            Item::Choice(choice) => Some(choice.name.as_str()),
            Item::Opaque(opaque) => Some(opaque.name.as_str()),
            Item::Function(function) => Some(function.name.as_str()),
            Item::Global(global) => Some(global.name.as_str()),
            Item::Import(_) => None,
        })
        .collect();

    assert_eq!(
        names,
        vec![
            "Vec2",
            "Particle",
            "World",
            "world_new",
            "clamp",
            "bounce",
            "step",
            "total_mass",
            "main",
        ]
    );
}

#[test]
fn parses_opaque_and_extern_flow_declarations() {
    let program = parse_source(
        "opaque Handle\n\
         \n\
         extern flow make\n\
             <- x: i32\n\
             => Handle\n\
         \n\
         extern flow release\n\
             <- h: Handle\n\
         \n\
         extern flow renamed as c_renamed\n\
             <- text: cstr\n\
             => i32\n",
    );

    let opaque = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Opaque(opaque) => Some(opaque),
            _ => None,
        })
        .expect("opaque declaration should parse");
    assert_eq!(opaque.name, "Handle");

    let make = function(&program, "make");
    assert!(make.external);
    assert_eq!(make.link_name, None);
    assert_eq!(make.params.len(), 1);
    assert_eq!(make.return_type, Some(Type::named("Handle")));

    let renamed = function(&program, "renamed");
    assert!(renamed.external);
    assert_eq!(renamed.link_name.as_deref(), Some("c_renamed"));

    let tokens = tokenize("extern flow bad {\n}\n", "<test>").expect("source should lex");
    assert!(
        parse(tokens).is_err(),
        "an extern flow must not have a body"
    );
}

#[test]
fn parses_imports_globals_strings_and_slice_literals() {
    let program = parse_source(
        "import \"lib/std.loi\"\n\
         GREETING: [u8] <- \"hi\\n\"\n\
         mut COUNT <- 2\n\
         flow main\n\
             => i32\n\
         {\n\
             typed: [i32] <- [i32: 1, 2]\n\
             inferred <- [10, 20]\n\
             => typed.len as i32 + inferred.len as i32\n\
         }\n",
    );

    let import = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Import(import) => Some(import),
            _ => None,
        })
        .expect("import should parse");
    assert_eq!(import.path, "lib/std.loi");

    let global = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Global(global) if global.name == "GREETING" => Some(global),
            _ => None,
        })
        .expect("greeting global should parse");
    assert!(!global.mutable);
    assert_eq!(global.ty, Some(Type::slice(Type::named("u8"))));
    assert_eq!(kind(&global.value), &ExprKind::String("hi\n".to_string()));

    let count = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Global(global) if global.name == "COUNT" => Some(global),
            _ => None,
        })
        .expect("count global should parse");
    assert!(count.mutable);
    assert_eq!(count.ty, None);

    let main = function(&program, "main");
    let StmtKind::Let {
        value: Some(typed), ..
    } = &main.body.statements[0].kind
    else {
        panic!("expected a typed let");
    };
    let ExprKind::SliceLiteral {
        element_type,
        elements,
    } = kind(typed)
    else {
        panic!("expected an explicit slice literal");
    };
    assert_eq!(element_type, &Some(Type::named("i32")));
    assert_eq!(elements.len(), 2);
    assert!(matches!(kind(&elements[0]), ExprKind::Int(text) if text == "1"));
    assert!(matches!(kind(&elements[1]), ExprKind::Int(text) if text == "2"));

    let StmtKind::Let {
        value: Some(inferred),
        ..
    } = &main.body.statements[1].kind
    else {
        panic!("expected an inferred let");
    };
    let ExprKind::SliceLiteral {
        element_type: None,
        elements,
    } = kind(inferred)
    else {
        panic!("expected an inferred slice literal");
    };
    assert_eq!(elements.len(), 2);
}

#[test]
fn parses_interfaces_and_field_defaults() {
    let program = reference_program();

    let vec2 = interface(&program, "Vec2");
    assert_eq!(vec2.fields.len(), 2);
    assert_eq!(vec2.fields[0].name, "x");
    assert_eq!(vec2.fields[0].ty, Type::named("f64"));

    let particle = interface(&program, "Particle");
    assert_eq!(particle.fields.len(), 4);
    let alive = &particle.fields[3];
    assert_eq!(alive.name, "alive");
    assert_eq!(alive.ty, Type::named("bool"));
    assert_eq!(
        alive.default.as_ref().map(kind),
        Some(&ExprKind::Bool(true))
    );

    let world = interface(&program, "World");
    assert_eq!(world.fields[0].ty, Type::slice(Type::named("Particle")));
    assert_eq!(
        world.fields[3].default.as_ref().map(kind),
        Some(&ExprKind::Int("0".to_string()))
    );
}

#[test]
fn parses_function_signatures() {
    let program = reference_program();

    let world_new = function(&program, "world_new");
    assert_eq!(world_new.visibility, Visibility::Visible);
    assert_eq!(world_new.return_type, None);
    assert_eq!(world_new.params.len(), 3);
    assert_eq!(world_new.params[0].direction, Direction::In);
    assert_eq!(world_new.params[0].name, "count");
    assert_eq!(world_new.params[1].name, "size");
    assert_eq!(world_new.params[2].direction, Direction::Out);
    assert_eq!(world_new.params[2].name, "w");
    assert_eq!(world_new.params[2].ty, Type::named("World"));

    let clamp = function(&program, "clamp");
    assert_eq!(clamp.visibility, Visibility::Hidden);
    assert_eq!(clamp.return_type, Some(Type::named("f64")));
    match &clamp.body.statements.last().unwrap().kind {
        StmtKind::Return(Some(value)) => assert_eq!(kind(value), &ExprKind::Ident("v".to_string())),
        other => panic!("expected return statement, got {other:?}"),
    }

    let step = function(&program, "step");
    assert_eq!(step.visibility, Visibility::Visible);
    assert_eq!(step.params[0].direction, Direction::Inout);

    let main = function(&program, "main");
    assert_eq!(main.return_type, Some(Type::named("i32")));
}

#[test]
fn parses_alloc_move_copy_and_literals() {
    let program = reference_program();
    let world_new = function(&program, "world_new");

    match &world_new.body.statements[0].kind {
        StmtKind::Let {
            mutable,
            name,
            ty,
            value,
        } => {
            assert!(*mutable);
            assert_eq!(name, "ps");
            assert_eq!(ty, &Some(Type::slice(Type::named("Particle"))));
            assert_eq!(value, &None);
        }
        other => panic!("expected 'var ps', got {other:?}"),
    }

    match &world_new.body.statements[1].kind {
        StmtKind::Expr(Expr {
            kind:
                ExprKind::Alloc {
                    target,
                    element_type,
                    count,
                },
            ..
        }) => {
            assert_eq!(kind(target), &ExprKind::Ident("ps".to_string()));
            assert_eq!(element_type, &Type::named("Particle"));
            assert_eq!(kind(count), &ExprKind::Ident("count".to_string()));
        }
        other => panic!("expected alloc expression, got {other:?}"),
    }

    match &world_new.body.statements[3].kind {
        StmtKind::Assign {
            target,
            op: AssignOp::Assign,
            value,
        } => {
            match kind(target) {
                ExprKind::Member { base, field } => {
                    assert_eq!(kind(base), &ExprKind::Ident("w".to_string()));
                    assert_eq!(field, "particles");
                }
                other => panic!("expected member assignment target, got {other:?}"),
            }
            match kind(value) {
                ExprKind::Move(value) => {
                    assert_eq!(kind(value), &ExprKind::Ident("ps".to_string()))
                }
                other => panic!("expected move expression, got {other:?}"),
            }
        }
        other => panic!("expected move assignment, got {other:?}"),
    }

    match &world_new.body.statements[5].kind {
        StmtKind::Assign { value, .. } => match kind(value) {
            ExprKind::Copy(value) => {
                assert_eq!(kind(value), &ExprKind::Ident("size".to_string()))
            }
            other => panic!("expected copy expression, got {other:?}"),
        },
        other => panic!("expected copy assignment, got {other:?}"),
    }
}

#[test]
fn parses_for_range_and_foreach() {
    let program = reference_program();
    let world_new = function(&program, "world_new");

    match &world_new.body.statements[2].kind {
        StmtKind::ForRange {
            variable,
            start,
            end,
            body,
        } => {
            assert_eq!(variable, "i");
            assert_eq!(kind(start), &ExprKind::Int("0".to_string()));
            assert_eq!(kind(end), &ExprKind::Ident("count".to_string()));

            match &body.statements[0].kind {
                StmtKind::Assign { target, value, .. } => {
                    match kind(target) {
                        ExprKind::Index { base, index } => {
                            assert_eq!(kind(base), &ExprKind::Ident("ps".to_string()));
                            assert_eq!(kind(index), &ExprKind::Ident("i".to_string()));
                        }
                        other => panic!("expected index target, got {other:?}"),
                    }
                    match kind(value) {
                        ExprKind::InterfaceLiteral { name, fields, .. } => {
                            assert_eq!(name, "Particle");
                            assert_eq!(fields.len(), 3);
                            assert_eq!(fields[0].name, "pos");
                            assert_eq!(fields[1].name, "vel");
                            assert_eq!(fields[2].name, "mass");
                        }
                        other => panic!("expected particle literal, got {other:?}"),
                    }
                }
                other => panic!("expected particle literal assignment, got {other:?}"),
            }
        }
        other => panic!("expected for-range statement, got {other:?}"),
    }

    let step = function(&program, "step");
    match &step.body.statements[1].kind {
        StmtKind::ForEach {
            direction,
            variable,
            iterable,
            ..
        } => {
            assert_eq!(*direction, Direction::Inout);
            assert_eq!(variable, "p");
            match kind(iterable) {
                ExprKind::Member { base, field } => {
                    assert_eq!(kind(base), &ExprKind::Ident("w".to_string()));
                    assert_eq!(field, "particles");
                }
                other => panic!("expected member iterable, got {other:?}"),
            }
        }
        other => panic!("expected foreach statement, got {other:?}"),
    }
}

#[test]
fn parses_main_and_protect() {
    let program = reference_program();
    let main = function(&program, "main");

    match &main.body.statements[1].kind {
        StmtKind::Protect(block) => {
            assert_eq!(block.statements.len(), 4);
            assert!(matches!(block.statements[0].kind, StmtKind::Let { .. }));
            assert!(matches!(
                block.statements[1].kind,
                StmtKind::Expr(Expr {
                    kind: ExprKind::Call { .. },
                    ..
                })
            ));
            assert!(matches!(
                block.statements[2].kind,
                StmtKind::ForRange { .. }
            ));
            assert!(matches!(
                block.statements[3].kind,
                StmtKind::Assign {
                    op: AssignOp::Assign,
                    ..
                }
            ));
        }
        other => panic!("expected protect block, got {other:?}"),
    }
}

#[test]
fn if_header_does_not_swallow_the_block_as_a_literal() {
    let program = parse_source(
        "flow f\n\
             <- x: bool\n\
         {\n\
             if x {\n\
                 => 1\n\
             }\n\
         }",
    );
    let function = function(&program, "f");

    match &function.body.statements[0].kind {
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            assert_eq!(kind(condition), &ExprKind::Ident("x".to_string()));
            assert_eq!(then_block.statements.len(), 1);
            match &then_block.statements[0].kind {
                StmtKind::Return(Some(value)) => {
                    assert_eq!(kind(value), &ExprKind::Int("1".to_string()))
                }
                other => panic!("expected return, got {other:?}"),
            }
            assert_eq!(else_branch, &None);
        }
        other => panic!("expected if statement, got {other:?}"),
    }
}

#[test]
fn parses_while_let_and_else_if() {
    let program = parse_source(
        "flow f {\n\
             x: i32 <- 0\n\
             while x < 3 {\n\
                 x += 1\n\
             }\n\
             if x == 3 {\n\
                 => 1\n\
             } else if x > 3 {\n\
                 => 2\n\
             } else {\n\
                 => 3\n\
             }\n\
         }",
    );
    let function = function(&program, "f");

    match &function.body.statements[0].kind {
        StmtKind::Let {
            mutable,
            name,
            ty,
            value,
        } => {
            assert!(!mutable);
            assert_eq!(name, "x");
            assert_eq!(ty, &Some(Type::named("i32")));
            assert_eq!(
                value.as_ref().map(kind),
                Some(&ExprKind::Int("0".to_string()))
            );
        }
        other => panic!("expected let statement, got {other:?}"),
    }

    assert!(matches!(
        function.body.statements[1].kind,
        StmtKind::While { .. }
    ));

    match &function.body.statements[2].kind {
        StmtKind::If {
            else_branch: Some(ElseBranch::If(inner)),
            ..
        } => match &inner.kind {
            StmtKind::If {
                else_branch: Some(ElseBranch::Block(_)),
                ..
            } => {}
            other => panic!("expected final else block, got {other:?}"),
        },
        other => panic!("expected else-if chain, got {other:?}"),
    }
}

#[test]
fn reports_missing_parameter_direction() {
    let tokens = tokenize("flow f(x: i32) {}", "<test>").unwrap();
    let error = parse(tokens).unwrap_err();
    assert!(error.message.contains("parameter direction"));
}

#[test]
fn reports_missing_statement_separator() {
    let tokens = tokenize("flow f { mut x: i32 mut y: i32 }", "<test>").unwrap();
    let error = parse(tokens).unwrap_err();
    assert!(error.message.contains("expected newline"));
}

#[test]
fn both_reference_files_describe_the_same_program() {
    let reference = parse_source(include_str!("../examples/particles.loi"));
    let flow = parse_source(include_str!("../examples/flow_syntax.loi"));

    assert_eq!(format_program(&reference), format_program(&flow));
}

#[test]
fn parses_flow_syntax_declarations() {
    let program = parse_source(include_str!("../examples/flow_syntax.loi"));

    let vec2 = interface(&program, "Vec2");
    // No visibility modifier means `sealed` (hidden is the default).
    assert_eq!(vec2.visibility, Visibility::Hidden);
    assert_eq!(vec2.fields.len(), 2);

    let world_new = function(&program, "world_new");
    assert_eq!(world_new.visibility, Visibility::Visible);
    assert_eq!(world_new.return_type, None);
    let directions: Vec<Direction> = world_new.params.iter().map(|p| p.direction).collect();
    assert_eq!(
        directions,
        vec![Direction::In, Direction::In, Direction::Out]
    );
    assert_eq!(world_new.params[0].name, "count");
    assert_eq!(world_new.params[2].name, "w");

    let clamp = function(&program, "clamp");
    assert_eq!(clamp.visibility, Visibility::Hidden);
    assert_eq!(clamp.return_type, Some(Type::named("f64")));

    let step = function(&program, "step");
    assert_eq!(step.params[0].direction, Direction::Inout);

    let main = function(&program, "main");
    assert_eq!(main.return_type, Some(Type::named("i32")));

    match &world_new.body.statements[0].kind {
        StmtKind::Let {
            mutable,
            name,
            ty,
            value,
        } => {
            assert!(*mutable);
            assert_eq!(name, "ps");
            assert_eq!(ty, &Some(Type::slice(Type::named("Particle"))));
            assert_eq!(value, &None);
        }
        other => panic!("expected flow binding, got {other:?}"),
    }

    match &world_new.body.statements[1].kind {
        StmtKind::Expr(Expr {
            kind: ExprKind::Alloc { target, .. },
            ..
        }) => assert_eq!(kind(target), &ExprKind::Ident("ps".to_string())),
        other => panic!("expected alloc expression, got {other:?}"),
    }

    match &world_new.body.statements[2].kind {
        StmtKind::ForRange { variable, .. } => assert_eq!(variable, "i"),
        other => panic!("expected for-range statement, got {other:?}"),
    }

    let StmtKind::ForRange { body, .. } = &world_new.body.statements[2].kind else {
        unreachable!()
    };
    match &body.statements[0].kind {
        StmtKind::Assign { value, .. } => match kind(value) {
            ExprKind::InterfaceLiteral { name, fields, .. } => {
                assert_eq!(name, "Particle");
                assert_eq!(fields.len(), 3);
                match kind(&fields[0].value) {
                    ExprKind::InterfaceLiteral { name, .. } => assert_eq!(name, "Vec2"),
                    other => panic!("expected Vec2 shape literal, got {other:?}"),
                }
            }
            other => panic!("expected Particle shape literal, got {other:?}"),
        },
        other => panic!("expected shape literal assignment, got {other:?}"),
    }

    match &step.body.statements[1].kind {
        StmtKind::ForEach {
            direction,
            variable,
            ..
        } => {
            assert_eq!(*direction, Direction::Inout);
            assert_eq!(variable, "p");
        }
        other => panic!("expected foreach statement, got {other:?}"),
    }

    let total_mass = function(&program, "total_mass");
    match &total_mass.body.statements[0].kind {
        StmtKind::Let {
            mutable,
            name,
            value,
            ..
        } => {
            assert!(*mutable);
            assert_eq!(name, "m");
            assert_eq!(
                value.as_ref().map(kind),
                Some(&ExprKind::Int("0".to_string()))
            );
        }
        other => panic!("expected `mut m: f64 <- 0`, got {other:?}"),
    }

    // `give` and `clone` map onto the ownership AST nodes.
    match &world_new.body.statements[3].kind {
        StmtKind::Assign { value, .. } => {
            assert!(matches!(kind(value), ExprKind::Move(_)));
        }
        other => panic!("expected ownership transfer, got {other:?}"),
    }
    match &world_new.body.statements[5].kind {
        StmtKind::Assign { value, .. } => {
            assert!(matches!(kind(value), ExprKind::Copy(_)));
        }
        other => panic!("expected clone, got {other:?}"),
    }
}

#[test]
fn parses_typed_slice_literals_in_flow_syntax() {
    let program = parse_source("flow f\n    => [i32]\n{\n    => [i32: 1, 2, 3]\n}\n");
    let function = function(&program, "f");

    match &function.body.statements[0].kind {
        StmtKind::Return(Some(value)) => match kind(value) {
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                assert_eq!(element_type, &Some(Type::named("i32")));
                assert_eq!(elements.len(), 3);
            }
            other => panic!("expected typed slice literal, got {other:?}"),
        },
        other => panic!("expected return, got {other:?}"),
    }
}

#[test]
fn parses_bounded_type_params() {
    let program = parse_source(
        "shape Pair<T: Copy> [\n    first: T\n    second: T\n]\n\nflow find<T: Clone + Eq, U>\n    <- xs: [T]\n    <- needle: U\n{\n}\n",
    );

    let pair = interface(&program, "Pair");
    assert_eq!(pair.type_params.len(), 1);
    assert_eq!(pair.type_params[0].name, "T");
    assert_eq!(pair.type_params[0].bounds, vec![TypeBound::Copy]);

    let find = function(&program, "find");
    assert_eq!(find.type_params.len(), 2);
    assert_eq!(find.type_params[0].name, "T");
    assert_eq!(
        find.type_params[0].bounds,
        vec![TypeBound::Clone, TypeBound::Eq]
    );
    assert_eq!(find.type_params[1].name, "U");
    assert!(find.type_params[1].bounds.is_empty());
}

#[test]
fn parses_ord_and_rejects_unknown_type_param_bound() {
    let tokens = tokenize("flow f<T: Clone + Ord> {\n}\n", "<test>").expect("source should lex");
    let program = parse(tokens).expect("Ord is a valid constraint");
    let Item::Function(decl) = &program.items[0] else {
        panic!("expected a flow declaration");
    };
    assert_eq!(
        decl.type_params[0].bounds,
        vec![TypeBound::Clone, TypeBound::Ord]
    );

    let tokens = tokenize("flow f<T: Hash> {\n}\n", "<test>").expect("source should lex");
    let error = parse(tokens).expect_err("an unknown constraint should fail to parse");
    assert!(
        error.message.contains("unknown constraint 'Hash'"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_generic_shapes_flows_and_calls() {
    let program = parse_source(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow push<T>\n    <-> xs: Vec<T>\n    <- value: T\n{\n}\n\nflow main\n    => i32\n{\n    mut v <- Vec<i32>[ data: [i32:], len: 0 ]\n    push<i32>(<-> v, 7)\n    => 0\n}\n",
    );

    let vec = interface(&program, "Vec");
    assert_eq!(vec.type_params.len(), 1);
    assert_eq!(vec.type_params[0].name, "T");
    assert!(vec.type_params[0].bounds.is_empty());
    assert_eq!(vec.fields[0].ty, Type::slice(Type::named("T")));

    let push = function(&program, "push");
    assert_eq!(push.type_params.len(), 1);
    assert_eq!(push.type_params[0].name, "T");
    assert!(push.type_params[0].bounds.is_empty());
    assert_eq!(
        push.params[0].ty,
        Type::generic("Vec", vec![Type::named("T")])
    );

    let main = function(&program, "main");
    match &main.body.statements[0].kind {
        StmtKind::Let {
            value: Some(value), ..
        } => match kind(value) {
            ExprKind::InterfaceLiteral {
                name,
                type_args,
                fields,
            } => {
                assert_eq!(name, "Vec");
                assert_eq!(type_args, &vec![Type::named("i32")]);
                assert_eq!(fields.len(), 2);
            }
            other => panic!("expected generic shape literal, got {other:?}"),
        },
        other => panic!("expected let statement, got {other:?}"),
    }

    match &main.body.statements[1].kind {
        StmtKind::Expr(expr) => match kind(expr) {
            ExprKind::Call {
                type_args, args, ..
            } => {
                assert_eq!(type_args, &vec![Type::named("i32")]);
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected generic call, got {other:?}"),
        },
        other => panic!("expected expression statement, got {other:?}"),
    }
}

#[test]
fn parses_nested_generic_types_with_shift_tokens() {
    let program = parse_source(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow f\n    <- xs: Vec<Vec<i32>>\n{\n}\n",
    );

    let f = function(&program, "f");
    assert_eq!(
        f.params[0].ty,
        Type::generic("Vec", vec![Type::generic("Vec", vec![Type::named("i32")])])
    );
}

#[test]
fn parses_comptime_flows_and_expressions() {
    let program = parse_source(
        "comptime flow twice\n    <- x: i32\n    => i32\n{\n    => x * 2\n}\n\nflow main\n    => i32\n{\n    => comptime twice(21)\n}\n",
    );

    let twice = function(&program, "twice");
    assert!(twice.comptime);

    let main = function(&program, "main");
    match &main.body.statements[0].kind {
        StmtKind::Return(Some(value)) => match kind(value) {
            ExprKind::Comptime(inner) => {
                assert!(matches!(kind(inner), ExprKind::Call { .. }));
            }
            other => panic!("expected comptime expression, got {other:?}"),
        },
        other => panic!("expected return, got {other:?}"),
    }
}

#[test]
fn parses_nested_generic_call_arguments_after_a_shift_split() {
    let program = parse_source(
        "shape Vec<T> [\n    data: [T]\n    len: u64\n]\n\nflow id<T>\n    <- x: T\n    => u64\n{\n    => 0\n}\n\nflow main\n    => i32\n{\n    v <- Vec<Vec<i32>>[ data: [Vec<i32>:], len: 0 ]\n    w <- id<Vec<Vec<i32>>>(v)\n    => w as i32\n}\n",
    );

    let main = function(&program, "main");
    let StmtKind::Let {
        value: Some(value), ..
    } = &main.body.statements[1].kind
    else {
        panic!("expected let statement");
    };
    match kind(value) {
        ExprKind::Call { type_args, .. } => {
            assert_eq!(
                type_args,
                &vec![Type::generic(
                    "Vec",
                    vec![Type::generic("Vec", vec![Type::named("i32")])]
                )]
            );
        }
        other => panic!("expected generic call, got {other:?}"),
    }
}

#[test]
fn comparisons_and_shifts_do_not_consume_generic_angle_brackets() {
    let program = parse_source(
        "flow f\n    <- a: i32\n    <- b: i32\n    <- x: i32\n    <- y: i32\n    <- z: i32\n    <- w: i32\n    => bool\n{\n    r <- a >> 1\n    s <- a << 2\n    => a < b and x < y and z > w and r > s\n}\n",
    );

    let f = function(&program, "f");
    let StmtKind::Let {
        value: Some(value), ..
    } = &f.body.statements[0].kind
    else {
        panic!("expected shift binding");
    };
    assert!(matches!(
        kind(value),
        ExprKind::Binary {
            op: BinaryOp::Shr,
            ..
        }
    ));

    let StmtKind::Return(Some(value)) = &f.body.statements[2].kind else {
        panic!("expected return");
    };
    // The top-level `and` chain keeps the comparisons as nested binaries.
    let mut seen = Vec::new();
    collect_binary_ops(value, &mut seen);
    for op in [BinaryOp::Lt, BinaryOp::Gt] {
        assert!(seen.contains(&op), "missing {op:?} in {seen:?}");
    }
}

#[test]
fn parses_mark_and_choice_declarations() {
    let program =
        parse_source("mark Color [ Red, Green, Blue ]\nchoice Result [ Ok: i32, Err: i64 ]\n");

    let Item::Mark(mark) = &program.items[0] else {
        panic!("expected a mark declaration");
    };
    assert_eq!(mark.name, "Color");
    assert_eq!(mark.visibility, Visibility::Hidden);
    assert_eq!(
        mark.variants
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Red", "Green", "Blue"]
    );

    let Item::Choice(choice) = &program.items[1] else {
        panic!("expected a choice declaration");
    };
    assert_eq!(choice.name, "Result");
    assert_eq!(choice.variants.len(), 2);
    assert_eq!(choice.variants[0].name, "Ok");
    assert_eq!(choice.variants[0].ty, Type::named("i32"));
    assert_eq!(choice.variants[1].name, "Err");
    assert_eq!(choice.variants[1].ty, Type::named("i64"));
}

#[test]
fn parses_brace_mark_and_choice_declarations() {
    let program =
        parse_source("mark Color { Red, Green, Blue }\nchoice Result { Ok: i32, Err: i64 }\n");

    let Item::Mark(mark) = &program.items[0] else {
        panic!("expected a mark declaration");
    };
    assert_eq!(mark.name, "Color");
    assert_eq!(
        mark.variants
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Red", "Green", "Blue"]
    );

    let Item::Choice(choice) = &program.items[1] else {
        panic!("expected a choice declaration");
    };
    assert_eq!(choice.name, "Result");
    assert_eq!(choice.variants.len(), 2);
    assert_eq!(choice.variants[0].name, "Ok");
    assert_eq!(choice.variants[0].ty, Type::named("i32"));
    assert_eq!(choice.variants[1].name, "Err");
    assert_eq!(choice.variants[1].ty, Type::named("i64"));
}

#[test]
fn parses_mark_and_choice_with_newlines_and_visibility() {
    let program = parse_source(
        "open mark Flag [\n    On\n    Off\n]\nsealed choice Maybe [\n    Some: i32,\n    None: bool,\n]\n",
    );
    let Item::Mark(mark) = &program.items[0] else {
        panic!("expected mark");
    };
    assert_eq!(mark.visibility, Visibility::Visible);
    assert_eq!(mark.variants.len(), 2);
    let Item::Choice(choice) = &program.items[1] else {
        panic!("expected choice");
    };
    assert_eq!(choice.visibility, Visibility::Hidden);
    assert_eq!(choice.variants.len(), 2);
}

#[test]
fn parses_variant_reference_constructor_and_is() {
    let program = parse_source(
        "flow main {\n    c <- Color.Green\n    r <- Result.Ok(42)\n    if r is Result.Ok {\n        print_int(r.Ok)\n    }\n}\n",
    );
    let main = function(&program, "main");

    let StmtKind::Let {
        value: Some(value), ..
    } = &main.body.statements[0].kind
    else {
        panic!("expected binding");
    };
    assert!(matches!(
        kind(value),
        ExprKind::Member { field, .. } if field == "Green"
    ));

    let StmtKind::Let {
        value: Some(value), ..
    } = &main.body.statements[1].kind
    else {
        panic!("expected binding");
    };
    assert!(matches!(kind(value), ExprKind::Call { .. }));

    let StmtKind::If { condition, .. } = &main.body.statements[2].kind else {
        panic!("expected if");
    };
    assert!(matches!(
        kind(condition),
        ExprKind::Binary {
            op: BinaryOp::Is,
            ..
        }
    ));
}

#[test]
fn parses_import_aliases_and_qualified_names() {
    let program = parse_source(
        r#"import "lib/math.loi" as m
import "lib/geo.loi"

flow main
    <- p: m::Point
    => i32
{
    v <- m::math_sum<i32>([i32: 1, 2])
    q <- m::Point[ x: 1 ]
    if v != geo::origin { => 1 }
    => v
}
"#,
    );

    let Item::Import(first) = &program.items[0] else {
        panic!("expected an import");
    };
    assert_eq!(first.path, "lib/math.loi");
    assert_eq!(first.alias.as_deref(), Some("m"));

    let Item::Import(second) = &program.items[1] else {
        panic!("expected a second import");
    };
    assert_eq!(second.path, "lib/geo.loi");
    assert_eq!(second.alias, None);

    let Item::Function(main) = &program.items[2] else {
        panic!("expected main");
    };
    assert_eq!(main.name, "main");
    assert_eq!(main.params[0].ty, Type::named("m::Point"));

    let StmtKind::Let {
        value: Some(value), ..
    } = &main.body.statements[0].kind
    else {
        panic!("expected the call binding");
    };
    let ExprKind::Call { callee, .. } = kind(value) else {
        panic!("expected a call");
    };
    assert_eq!(kind(callee), &ExprKind::Ident("m::math_sum".to_string()));

    let StmtKind::Let {
        value: Some(value), ..
    } = &main.body.statements[1].kind
    else {
        panic!("expected the shape literal");
    };
    match kind(value) {
        ExprKind::InterfaceLiteral { name, .. } => assert_eq!(name, "m::Point"),
        other => panic!("expected a shape literal, got {other:?}"),
    }

    let StmtKind::If { condition, .. } = &main.body.statements[2].kind else {
        panic!("expected an if");
    };
    assert!(format!("{:?}", kind(condition)).contains("geo::origin"));
}

fn collect_binary_ops(expr: &Expr, ops: &mut Vec<BinaryOp>) {
    if let ExprKind::Binary { op, left, right } = kind(expr) {
        ops.push(*op);
        collect_binary_ops(left, ops);
        collect_binary_ops(right, ops);
    }
}
