use loi::ast::*;
use loi::{format_program, parse, tokenize};

fn parse_source(source: &str) -> Program {
    let tokens = tokenize(source, "<test>").expect("source should lex");
    parse(tokens).expect("source should parse")
}

fn reference_program() -> Program {
    parse_source(include_str!("../examples/particles.loi"))
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
    assert_eq!(
        global.ty,
        Some(Type::Slice(Box::new(Type::Named("u8".to_string()))))
    );
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
    assert_eq!(element_type, &Some(Type::Named("i32".to_string())));
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
    assert_eq!(vec2.fields[0].ty, Type::Named("f64".to_string()));

    let particle = interface(&program, "Particle");
    assert_eq!(particle.fields.len(), 4);
    let alive = &particle.fields[3];
    assert_eq!(alive.name, "alive");
    assert_eq!(alive.ty, Type::Named("bool".to_string()));
    assert_eq!(
        alive.default.as_ref().map(kind),
        Some(&ExprKind::Bool(true))
    );

    let world = interface(&program, "World");
    assert_eq!(
        world.fields[0].ty,
        Type::Slice(Box::new(Type::Named("Particle".to_string())))
    );
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
    assert_eq!(world_new.params[2].ty, Type::Named("World".to_string()));

    let clamp = function(&program, "clamp");
    assert_eq!(clamp.visibility, Visibility::Hidden);
    assert_eq!(clamp.return_type, Some(Type::Named("f64".to_string())));
    match &clamp.body.statements.last().unwrap().kind {
        StmtKind::Return(Some(value)) => assert_eq!(kind(value), &ExprKind::Ident("v".to_string())),
        other => panic!("expected return statement, got {other:?}"),
    }

    let step = function(&program, "step");
    assert_eq!(step.visibility, Visibility::Visible);
    assert_eq!(step.params[0].direction, Direction::Inout);

    let main = function(&program, "main");
    assert_eq!(main.return_type, Some(Type::Named("i32".to_string())));
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
            assert_eq!(
                ty,
                &Some(Type::Slice(Box::new(Type::Named("Particle".to_string()))))
            );
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
            assert_eq!(element_type, &Type::Named("Particle".to_string()));
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
                        ExprKind::InterfaceLiteral { name, fields } => {
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
            assert_eq!(ty, &Some(Type::Named("i32".to_string())));
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
    assert_eq!(clamp.return_type, Some(Type::Named("f64".to_string())));

    let step = function(&program, "step");
    assert_eq!(step.params[0].direction, Direction::Inout);

    let main = function(&program, "main");
    assert_eq!(main.return_type, Some(Type::Named("i32".to_string())));

    match &world_new.body.statements[0].kind {
        StmtKind::Let {
            mutable,
            name,
            ty,
            value,
        } => {
            assert!(*mutable);
            assert_eq!(name, "ps");
            assert_eq!(
                ty,
                &Some(Type::Slice(Box::new(Type::Named("Particle".to_string()))))
            );
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
            ExprKind::InterfaceLiteral { name, fields } => {
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
                assert_eq!(element_type, &Some(Type::Named("i32".to_string())));
                assert_eq!(elements.len(), 3);
            }
            other => panic!("expected typed slice literal, got {other:?}"),
        },
        other => panic!("expected return, got {other:?}"),
    }
}
