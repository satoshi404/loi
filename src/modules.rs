//! Module name resolution and visibility enforcement.
//!
//! The driver loads each source file as a module and flattens the items into
//! one program, as it did before modules existed. This pass runs on each
//! module's own items *before* monomorphization: it rewrites `module::item`
//! references to the plain name the rest of the compiler expects and rejects
//! references to `sealed` declarations from another module. Rewriting here
//! keeps the checker, C backend, headers and ABI unaware of namespaces.

use std::collections::HashMap;

use crate::ast::*;
use crate::diagnostic::Diagnostic;

/// Where a declaration was declared, for sealed-reference diagnostics.
#[derive(Debug, Clone)]
pub struct ModuleExport {
    pub module: usize,
    pub name: String,
    pub visibility: Visibility,
    pub span: Span,
}

/// The names a module can use in qualified references. Every module can refer
/// to itself by its own name; imports add the aliases written in that file.
#[derive(Debug, Clone)]
pub struct ModuleView {
    pub name: String,
    pub aliases: HashMap<String, usize>,
}

/// All declarations, keyed by their unqualified name. A name declared in more
/// than one module is left to the checker's existing duplicate-name diagnostic.
#[derive(Debug, Clone, Default)]
pub struct ResolutionContext {
    pub exports: HashMap<String, Vec<ModuleExport>>,
}

struct Resolver<'a> {
    module: usize,
    views: &'a [ModuleView],
    context: &'a ResolutionContext,
    diagnostics: Vec<Diagnostic>,
}

impl Resolver<'_> {
    /// Resolve one reference. Qualified names are rewritten to their item
    /// part; unqualified names are checked for cross-module sealed access.
    fn name(&mut self, name: &str, span: Span) -> String {
        if let Some((alias, item)) = name.split_once("::") {
            if item.contains("::") {
                self.diagnostics.push(
                    Diagnostic::from_error(
                        format!("nested module path '{name}' is not supported; use one 'module::item' level"),
                        span,
                    )
                    .with_label("unsupported qualified name"),
                );
                return item.rsplit("::").next().unwrap_or(item).to_string();
            }
            let Some(&target) = self.views[self.module].aliases.get(alias) else {
                self.diagnostics.push(
                    Diagnostic::from_error(format!("unknown module '{alias}'"), span)
                        .with_label("no module with this name is imported here")
                        .with_help(format!("add an `import \"...\" as {alias}` declaration")),
                );
                return item.to_string();
            };
            let declaration = self
                .context
                .exports
                .get(item)
                .and_then(|entries| entries.iter().find(|entry| entry.module == target));
            let Some(declaration) = declaration else {
                self.diagnostics.push(
                    Diagnostic::from_error(format!("module '{alias}' has no item '{item}'"), span)
                        .with_label("unknown item in this module"),
                );
                return item.to_string();
            };
            if declaration.visibility == Visibility::Hidden && target != self.module {
                self.diagnostics.push(
                    Diagnostic::from_error(
                        format!(
                            "item '{item}' is sealed in module '{alias}' and cannot be used here"
                        ),
                        span,
                    )
                    .with_label("sealed item used here")
                    .with_secondary(declaration.span, "sealed here"),
                );
            }
            return item.to_string();
        }

        if let Some(entries) = self.context.exports.get(name)
            && entries.len() == 1
        {
            let declaration = &entries[0];
            if declaration.module != self.module && declaration.visibility == Visibility::Hidden {
                self.diagnostics.push(
                    Diagnostic::from_error(
                        format!(
                            "'{name}' is sealed in module '{}' and cannot be used from module '{}'",
                            self.views[declaration.module].name, self.views[self.module].name
                        ),
                        span,
                    )
                    .with_label("sealed item used here")
                    .with_secondary(declaration.span, "sealed here"),
                );
            }
        }
        name.to_string()
    }

    fn ty(&mut self, ty: &mut Type) {
        match ty {
            Type::Named(name, span) => {
                let resolved = self.name(name, *span);
                *name = resolved;
            }
            Type::Generic(name, args, span) => {
                let resolved = self.name(name, *span);
                *name = resolved;
                for arg in args {
                    self.ty(arg);
                }
            }
            Type::Slice(element, _) => self.ty(element),
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        match &mut expr.kind {
            ExprKind::Ident(name) => {
                let resolved = self.name(name, expr.span);
                *name = resolved;
            }
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                if let Some(element_type) = element_type {
                    self.ty(element_type);
                }
                for element in elements {
                    self.expr(element);
                }
            }
            ExprKind::InterfaceLiteral {
                name,
                type_args,
                fields,
            } => {
                let resolved = self.name(name, expr.span);
                *name = resolved;
                for arg in type_args {
                    self.ty(arg);
                }
                for field in fields {
                    self.expr(&mut field.value);
                }
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Cast { expr: inner, ty } => {
                self.expr(inner);
                self.ty(ty);
            }
            ExprKind::Call {
                callee,
                type_args,
                args,
            } => {
                self.expr(callee);
                for arg in type_args {
                    self.ty(arg);
                }
                for arg in args {
                    self.expr(&mut arg.value);
                }
            }
            ExprKind::Alloc {
                target,
                element_type,
                count,
            } => {
                self.expr(target);
                self.ty(element_type);
                self.expr(count);
            }
            ExprKind::Move(value) | ExprKind::Copy(value) | ExprKind::Comptime(value) => {
                self.expr(value)
            }
            ExprKind::Member { base, .. } => self.expr(base),
            ExprKind::Index { base, index } => {
                self.expr(base);
                self.expr(index);
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {}
        }
    }

    fn block(&mut self, block: &mut Block) {
        for statement in &mut block.statements {
            self.stmt(statement);
        }
    }

    fn stmt(&mut self, statement: &mut Stmt) {
        match &mut statement.kind {
            StmtKind::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    self.ty(ty);
                }
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            StmtKind::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            StmtKind::Expr(expr) => self.expr(expr),
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.expr(condition);
                self.block(then_block);
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.block(block),
                    Some(ElseBranch::If(inner)) => self.stmt(inner),
                    None => {}
                }
            }
            StmtKind::While { condition, body } => {
                self.expr(condition);
                self.block(body);
            }
            StmtKind::ForRange {
                start, end, body, ..
            } => {
                self.expr(start);
                self.expr(end);
                self.block(body);
            }
            StmtKind::ForEach { iterable, body, .. } => {
                self.expr(iterable);
                self.block(body);
            }
            StmtKind::Protect(block) => self.block(block),
            StmtKind::Guard {
                ty, value, body, ..
            } => {
                if let Some(ty) = ty {
                    self.ty(ty);
                }
                self.expr(value);
                self.block(body);
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            StmtKind::Continue | StmtKind::Break => {}
        }
    }

    fn item(&mut self, item: &mut Item) {
        match item {
            Item::Import(_) => {}
            Item::Interface(decl) => {
                for field in &mut decl.fields {
                    self.ty(&mut field.ty);
                    if let Some(default) = &mut field.default {
                        self.expr(default);
                    }
                }
            }
            Item::Mark(_) | Item::Opaque(_) => {}
            Item::Choice(decl) => {
                for variant in &mut decl.variants {
                    self.ty(&mut variant.ty);
                }
            }
            Item::Global(decl) => {
                if let Some(ty) = &mut decl.ty {
                    self.ty(ty);
                }
                self.expr(&mut decl.value);
            }
            Item::Function(decl) => {
                for param in &mut decl.params {
                    self.ty(&mut param.ty);
                }
                if let Some(return_type) = &mut decl.return_type {
                    self.ty(return_type);
                }
                self.block(&mut decl.body);
            }
        }
    }
}

/// Rewrite every reference in `items` and return the resolution diagnostics.
/// `items` must all belong to the module at index `module`.
pub fn resolve_items(
    items: &mut [Item],
    module: usize,
    views: &[ModuleView],
    context: &ResolutionContext,
) -> Vec<Diagnostic> {
    let mut resolver = Resolver {
        module,
        views,
        context,
        diagnostics: Vec::new(),
    };
    for item in items {
        resolver.item(item);
    }
    resolver.diagnostics
}
