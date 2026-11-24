//! Expansion of code-as-value constructs (`quote`, `unquote`, `splice`).
//!
//! `quote` and `unquote` produce a comptime `Code` value, and `splice` inserts
//! the statements of such a value into a flow. Expansion runs before type
//! checking and before either backend, so the rest of the compiler only ever
//! sees ordinary statements and expressions.

use crate::ast::*;
use crate::const_eval::{ConstEvaluator, ConstValue};
use crate::diagnostic::Diagnostic;
use crate::types::Ty;

const SPLICE_TYPE_ERROR: &str = "'splice' expects a comptime expression of type 'Code'";

/// A comptime flow that yields `Code` exists only to feed `splice`. It has no
/// runtime body, so the backends must not emit it. Other comptime flows may be
/// called at runtime or from a constant expression and must still be emitted.
pub fn is_code_template(decl: &FunctionDecl) -> bool {
    let declared_code = matches!(&decl.return_type, Some(Type::Named(name, _)) if name == "Code");
    declared_code || block_contains_quote(&decl.body)
}

fn block_contains_quote(block: &Block) -> bool {
    block.statements.iter().any(stmt_contains_quote)
}

fn stmt_contains_quote(statement: &Stmt) -> bool {
    match &statement.kind {
        StmtKind::Let { value, .. } => value.as_ref().is_some_and(expr_contains_quote),
        StmtKind::Assign { target, value, .. } => {
            expr_contains_quote(target) || expr_contains_quote(value)
        }
        StmtKind::Expr(expr) | StmtKind::Splice(expr) => expr_contains_quote(expr),
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            expr_contains_quote(condition)
                || block_contains_quote(then_block)
                || match else_branch {
                    Some(ElseBranch::Block(block)) => block_contains_quote(block),
                    Some(ElseBranch::If(stmt)) => stmt_contains_quote(stmt),
                    None => false,
                }
        }
        StmtKind::While { condition, body } => {
            expr_contains_quote(condition) || block_contains_quote(body)
        }
        StmtKind::ForRange {
            start, end, body, ..
        } => expr_contains_quote(start) || expr_contains_quote(end) || block_contains_quote(body),
        StmtKind::ForEach { iterable, body, .. } => {
            expr_contains_quote(iterable) || block_contains_quote(body)
        }
        StmtKind::Protect(body) => block_contains_quote(body),
        StmtKind::Guard { value, body, .. } => {
            expr_contains_quote(value) || block_contains_quote(body)
        }
        StmtKind::Return(value) => value.as_ref().is_some_and(expr_contains_quote),
        StmtKind::Continue | StmtKind::Break => false,
    }
}

fn expr_contains_quote(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Quote(_) => true,
        ExprKind::SliceLiteral { elements, .. } => elements.iter().any(expr_contains_quote),
        ExprKind::InterfaceLiteral { fields, .. } => {
            fields.iter().any(|field| expr_contains_quote(&field.value))
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Cast { expr: operand, .. }
        | ExprKind::Move(operand)
        | ExprKind::Copy(operand)
        | ExprKind::Comptime(operand)
        | ExprKind::Member { base: operand, .. }
        | ExprKind::Unquote(operand) => expr_contains_quote(operand),
        ExprKind::Binary { left, right, .. } => {
            expr_contains_quote(left) || expr_contains_quote(right)
        }
        ExprKind::Call { callee, args, .. } => {
            expr_contains_quote(callee) || args.iter().any(|arg| expr_contains_quote(&arg.value))
        }
        ExprKind::Alloc { target, count, .. } => {
            expr_contains_quote(target) || expr_contains_quote(count)
        }
        ExprKind::Index { base, index } => expr_contains_quote(base) || expr_contains_quote(index),
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => false,
    }
}

/// Expand every `splice` in a program, returning diagnostics instead of a
/// partially rewritten program when a construct is used incorrectly.
pub fn expand_program(program: &Program) -> Result<Program, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    validate_program(program, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut evaluator = ConstEvaluator::new(program);
    for item in &program.items {
        if let Item::Global(decl) = item {
            let _ = evaluator.eval_global(decl);
        }
    }

    let mut expanded = program.clone();
    let mut errors = Vec::new();
    for item in &mut expanded.items {
        if let Item::Function(decl) = item {
            decl.body = expand_block(&mut evaluator, &decl.body, &mut errors);
        }
    }
    if errors.is_empty() {
        Ok(expanded)
    } else {
        Err(errors)
    }
}

/// Validate placement of the new nodes before evaluating anything.
fn validate_program(program: &Program, errors: &mut Vec<Diagnostic>) {
    for item in &program.items {
        match item {
            Item::Function(decl) => validate_block(&decl.body, decl.comptime, false, errors),
            // Globals are evaluated before any flow runs. Their initializers
            // must obey the same placement rules as ordinary statements;
            // otherwise a `Code` value can reach codegen, which has no C
            // representation for it.
            Item::Global(decl) => validate_expr(&decl.value, false, false, errors),
            _ => {}
        }
    }
}

fn validate_block(block: &Block, in_comptime: bool, in_quote: bool, errors: &mut Vec<Diagnostic>) {
    for statement in &block.statements {
        match &statement.kind {
            StmtKind::Let { value, .. } => {
                if let Some(value) = value {
                    validate_expr(value, in_comptime, in_quote, errors);
                }
            }
            StmtKind::Assign { target, value, .. } => {
                validate_expr(target, in_comptime, in_quote, errors);
                validate_expr(value, in_comptime, in_quote, errors);
            }
            StmtKind::Expr(expr) | StmtKind::Splice(expr) => {
                validate_expr(expr, in_comptime, in_quote, errors)
            }
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                validate_expr(condition, in_comptime, in_quote, errors);
                validate_block(then_block, in_comptime, in_quote, errors);
                match else_branch {
                    Some(ElseBranch::Block(block)) => {
                        validate_block(block, in_comptime, in_quote, errors)
                    }
                    Some(ElseBranch::If(stmt)) => {
                        if let StmtKind::If { .. } = &stmt.kind {
                            validate_block(
                                &Block {
                                    statements: vec![(**stmt).clone()],
                                    open_line: stmt.span.line,
                                    close_line: stmt.span.line,
                                },
                                in_comptime,
                                in_quote,
                                errors,
                            );
                        }
                    }
                    None => {}
                }
            }
            StmtKind::While { condition, body } => {
                validate_expr(condition, in_comptime, in_quote, errors);
                validate_block(body, in_comptime, in_quote, errors);
            }
            StmtKind::ForRange {
                start, end, body, ..
            } => {
                validate_expr(start, in_comptime, in_quote, errors);
                validate_expr(end, in_comptime, in_quote, errors);
                validate_block(body, in_comptime, in_quote, errors);
            }
            StmtKind::ForEach { iterable, body, .. } => {
                validate_expr(iterable, in_comptime, in_quote, errors);
                validate_block(body, in_comptime, in_quote, errors);
            }
            StmtKind::Protect(body) => validate_block(body, in_comptime, in_quote, errors),
            StmtKind::Guard { value, body, .. } => {
                validate_expr(value, in_comptime, in_quote, errors);
                validate_block(body, in_comptime, in_quote, errors);
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    validate_expr(value, in_comptime, in_quote, errors);
                }
            }
            StmtKind::Continue | StmtKind::Break => {}
        }
    }
}

fn validate_expr(expr: &Expr, in_comptime: bool, in_quote: bool, errors: &mut Vec<Diagnostic>) {
    match &expr.kind {
        ExprKind::Quote(block) => {
            if !in_comptime {
                errors.push(
                    Diagnostic::error("'quote' is only allowed in a comptime flow", expr.span)
                        .with_help("declare the enclosing flow with `comptime flow`"),
                );
            }
            validate_block(block, in_comptime, true, errors);
        }
        ExprKind::Unquote(inner) => {
            if !in_quote {
                errors.push(
                    Diagnostic::error("'unquote' is only valid inside 'quote'", expr.span)
                        .with_help("`unquote` injects a value into an enclosing `quote { ... }`"),
                );
            }
            validate_expr(inner, in_comptime, in_quote, errors);
        }
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                validate_expr(element, in_comptime, in_quote, errors);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                validate_expr(&field.value, in_comptime, in_quote, errors);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Cast { expr: operand, .. }
        | ExprKind::Move(operand)
        | ExprKind::Copy(operand)
        | ExprKind::Comptime(operand)
        | ExprKind::Member { base: operand, .. } => {
            validate_expr(operand, in_comptime, in_quote, errors)
        }
        ExprKind::Binary { left, right, .. } => {
            validate_expr(left, in_comptime, in_quote, errors);
            validate_expr(right, in_comptime, in_quote, errors);
        }
        ExprKind::Call { callee, args, .. } => {
            validate_expr(callee, in_comptime, in_quote, errors);
            for arg in args {
                validate_expr(&arg.value, in_comptime, in_quote, errors);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            validate_expr(target, in_comptime, in_quote, errors);
            validate_expr(count, in_comptime, in_quote, errors);
        }
        ExprKind::Index { base, index } => {
            validate_expr(base, in_comptime, in_quote, errors);
            validate_expr(index, in_comptime, in_quote, errors);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => {}
    }
}

fn expand_block(
    evaluator: &mut ConstEvaluator<'_>,
    block: &Block,
    errors: &mut Vec<Diagnostic>,
) -> Block {
    let mut statements = Vec::new();
    for statement in &block.statements {
        if let StmtKind::Splice(expr) = &statement.kind {
            match evaluator.eval_expr(expr, Some(&Ty::Code)) {
                Ok(ConstValue::Code(code)) => {
                    let expanded = expand_block(evaluator, &code, errors);
                    statements.extend(expanded.statements);
                }
                Ok(_) => errors.push(
                    Diagnostic::error(SPLICE_TYPE_ERROR, expr.span)
                        .with_help("call a `comptime flow ... => Code` or use `quote { ... }`"),
                ),
                Err(error) => errors.push(
                    Diagnostic::error(SPLICE_TYPE_ERROR, expr.span)
                        .with_help(error.message)
                        .with_note("the expression must be evaluable at compile time"),
                ),
            }
            continue;
        }
        statements.push(expand_stmt(evaluator, statement, errors));
    }
    Block {
        statements,
        open_line: block.open_line,
        close_line: block.close_line,
    }
}

fn expand_stmt(
    evaluator: &mut ConstEvaluator<'_>,
    statement: &Stmt,
    errors: &mut Vec<Diagnostic>,
) -> Stmt {
    let mut result = statement.clone();
    match &mut result.kind {
        StmtKind::Let { value, .. } => {
            if let Some(value) = value {
                expand_expr(evaluator, value, errors);
            }
        }
        StmtKind::Assign { target, value, .. } => {
            expand_expr(evaluator, target, errors);
            expand_expr(evaluator, value, errors);
        }
        StmtKind::Expr(expr) => expand_expr(evaluator, expr, errors),
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            expand_expr(evaluator, condition, errors);
            *then_block = expand_block(evaluator, then_block, errors);
            match else_branch {
                Some(ElseBranch::Block(block)) => {
                    *block = expand_block(evaluator, block, errors);
                }
                Some(ElseBranch::If(stmt)) => {
                    let expanded = expand_stmt(evaluator, stmt, errors);
                    **stmt = expanded;
                }
                None => {}
            }
        }
        StmtKind::While { condition, body } => {
            expand_expr(evaluator, condition, errors);
            *body = expand_block(evaluator, body, errors);
        }
        StmtKind::ForRange {
            start, end, body, ..
        } => {
            expand_expr(evaluator, start, errors);
            expand_expr(evaluator, end, errors);
            *body = expand_block(evaluator, body, errors);
        }
        StmtKind::ForEach { iterable, body, .. } => {
            expand_expr(evaluator, iterable, errors);
            *body = expand_block(evaluator, body, errors);
        }
        StmtKind::Protect(body) => *body = expand_block(evaluator, body, errors),
        StmtKind::Guard { value, body, .. } => {
            expand_expr(evaluator, value, errors);
            *body = expand_block(evaluator, body, errors);
        }
        StmtKind::Return(value) => {
            if let Some(value) = value {
                expand_expr(evaluator, value, errors);
            }
        }
        StmtKind::Splice(_) | StmtKind::Continue | StmtKind::Break => {}
    }
    result
}

fn expand_expr(evaluator: &mut ConstEvaluator<'_>, expr: &mut Expr, errors: &mut Vec<Diagnostic>) {
    match &mut expr.kind {
        ExprKind::Quote(block) => *block = expand_block(evaluator, block, errors),
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                expand_expr(evaluator, element, errors);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                expand_expr(evaluator, &mut field.value, errors);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Cast { expr: operand, .. }
        | ExprKind::Move(operand)
        | ExprKind::Copy(operand)
        | ExprKind::Comptime(operand)
        | ExprKind::Unquote(operand) => expand_expr(evaluator, operand, errors),
        ExprKind::Binary { left, right, .. } => {
            expand_expr(evaluator, left, errors);
            expand_expr(evaluator, right, errors);
        }
        ExprKind::Call { callee, args, .. } => {
            expand_expr(evaluator, callee, errors);
            for arg in args {
                expand_expr(evaluator, &mut arg.value, errors);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            expand_expr(evaluator, target, errors);
            expand_expr(evaluator, count, errors);
        }
        ExprKind::Member { base, .. } => expand_expr(evaluator, base, errors),
        ExprKind::Index { base, index } => {
            expand_expr(evaluator, base, errors);
            expand_expr(evaluator, index, errors);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => {}
    }
}
