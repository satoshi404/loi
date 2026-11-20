//! Module name resolution and visibility enforcement.
//!
//! The driver loads each source file as a module and flattens the items into
//! one program. This pass runs on each module's own items *before*
//! monomorphization: it resolves `module::item` references, rejects references
//! to `sealed` declarations from another module, and gives declarations that
//! exist in more than one module a qualified internal name such as `a::helper`.
//! Unique names stay untouched, so single-module programs keep their existing
//! C symbols. The checker, monomorphizer and backend therefore see one global
//! namespace, but one with distinct names for cross-module collisions.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::checker::is_intrinsic_name;
use crate::diagnostic::Diagnostic;
use crate::types::release_flow_name_for;

/// Where a declaration was declared, for sealed-reference diagnostics.
#[derive(Debug, Clone)]
pub struct ModuleExport {
    pub module: usize,
    pub name: String,
    pub visibility: Visibility,
    pub span: Span,
    /// Named result of a flow, used to check the release call a `guard`
    /// synthesizes from an initializer's type.
    pub return_type: Option<String>,
}

/// The names a module can use in qualified references. Every module can refer
/// to itself by its own name; imports add the aliases written in that file.
#[derive(Debug, Clone)]
pub struct ModuleView {
    pub name: String,
    pub aliases: HashMap<String, usize>,
}

/// All declarations, keyed by their source name. Several modules may contribute
/// the same name; the resolver gives each contribution a qualified internal
/// name so the rest of the compiler sees a unique namespace.
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
    /// Names owned by the compiler or by the C entry point cannot be mangled.
    /// A duplicate `main` is a genuine program error and stays a checker error.
    fn is_reserved(name: &str) -> bool {
        name == "main" || is_intrinsic_name(name)
    }

    /// The internal name a declaration from `module` contributes. Only names
    /// declared in several modules need a qualifier; unique names keep their
    /// source spelling and therefore their existing C symbols.
    fn declared_name(&self, name: &str, module: usize) -> String {
        let shared = self.context.exports.get(name).is_some_and(|entries| {
            entries
                .iter()
                .map(|entry| entry.module)
                .collect::<HashSet<_>>()
                .len()
                > 1
        });
        if shared && !Self::is_reserved(name) {
            format!("{}::{}", self.views[module].name, name)
        } else {
            name.to_string()
        }
    }

    fn check_sealed(&mut self, declaration: &ModuleExport, alias: Option<&str>, span: Span) {
        if declaration.visibility == Visibility::Hidden && declaration.module != self.module {
            let message = match alias {
                Some(alias) => format!(
                    "item '{}' is sealed in module '{alias}' and cannot be used here",
                    declaration.name
                ),
                None => format!(
                    "'{}' is sealed in module '{}' and cannot be used from module '{}'",
                    declaration.name,
                    self.views[declaration.module].name,
                    self.views[self.module].name
                ),
            };
            self.diagnostics.push(
                Diagnostic::from_error(message, span)
                    .with_label("sealed item used here")
                    .with_secondary(declaration.span, "sealed here"),
            );
        }
    }

    /// Resolve one reference. Qualified names select a module; unqualified
    /// names bind to the current module first, then to a unique declaration
    /// elsewhere. An unqualified name shared by several other modules is
    /// ambiguous and must be qualified by the caller.
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
                .and_then(|entries| entries.iter().find(|entry| entry.module == target))
                .cloned();
            let Some(declaration) = declaration else {
                self.diagnostics.push(
                    Diagnostic::from_error(format!("module '{alias}' has no item '{item}'"), span)
                        .with_label("unknown item in this module"),
                );
                return item.to_string();
            };
            self.check_sealed(&declaration, Some(alias), span);
            return self.declared_name(item, target);
        }

        let Some(entries) = self.context.exports.get(name) else {
            return name.to_string();
        };
        if entries.iter().any(|entry| entry.module == self.module) {
            return self.declared_name(name, self.module);
        }
        let modules = entries
            .iter()
            .map(|entry| entry.module)
            .collect::<HashSet<_>>();
        if modules.len() == 1 {
            let declaration = entries[0].clone();
            self.check_sealed(&declaration, None, span);
            return self.declared_name(name, declaration.module);
        }
        let candidates = entries
            .iter()
            .map(|entry| format!("{}::{}", self.views[entry.module].name, name))
            .collect::<Vec<_>>()
            .join(", ");
        self.diagnostics.push(
            Diagnostic::from_error(
                format!("ambiguous name '{name}'; qualify it as one of {candidates}"),
                span,
            )
            .with_label("name declared in several modules")
            .with_help("write module::item to choose a declaration"),
        );
        self.declared_name(name, entries[0].module)
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
                // `guard` inserts a call to the convention-named release flow.
                // Resolve that implicit name too, so a sealed flow in another
                // module cannot be called through a guard.
                let release = if let Some(Type::Named(shape, span)) = ty.as_ref() {
                    Some((release_flow_name_for(shape), *span))
                } else if let ExprKind::InterfaceLiteral { name, .. } = &value.kind {
                    Some((release_flow_name_for(name), value.span))
                } else if let ExprKind::Call { callee, .. } = &value.kind
                    && let ExprKind::Ident(callee) = &callee.kind
                {
                    self.context.exports.get(callee).and_then(|entries| {
                        entries.iter().find_map(|entry| {
                            entry
                                .return_type
                                .as_ref()
                                .map(|shape| (release_flow_name_for(shape), value.span))
                        })
                    })
                } else {
                    None
                };
                if let Some((release, span)) = release {
                    self.name(&release, span);
                }
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
        let source_name = item.declared_name().unwrap_or("").to_string();
        match item {
            Item::Import(_) => {}
            Item::Interface(decl) => {
                for field in &mut decl.fields {
                    self.ty(&mut field.ty);
                    if let Some(default) = &mut field.default {
                        self.expr(default);
                    }
                }
                decl.name = self.declared_name(&source_name, self.module);
            }
            Item::Mark(decl) => {
                decl.name = self.declared_name(&source_name, self.module);
            }
            Item::Opaque(decl) => {
                decl.name = self.declared_name(&source_name, self.module);
            }
            Item::Choice(decl) => {
                for variant in &mut decl.variants {
                    self.ty(&mut variant.ty);
                }
                decl.name = self.declared_name(&source_name, self.module);
            }
            Item::Global(decl) => {
                if let Some(ty) = &mut decl.ty {
                    self.ty(ty);
                }
                self.expr(&mut decl.value);
                decl.name = self.declared_name(&source_name, self.module);
            }
            Item::Function(decl) => {
                for param in &mut decl.params {
                    self.ty(&mut param.ty);
                }
                if let Some(return_type) = &mut decl.return_type {
                    self.ty(return_type);
                }
                self.block(&mut decl.body);
                decl.name = self.declared_name(&source_name, self.module);
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
