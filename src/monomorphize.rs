//! Monomorphization of generic shapes and flows.
//!
//! Generic declarations (`shape Vec<T>`, `flow push<T>`) are templates. This
//! pass removes them from the program and replaces every explicit
//! instantiation with a concrete declaration whose name is derived from the
//! template and its type arguments, for example `Vec<i32>` becomes
//! `Vec__i32`. The type checker and the backends therefore never see a type
//! parameter: `ast::Type::Generic` only exists between parsing and this pass.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::types::Ty;

/// A diagnostic produced by monomorphization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonoError {
    pub span: Span,
    pub message: String,
}

impl MonoError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// Maximum nesting of generic instantiations. A template such as
/// `flow f<T> { => f<Vec<T>>(...) }` would otherwise generate types forever.
const MAX_INSTANTIATION_DEPTH: usize = 64;

/// Instantiate every generic declaration used by `program`.
///
/// The returned program contains no `ast::Type::Generic`, no type parameters
/// and no generic templates.
pub fn monomorphize(program: &Program) -> (Program, Vec<MonoError>) {
    let (program, errors, _) = run(program, false);
    (program, errors)
}

/// Like [`monomorphize`], but also instantiates templates that the program
/// never uses so the checker can validate their bodies.
///
/// An unused template is validated once with `i32` substituted for every
/// parameter. `i32` satisfies all three constraints (`Clone`, `Copy`, `Eq`), so
/// this remains a smoke test for name, type and ownership errors rather than a
/// proof that the body works for every type. Real uses are still checked with
/// their actual type arguments, and declared bounds are validated here too.
/// The extra declarations are only used by the checker; code generation calls
/// [`monomorphize`] and never sees them.
pub fn monomorphize_for_check(
    program: &Program,
) -> (Program, Vec<MonoError>, HashMap<String, String>) {
    run(program, true)
}

fn run(
    program: &Program,
    validate_unused_templates: bool,
) -> (Program, Vec<MonoError>, HashMap<String, String>) {
    let mut pass = Monomorphizer {
        interfaces: HashMap::new(),
        functions: HashMap::new(),
        named_interfaces: HashSet::new(),
        named_functions: HashSet::new(),
        marks: HashSet::new(),
        opaques: HashSet::new(),
        requested: HashMap::new(),
        instantiated: HashSet::new(),
        queue: Vec::new(),
        generated: Vec::new(),
        errors: Vec::new(),
        current_depth: 0,
        depth_error_reported: false,
    };

    for item in &program.items {
        match item {
            Item::Interface(decl) if !decl.type_params.is_empty() => {
                pass.interfaces.insert(decl.name.clone(), decl);
            }
            Item::Function(decl) if !decl.type_params.is_empty() => {
                pass.functions.insert(decl.name.clone(), decl);
            }
            Item::Interface(decl) => {
                pass.named_interfaces.insert(decl.name.clone());
            }
            Item::Function(decl) => {
                pass.named_functions.insert(decl.name.clone());
            }
            Item::Mark(decl) => {
                pass.marks.insert(decl.name.clone());
            }
            Item::Opaque(decl) => {
                pass.opaques.insert(decl.name.clone());
            }
            _ => {}
        }
    }

    let mut items = Vec::new();
    let empty = HashMap::new();
    for item in &program.items {
        match item {
            Item::Interface(decl) if !decl.type_params.is_empty() => {}
            Item::Function(decl) if !decl.type_params.is_empty() => {}
            Item::Interface(decl) => {
                let mut decl = decl.clone();
                pass.rewrite_interface(&mut decl, &empty);
                items.push(Item::Interface(decl));
            }
            Item::Function(decl) => {
                let mut decl = decl.clone();
                pass.rewrite_function(&mut decl, &empty);
                items.push(Item::Function(decl));
            }
            Item::Global(decl) => {
                let mut decl = decl.clone();
                if let Some(ty) = &mut decl.ty {
                    *ty = pass.rewrite_type(ty, &empty);
                }
                pass.rewrite_expr(&mut decl.value, &empty);
                items.push(Item::Global(decl));
            }
            Item::Mark(decl) => items.push(Item::Mark(decl.clone())),
            Item::Choice(decl) => items.push(Item::Choice(decl.clone())),
            Item::Opaque(decl) => items.push(Item::Opaque(decl.clone())),
            Item::Import(decl) => items.push(Item::Import(decl.clone())),
        }
    }

    pass.drain_queue();

    if validate_unused_templates {
        pass.request_unused_templates();
        pass.drain_queue();
    }

    items.append(&mut pass.generated);
    (Program { items }, pass.errors, pass.requested)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RequestKind {
    Interface,
    Function,
}

#[derive(Debug, Clone)]
struct Request {
    kind: RequestKind,
    base: String,
    args: Vec<Type>,
    name: String,
    span: Span,
    depth: usize,
}

struct Monomorphizer<'a> {
    interfaces: HashMap<String, &'a InterfaceDecl>,
    functions: HashMap<String, &'a FunctionDecl>,
    named_interfaces: HashSet<String>,
    named_functions: HashSet<String>,
    /// Non-shape named types that satisfy `Copy` and/or `Eq`. Choices and
    /// shapes are composite by construction and are not listed here.
    marks: HashSet<String>,
    opaques: HashSet<String>,
    /// Concrete name -> readable instantiation, used to detect mangling
    /// collisions as well as collisions with user declarations.
    requested: HashMap<String, String>,
    instantiated: HashSet<(RequestKind, String)>,
    queue: Vec<Request>,
    generated: Vec<Item>,
    errors: Vec<MonoError>,
    /// Depth of the request currently being rewritten. Requests created while
    /// rewriting it are one level deeper.
    current_depth: usize,
    depth_error_reported: bool,
}

impl<'a> Monomorphizer<'a> {
    fn rewrite_interface(&mut self, decl: &mut InterfaceDecl, mapping: &HashMap<String, Type>) {
        decl.type_params.clear();
        for field in &mut decl.fields {
            field.ty = self.rewrite_type(&field.ty, mapping);
            if let Some(default) = &mut field.default {
                self.rewrite_expr(default, mapping);
            }
        }
    }

    fn rewrite_function(&mut self, decl: &mut FunctionDecl, mapping: &HashMap<String, Type>) {
        decl.type_params.clear();
        for param in &mut decl.params {
            param.ty = self.rewrite_type(&param.ty, mapping);
        }
        if let Some(return_type) = &mut decl.return_type {
            *return_type = self.rewrite_type(return_type, mapping);
        }
        self.rewrite_block(&mut decl.body, mapping);
    }

    fn rewrite_block(&mut self, block: &mut Block, mapping: &HashMap<String, Type>) {
        for statement in &mut block.statements {
            self.rewrite_statement(statement, mapping);
        }
    }

    fn rewrite_statement(&mut self, statement: &mut Stmt, mapping: &HashMap<String, Type>) {
        match &mut statement.kind {
            StmtKind::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    *ty = self.rewrite_type(ty, mapping);
                }
                if let Some(value) = value {
                    self.rewrite_expr(value, mapping);
                }
            }
            StmtKind::Assign { target, value, .. } => {
                self.rewrite_expr(target, mapping);
                self.rewrite_expr(value, mapping);
            }
            StmtKind::Expr(expr) => self.rewrite_expr(expr, mapping),
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.rewrite_expr(condition, mapping);
                self.rewrite_block(then_block, mapping);
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.rewrite_block(block, mapping),
                    Some(ElseBranch::If(inner)) => self.rewrite_statement(inner, mapping),
                    None => {}
                }
            }
            StmtKind::While { condition, body } => {
                self.rewrite_expr(condition, mapping);
                self.rewrite_block(body, mapping);
            }
            StmtKind::ForRange {
                start, end, body, ..
            } => {
                self.rewrite_expr(start, mapping);
                self.rewrite_expr(end, mapping);
                self.rewrite_block(body, mapping);
            }
            StmtKind::ForEach { iterable, body, .. } => {
                self.rewrite_expr(iterable, mapping);
                self.rewrite_block(body, mapping);
            }
            StmtKind::Protect(block) => self.rewrite_block(block, mapping),
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.rewrite_expr(value, mapping);
                }
            }
            StmtKind::Continue | StmtKind::Break => {}
        }
    }

    fn rewrite_expr(&mut self, expr: &mut Expr, mapping: &HashMap<String, Type>) {
        let span = expr.span;
        match &mut expr.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::String(_)
            | ExprKind::Ident(_) => {}
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                if let Some(element_type) = element_type {
                    *element_type = self.rewrite_type(element_type, mapping);
                }
                for element in elements {
                    self.rewrite_expr(element, mapping);
                }
            }
            ExprKind::InterfaceLiteral {
                name,
                type_args,
                fields,
            } => {
                if type_args.is_empty() {
                    if self.interfaces.contains_key(name) {
                        self.errors.push(MonoError::new(
                            span,
                            format!(
                                "generic shape '{name}' requires explicit type arguments, e.g. {name}<i32>[...]"
                            ),
                        ));
                    }
                } else {
                    let args = type_args
                        .iter()
                        .map(|ty| self.rewrite_type(ty, mapping))
                        .collect::<Vec<_>>();
                    let instantiated = self.request(RequestKind::Interface, name, args, span);
                    *name = instantiated;
                    type_args.clear();
                }
                for field in fields {
                    self.rewrite_expr(&mut field.value, mapping);
                }
            }
            ExprKind::Unary { operand, .. } => self.rewrite_expr(operand, mapping),
            ExprKind::Binary { left, right, .. } => {
                self.rewrite_expr(left, mapping);
                self.rewrite_expr(right, mapping);
            }
            ExprKind::Cast { expr, ty } => {
                self.rewrite_expr(expr, mapping);
                *ty = self.rewrite_type(ty, mapping);
            }
            ExprKind::Call {
                callee,
                type_args,
                args,
            } => {
                self.rewrite_expr(callee, mapping);
                if type_args.is_empty() {
                    if let ExprKind::Ident(name) = &callee.kind
                        && self.functions.contains_key(name)
                    {
                        self.errors.push(MonoError::new(
                            span,
                            format!(
                                "generic flow '{name}' requires explicit type arguments, e.g. {name}<i32>(...)"
                            ),
                        ));
                    }
                } else {
                    let args_ty = type_args
                        .iter()
                        .map(|ty| self.rewrite_type(ty, mapping))
                        .collect::<Vec<_>>();
                    match &callee.kind {
                        ExprKind::Ident(name) if self.functions.contains_key(name) => {
                            let name = name.clone();
                            let instantiated =
                                self.request(RequestKind::Function, &name, args_ty, span);
                            if let ExprKind::Ident(target) = &mut callee.as_mut().kind {
                                *target = instantiated;
                            }
                        }
                        ExprKind::Ident(name) => {
                            self.errors.push(MonoError::new(
                                span,
                                format!("flow '{name}' does not take type arguments"),
                            ));
                        }
                        _ => self.errors.push(MonoError::new(
                            span,
                            "only named flows can take type arguments",
                        )),
                    }
                    type_args.clear();
                }
                for arg in args {
                    self.rewrite_expr(&mut arg.value, mapping);
                }
            }
            ExprKind::Alloc {
                target,
                element_type,
                count,
            } => {
                self.rewrite_expr(target, mapping);
                *element_type = self.rewrite_type(element_type, mapping);
                self.rewrite_expr(count, mapping);
            }
            ExprKind::Move(value) | ExprKind::Copy(value) | ExprKind::Comptime(value) => {
                self.rewrite_expr(value, mapping);
            }
            ExprKind::Member { base, .. } => self.rewrite_expr(base, mapping),
            ExprKind::Index { base, index } => {
                self.rewrite_expr(base, mapping);
                self.rewrite_expr(index, mapping);
            }
        }
    }

    fn rewrite_type(&mut self, ty: &Type, mapping: &HashMap<String, Type>) -> Type {
        let span = ty.span();
        match ty {
            Type::Named(name, _) => match mapping.get(name) {
                Some(ty) => ty.clone(),
                None => Type::Named(name.clone(), span),
            },
            Type::Slice(element, _) => {
                Type::Slice(Box::new(self.rewrite_type(element, mapping)), span)
            }
            Type::Generic(name, args, _) => {
                let args = args
                    .iter()
                    .map(|arg| self.rewrite_type(arg, mapping))
                    .collect::<Vec<_>>();
                if self.interfaces.contains_key(name) {
                    Type::Named(self.request(RequestKind::Interface, name, args, span), span)
                } else {
                    self.errors.push(MonoError::new(
                        span,
                        format!("unknown generic type '{name}'"),
                    ));
                    Type::Named(name.clone(), span)
                }
            }
        }
    }

    fn request(&mut self, kind: RequestKind, base: &str, args: Vec<Type>, span: Span) -> String {
        let name = instantiation_name(base, &args);
        let display = instantiation_display(base, &args);

        let declared = match kind {
            RequestKind::Interface => self.named_interfaces.contains(&name),
            RequestKind::Function => self.named_functions.contains(&name),
        };
        if declared {
            let noun = match kind {
                RequestKind::Interface => "shape",
                RequestKind::Function => "flow",
            };
            self.errors.push(MonoError::new(
                span,
                format!(
                    "generated name '{name}' for {noun} '{display}' collides with an existing declaration named '{name}'; rename the declaration or change the type argument"
                ),
            ));
        }
        if let Some(existing) = self.requested.get(&name) {
            if existing != &display {
                self.errors.push(MonoError::new(
                    span,
                    format!(
                        "generated name '{name}' is ambiguous: it names both '{existing}' and '{display}'; rename one of the types involved"
                    ),
                ));
            }
            return name;
        }
        self.requested.insert(name.clone(), display);

        let depth = self.current_depth + 1;
        if depth > MAX_INSTANTIATION_DEPTH {
            if !self.depth_error_reported {
                self.depth_error_reported = true;
                self.errors.push(MonoError::new(
                    span,
                    format!(
                        "generic instantiation exceeds the depth limit of {MAX_INSTANTIATION_DEPTH}; recursive instantiations such as 'f<T>' calling 'f<Vec<T>>' are not allowed"
                    ),
                ));
            }
            return name;
        }

        self.queue.push(Request {
            kind,
            base: base.to_string(),
            args,
            name: name.clone(),
            span,
            depth,
        });
        name
    }

    fn drain_queue(&mut self) {
        while let Some(request) = self.queue.pop() {
            self.current_depth = request.depth;
            self.build(request);
        }
        self.current_depth = 0;
    }

    /// Instantiate every still-unused template with `i32` for each type
    /// parameter, so the checker validates its body.
    fn request_unused_templates(&mut self) {
        let mut interfaces = self
            .interfaces
            .iter()
            .map(|(name, decl)| (name.clone(), decl.type_params.len(), decl.span))
            .collect::<Vec<_>>();
        let mut functions = self
            .functions
            .iter()
            .map(|(name, decl)| (name.clone(), decl.type_params.len(), decl.span))
            .collect::<Vec<_>>();
        interfaces.sort_by(|left, right| left.0.cmp(&right.0));
        functions.sort_by(|left, right| left.0.cmp(&right.0));

        for (name, arity, span) in interfaces {
            if self
                .instantiated
                .contains(&(RequestKind::Interface, name.clone()))
            {
                continue;
            }
            let args = vec![Type::named("i32"); arity];
            self.request(RequestKind::Interface, &name, args, span);
        }
        for (name, arity, span) in functions {
            if self
                .instantiated
                .contains(&(RequestKind::Function, name.clone()))
            {
                continue;
            }
            let args = vec![Type::named("i32"); arity];
            self.request(RequestKind::Function, &name, args, span);
        }
    }

    fn build(&mut self, request: Request) {
        self.instantiated
            .insert((request.kind, request.base.clone()));
        let mut mapping = HashMap::new();
        match request.kind {
            RequestKind::Interface => {
                let Some(template) = self.interfaces.get(&request.base).copied() else {
                    return;
                };
                if template.type_params.len() != request.args.len() {
                    self.errors.push(MonoError::new(
                        request.span,
                        format!(
                            "shape '{}' expects {} type argument(s), found {}",
                            request.base,
                            template.type_params.len(),
                            request.args.len()
                        ),
                    ));
                    return;
                }
                let bound_errors = self.check_bounds(
                    &template.type_params,
                    &request.args,
                    "shape",
                    &request.base,
                    request.span,
                );
                self.errors.extend(bound_errors);
                for (param, arg) in template.type_params.iter().zip(request.args.iter()) {
                    mapping.insert(param.name.clone(), arg.clone());
                }
                let mut decl = template.clone();
                decl.name = request.name;
                decl.type_params.clear();
                self.rewrite_interface(&mut decl, &mapping);
                self.generated.push(Item::Interface(decl));
            }
            RequestKind::Function => {
                let Some(template) = self.functions.get(&request.base).copied() else {
                    return;
                };
                if template.type_params.len() != request.args.len() {
                    self.errors.push(MonoError::new(
                        request.span,
                        format!(
                            "flow '{}' expects {} type argument(s), found {}",
                            request.base,
                            template.type_params.len(),
                            request.args.len()
                        ),
                    ));
                    return;
                }
                let bound_errors = self.check_bounds(
                    &template.type_params,
                    &request.args,
                    "flow",
                    &request.base,
                    request.span,
                );
                self.errors.extend(bound_errors);
                for (param, arg) in template.type_params.iter().zip(request.args.iter()) {
                    mapping.insert(param.name.clone(), arg.clone());
                }
                let mut decl = template.clone();
                decl.name = request.name;
                decl.type_params.clear();
                self.rewrite_function(&mut decl, &mapping);
                self.generated.push(Item::Function(decl));
            }
        }
    }

    /// Human-readable type spelling for bound diagnostics. Instantiated
    /// arguments reach this pass already mangled (`Vec__i32`), but users wrote
    /// `Vec<i32>`; `requested` records the mapping. Collision diagnostics keep
    /// the mangled name on purpose, so only bound messages use this helper.
    fn type_display_source(&self, ty: &Type) -> String {
        match ty {
            Type::Named(name, _) => self
                .requested
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone()),
            Type::Slice(element, _) => format!("[{}]", self.type_display_source(element)),
            Type::Generic(name, args, _) => {
                let base = self
                    .requested
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                let rendered = args
                    .iter()
                    .map(|arg| self.type_display_source(arg))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{base}<{rendered}>")
            }
        }
    }

    /// Validate declared constraints once the concrete type arguments are
    /// known. The request span is the call site or type use that triggered the
    /// instantiation, so the diagnostic points at code the user wrote rather
    /// than at a generated declaration.
    fn check_bounds(
        &self,
        params: &[TypeParam],
        args: &[Type],
        noun: &str,
        base: &str,
        span: Span,
    ) -> Vec<MonoError> {
        let mut errors = Vec::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            let missing = param
                .bounds
                .iter()
                .copied()
                .filter(|bound| !self.bound_satisfied(*bound, arg))
                .collect::<Vec<_>>();
            if missing.is_empty() {
                continue;
            }
            let names = missing
                .iter()
                .map(|bound| format!("'{bound}'"))
                .collect::<Vec<_>>()
                .join(" and ");
            let hint = if missing.contains(&TypeBound::Eq) {
                "Eq requires a numeric type, bool, a mark or [u8]"
            } else if missing.contains(&TypeBound::Copy) {
                "Copy requires a numeric type, bool, a mark, an opaque handle or cstr"
            } else {
                "every Loi type is Clone, so this bound is always satisfied"
            };
            errors.push(MonoError::new(
                span,
                format!(
                    "type '{}' does not satisfy the {names} bound on type parameter '{}' of {noun} '{base}'; {hint}",
                    self.type_display_source(arg),
                    param.name
                ),
            ));
        }
        errors
    }

    fn bound_satisfied(&self, bound: TypeBound, ty: &Type) -> bool {
        match bound {
            TypeBound::Clone => true,
            TypeBound::Copy => self.is_copy_type(ty),
            TypeBound::Eq => self.is_eq_type(ty),
        }
    }

    /// Mirror the checker's implicit-copy rule: scalars, marks, opaque handles
    /// and `cstr` are bit-copied; shapes, choices and slices are composite.
    fn is_copy_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(name, _) => {
                Ty::from_name(name).is_some()
                    || self.marks.contains(name)
                    || self.opaques.contains(name)
                    || name == "cstr"
            }
            Type::Slice(_, _) | Type::Generic(..) => false,
        }
    }

    /// Mirror the checker's `==`/`!=` rule: numbers, bool, two values of the
    /// same mark, and byte slices. Structural equality for shapes, choices and
    /// other slices is deliberately not part of `Eq`.
    fn is_eq_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Named(name, _) => Ty::from_name(name).is_some() || self.marks.contains(name),
            Type::Slice(element, _) => {
                matches!(&**element, Type::Named(name, _) if name == "u8")
            }
            Type::Generic(..) => false,
        }
    }

    #[allow(dead_code)]
    fn is_known_type(&self, name: &str) -> bool {
        self.named_interfaces.contains(name) || self.named_functions.contains(name)
    }
}

/// The mangled name of `base<args>`.
pub fn instantiation_name(base: &str, args: &[Type]) -> String {
    if args.is_empty() {
        return base.to_string();
    }
    format!(
        "{}__{}",
        base,
        args.iter().map(type_mangle).collect::<Vec<_>>().join("__")
    )
}

fn type_mangle(ty: &Type) -> String {
    match ty {
        Type::Named(name, _) => name.clone(),
        Type::Slice(element, _) => format!("slice_{}", type_mangle(element)),
        Type::Generic(name, args, _) => instantiation_name(name, args),
    }
}

/// Human-readable spelling of an instantiation, for diagnostics. This is what
/// users wrote (`Vec<i32>`), unlike the mangled C identifier (`Vec__i32`).
fn instantiation_display(base: &str, args: &[Type]) -> String {
    if args.is_empty() {
        return base.to_string();
    }
    format!(
        "{base}<{}>",
        args.iter().map(type_display).collect::<Vec<_>>().join(", ")
    )
}

fn type_display(ty: &Type) -> String {
    match ty {
        Type::Named(name, _) => name.clone(),
        Type::Slice(element, _) => format!("[{}]", type_display(element)),
        Type::Generic(name, args, _) => instantiation_display(name, args),
    }
}
