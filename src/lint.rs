//! Source-level warnings that do not make a program invalid.
//!
//! Warnings are produced from the original AST, before monomorphization, so
//! their spans and names are exactly what the user wrote. The compiler driver
//! still prints them through the usual diagnostic renderer; the LSP maps
//! `warning` to LSP severity 2.

use std::collections::HashSet;

use crate::ast::{Block, ElseBranch, Expr, ExprKind, Item, Program, Span, Stmt, StmtKind};
use crate::diagnostic::Diagnostic;

/// A binding that can be reported as unused. Parameters keep their own span;
/// loop and guard bindings reuse the span of the enclosing statement.
struct Binding {
    name: String,
    span: Span,
    kind: &'static str,
}

/// Collect all warnings for `program`.
pub fn warnings(program: &Program) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for item in &program.items {
        let Item::Function(decl) = item else {
            continue;
        };
        // `extern flow` declarations have no body; their parameters exist only
        // to describe the foreign ABI.
        if decl.external {
            continue;
        }

        let mut bindings = Vec::new();
        collect_bindings(&decl.body, &mut bindings);

        let mut used = HashSet::new();
        collect_uses(&decl.body, &mut used);

        for binding in bindings {
            if !used.contains(&binding.name) {
                out.push(Diagnostic::warning(
                    format!("unused {} '{}'", binding.kind, binding.name),
                    binding.span,
                ));
            }
        }
    }
    out
}

/// Declarations introduced by a block, including nested blocks.
fn collect_bindings(block: &Block, out: &mut Vec<Binding>) {
    for statement in &block.statements {
        collect_statement_bindings(statement, out);
    }
}

fn collect_statement_bindings(statement: &Stmt, out: &mut Vec<Binding>) {
    match &statement.kind {
        StmtKind::Let { name, value, .. } => {
            if name != "_" {
                out.push(Binding {
                    name: name.clone(),
                    span: statement.span,
                    kind: "variable",
                });
            }
            if let Some(value) = value {
                collect_expr_uses(value, &mut HashSet::new());
            }
        }
        StmtKind::Assign { target, value, .. } => {
            // Bindings cannot be introduced by an assignment.
            let _ = (target, value);
        }
        StmtKind::Expr(expr) => {
            let _ = expr;
        }
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            let _ = condition;
            collect_bindings(then_block, out);
            match else_branch {
                Some(ElseBranch::Block(block)) => collect_bindings(block, out),
                Some(ElseBranch::If(stmt)) => collect_statement_bindings(stmt, out),
                None => {}
            }
        }
        StmtKind::While { body, .. } => collect_bindings(body, out),
        StmtKind::ForRange { variable, body, .. } => {
            if variable != "_" {
                out.push(Binding {
                    name: variable.clone(),
                    span: statement.span,
                    kind: "loop variable",
                });
            }
            collect_bindings(body, out);
        }
        StmtKind::ForEach { variable, body, .. } => {
            if variable != "_" {
                out.push(Binding {
                    name: variable.clone(),
                    span: statement.span,
                    kind: "loop variable",
                });
            }
            collect_bindings(body, out);
        }
        StmtKind::Protect(block) => collect_bindings(block, out),
        StmtKind::Guard {
            name, value, body, ..
        } => {
            if name != "_" {
                out.push(Binding {
                    name: name.clone(),
                    span: statement.span,
                    kind: "guard binding",
                });
            }
            let _ = value;
            collect_bindings(body, out);
        }
        StmtKind::Return(_) | StmtKind::Continue | StmtKind::Break => {}
    }
}

/// Every identifier read anywhere in a block.
fn collect_uses(block: &Block, out: &mut HashSet<String>) {
    for statement in &block.statements {
        collect_statement_uses(statement, out);
    }
}

fn collect_statement_uses(statement: &Stmt, out: &mut HashSet<String>) {
    match &statement.kind {
        StmtKind::Let { value, .. } => {
            if let Some(value) = value {
                collect_expr_uses(value, out);
            }
        }
        StmtKind::Assign { target, op, value } => {
            // A plain `x = ...` only writes `x`. Compound assignments also
            // read it, so treat all other targets as uses.
            if !matches!(op, crate::ast::AssignOp::Assign)
                || !matches!(target.kind, ExprKind::Ident(_))
            {
                collect_expr_uses(target, out);
            }
            collect_expr_uses(value, out);
        }
        StmtKind::Expr(expr) => collect_expr_uses(expr, out),
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            collect_expr_uses(condition, out);
            collect_uses(then_block, out);
            match else_branch {
                Some(ElseBranch::Block(block)) => collect_uses(block, out),
                Some(ElseBranch::If(stmt)) => collect_statement_uses(stmt, out),
                None => {}
            }
        }
        StmtKind::While { condition, body } => {
            collect_expr_uses(condition, out);
            collect_uses(body, out);
        }
        StmtKind::ForRange {
            start, end, body, ..
        } => {
            collect_expr_uses(start, out);
            collect_expr_uses(end, out);
            collect_uses(body, out);
        }
        StmtKind::ForEach { iterable, body, .. } => {
            collect_expr_uses(iterable, out);
            collect_uses(body, out);
        }
        StmtKind::Protect(block) => collect_uses(block, out),
        StmtKind::Guard { value, body, .. } => {
            collect_expr_uses(value, out);
            collect_uses(body, out);
        }
        StmtKind::Return(value) => {
            if let Some(value) = value {
                collect_expr_uses(value, out);
            }
        }
        StmtKind::Continue | StmtKind::Break => {}
    }
}

/// Every value identifier read in an expression. A call's callee and a literal's
/// type name are declarations, not variable reads, so they are skipped.
fn collect_expr_uses(expr: &Expr, out: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {}
        ExprKind::Ident(name) => {
            out.insert(name.clone());
        }
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                collect_expr_uses(element, out);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                collect_expr_uses(&field.value, out);
            }
        }
        ExprKind::Unary { operand, .. } => collect_expr_uses(operand, out),
        ExprKind::Binary { left, right, .. } => {
            collect_expr_uses(left, out);
            collect_expr_uses(right, out);
        }
        ExprKind::Cast { expr, .. } => collect_expr_uses(expr, out),
        ExprKind::Call { args, .. } => {
            // Every argument names a binding; treating `-> x` as a use errs on
            // the quiet side and avoids false positives for out-parameters.
            for arg in args {
                collect_expr_uses(&arg.value, out);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            // `alloc(-> target, T, count)` writes `target`.
            if !matches!(target.kind, ExprKind::Ident(_)) {
                collect_expr_uses(target, out);
            }
            collect_expr_uses(count, out);
        }
        ExprKind::Move(inner) | ExprKind::Copy(inner) | ExprKind::Comptime(inner) => {
            collect_expr_uses(inner, out)
        }
        ExprKind::Member { base, .. } => collect_expr_uses(base, out),
        ExprKind::Index { base, index } => {
            collect_expr_uses(base, out);
            collect_expr_uses(index, out);
        }
    }
}
