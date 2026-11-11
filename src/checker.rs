//! Semantic analysis: names, types, directions and ownership.
//!
//! The checker performs three passes over the AST:
//!
//! 1. collect interface and function names (so declarations can refer to each
//!    other regardless of order);
//! 2. resolve interface fields and function signatures;
//! 3. check interface defaults and function bodies.
//!
//! Body checking includes the core ownership rules: path-sensitive move
//! liveness, branch-aware definite assignment and definite return, exclusive
//! `out`/`inout` arguments, cross-function parameter consumption, the ban on
//! implicit composite copies, and `protect`/`alloc` region tracking with escape
//! analysis, including taint propagation through consumed `inout` arguments.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::types::{self, Ty};

/// A semantic error with a source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

impl Diagnostic {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// Check a complete program, returning every diagnostic found.
pub fn check(program: &Program) -> Vec<Diagnostic> {
    let mut checker = Checker::new();
    checker.collect_names(program);
    checker.resolve_interfaces(program);
    checker.check_interface_value_cycles(program);
    checker.resolve_functions(program);
    checker.analyze_function_effects(program);
    checker.check_interface_defaults(program);
    checker.declare_globals(program);
    checker.check_functions(program);
    checker.diagnostics
}

#[derive(Debug, Clone)]
struct FieldInfo {
    name: String,
    ty: Ty,
    has_default: bool,
    span: Span,
}

#[derive(Debug, Clone)]
struct InterfaceInfo {
    span: Span,
    fields: Vec<FieldInfo>,
    field_index: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
struct ParamSig {
    name: String,
    direction: Direction,
    ty: Ty,
}

#[derive(Debug, Clone)]
struct FunctionSig {
    span: Span,
    params: Vec<ParamSig>,
    return_type: Ty,
}

#[derive(Debug, Clone)]
struct VarInfo {
    ty: Ty,
    mutable: bool,
    initialized: bool,
    direction: Option<Direction>,
    span: Span,
    /// Paths moved out of this variable. `""` means the whole variable.
    moved: HashSet<String>,
    /// Lexical block or `protect` region where this variable's storage lives;
    /// `None` means a parameter or the caller.
    home_region: Option<usize>,
    /// `protect` allocations or block-local slice literals reachable through
    /// this variable.
    taints: Vec<RegionTaint>,
}

#[derive(Debug, Clone)]
struct RegionTaint {
    path: String,
    region: usize,
}

/// Why a region exists. Lexical blocks own slice-literal storage; `protect`
/// blocks own `alloc` storage. Both are checked for escape, but the diagnostic
/// names the kind so the fix is clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionKind {
    Block,
    Protect,
}

/// Effects collected from one function body before body checking.
#[derive(Default)]
struct FunctionScan {
    /// An `alloc` outside every lexical `protect`.
    direct_alloc: bool,
    /// Calls made outside every lexical `protect`.
    unguarded_deps: Vec<String>,
    /// Every called function name.
    calls: HashSet<String>,
    /// Roots of expressions passed to `move(...)`.
    moved_roots: HashSet<String>,
    /// Call sites with the direction and root of every argument.
    call_args: Vec<CallSiteScan>,
    /// Whether returning a place transfers ownership (composite return type).
    returns_composite: bool,
}

struct CallSiteScan {
    callee: String,
    args: Vec<(Direction, Option<String>)>,
}

struct LvalueInfo {
    ty: Ty,
    root: Option<String>,
    /// Path relative to the root variable. `Some("")` is the whole variable.
    path: Option<String>,
}

#[derive(Debug, Clone)]
struct ControlState {
    initialized: bool,
    moved: HashSet<String>,
    taints: Vec<RegionTaint>,
}

struct Checker {
    interfaces: HashMap<String, InterfaceInfo>,
    /// Foreign handle names declared with `opaque`.
    opaques: HashSet<String>,
    functions: HashMap<String, FunctionSig>,
    scopes: Vec<HashMap<String, VarInfo>>,
    diagnostics: Vec<Diagnostic>,
    current_return: Ty,
    current_function: String,
    /// Whether control can reach the current point in the current block.
    reachable: bool,
    /// Nesting depth of loops (`while`, `for`, `for in`, `for inout`). Used to
    /// validate that `break` and `continue` only appear inside loops.
    loop_depth: usize,
    /// Function names that can allocate without a lexical `protect` in this
    /// function (directly or through another such function).
    unguarded_allocators: HashSet<String>,
    /// Per function, whether each parameter may be moved out by the call.
    param_consumption: HashMap<String, Vec<bool>>,
    /// Names declared by top-level `let`/`var`, for duplicate diagnostics.
    global_names: HashSet<String>,
    /// Parent region for each lexical block or `protect` region.
    region_parents: Vec<Option<usize>>,
    /// Kind of each region, indexed by region id.
    region_kinds: Vec<RegionKind>,
    /// Regions currently active, innermost last.
    region_stack: Vec<usize>,
}

impl Checker {
    fn new() -> Self {
        Self {
            interfaces: HashMap::new(),
            opaques: HashSet::new(),
            functions: HashMap::new(),
            // Index 0 is the global scope; function bodies push on top of it.
            scopes: vec![HashMap::new()],
            diagnostics: Vec::new(),
            current_return: Ty::Void,
            current_function: String::new(),
            reachable: true,
            loop_depth: 0,
            unguarded_allocators: HashSet::new(),
            param_consumption: HashMap::new(),
            global_names: HashSet::new(),
            region_parents: Vec::new(),
            region_kinds: Vec::new(),
            region_stack: Vec::new(),
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::new(span, message));
    }

    // ------------------------------------------------------------------
    // Pass 1: names
    // ------------------------------------------------------------------
    fn collect_names(&mut self, program: &Program) {
        // Intrinsics: a closed, reserved set of compiler-known functions with
        // direct backend mappings (`LOI_SPEC.md` 5.9). Everything else belongs
        // to the explicitly imported standard library in `lib/std.loi`.
        //
        // print(in msg: []u8) -> void       -> fwrite(msg.data, 1, msg.len, stdout)
        // sqrt(in x: f64) -> f64            -> sqrt(x)
        self.register_intrinsic(
            "print",
            vec![("msg", Direction::In, Ty::Slice(Box::new(Ty::U8)))],
            Ty::Void,
        );
        self.register_intrinsic("sqrt", vec![("x", Direction::In, Ty::F64)], Ty::F64);
        self.register_intrinsic(
            "eprint",
            vec![("msg", Direction::In, Ty::Slice(Box::new(Ty::U8)))],
            Ty::Void,
        );
        self.register_intrinsic(
            "panic",
            vec![("msg", Direction::In, Ty::Slice(Box::new(Ty::U8)))],
            Ty::Void,
        );
        self.register_intrinsic(
            "read_file",
            vec![("path", Direction::In, Ty::Slice(Box::new(Ty::U8)))],
            Ty::Slice(Box::new(Ty::U8)),
        );
        self.register_intrinsic(
            "write_file",
            vec![
                ("path", Direction::In, Ty::Slice(Box::new(Ty::U8))),
                ("data", Direction::In, Ty::Slice(Box::new(Ty::U8))),
            ],
            Ty::Bool,
        );
        self.register_intrinsic("read_stdin", Vec::new(), Ty::Slice(Box::new(Ty::U8)));
        self.register_intrinsic("read_line", Vec::new(), Ty::Slice(Box::new(Ty::U8)));
        self.register_intrinsic(
            "print_int",
            vec![("value", Direction::In, Ty::I64)],
            Ty::Void,
        );
        self.register_intrinsic(
            "print_float",
            vec![("value", Direction::In, Ty::F64)],
            Ty::Void,
        );
        self.register_intrinsic("floor", vec![("x", Direction::In, Ty::F64)], Ty::F64);
        self.register_intrinsic("ceil", vec![("x", Direction::In, Ty::F64)], Ty::F64);
        self.register_intrinsic("fabs", vec![("x", Direction::In, Ty::F64)], Ty::F64);
        self.register_intrinsic(
            "pow",
            vec![
                ("base", Direction::In, Ty::F64),
                ("exponent", Direction::In, Ty::F64),
            ],
            Ty::F64,
        );
        self.register_intrinsic(
            "fmod",
            vec![
                ("left", Direction::In, Ty::F64),
                ("right", Direction::In, Ty::F64),
            ],
            Ty::F64,
        );

        for item in &program.items {
            match item {
                Item::Import(_) => {}
                Item::Opaque(decl) => {
                    if is_intrinsic_name(&decl.name) {
                        self.error(
                            decl.span,
                            format!("intrinsic '{}' cannot be redeclared", decl.name),
                        );
                    } else if self.opaques.contains(&decl.name)
                        || self.interfaces.contains_key(&decl.name)
                        || self.functions.contains_key(&decl.name)
                        || self.global_names.contains(&decl.name)
                    {
                        self.error(decl.span, format!("duplicate type '{}'", decl.name));
                    } else {
                        self.opaques.insert(decl.name.clone());
                    }
                }
                Item::Global(decl) => {
                    if is_intrinsic_name(&decl.name) {
                        self.error(
                            decl.span,
                            format!("intrinsic '{}' cannot be redeclared", decl.name),
                        );
                        continue;
                    }
                    if self.opaques.contains(&decl.name) {
                        self.error(
                            decl.span,
                            format!(
                                "global '{}' conflicts with an opaque type of the same name",
                                decl.name
                            ),
                        );
                    }
                    if !self.global_names.insert(decl.name.clone()) {
                        self.error(decl.span, format!("duplicate global '{}'", decl.name));
                    }
                    if self.functions.contains_key(&decl.name) {
                        self.error(
                            decl.span,
                            format!(
                                "global '{}' conflicts with a function of the same name",
                                decl.name
                            ),
                        );
                    }
                }
                Item::Interface(decl) => {
                    if self.interfaces.contains_key(&decl.name) {
                        self.error(decl.span, format!("duplicate shape '{}'", decl.name));
                    } else if self.opaques.contains(&decl.name)
                        || self.functions.contains_key(&decl.name)
                        || self.global_names.contains(&decl.name)
                    {
                        self.error(decl.span, format!("duplicate type '{}'", decl.name));
                    } else {
                        self.interfaces.insert(
                            decl.name.clone(),
                            InterfaceInfo {
                                span: decl.span,
                                fields: Vec::new(),
                                field_index: HashMap::new(),
                            },
                        );
                    }
                }
                Item::Function(decl) => {
                    if is_intrinsic_name(&decl.name) {
                        self.error(
                            decl.span,
                            format!("intrinsic '{}' cannot be redeclared", decl.name),
                        );
                    } else if self.opaques.contains(&decl.name) {
                        self.error(
                            decl.span,
                            format!(
                                "function '{}' conflicts with an opaque type of the same name",
                                decl.name
                            ),
                        );
                    } else if self.functions.contains_key(&decl.name) {
                        self.error(decl.span, format!("duplicate function '{}'", decl.name));
                    } else {
                        if self.global_names.contains(&decl.name) {
                            self.error(
                                decl.span,
                                format!(
                                    "function '{}' conflicts with a global of the same name",
                                    decl.name
                                ),
                            );
                        }
                        self.functions.insert(
                            decl.name.clone(),
                            FunctionSig {
                                span: decl.span,
                                params: Vec::new(),
                                return_type: Ty::Void,
                            },
                        );
                    }
                }
            }
        }
    }

    /// Register a built-in intrinsic with a fixed signature. Intrinsics are
    /// reserved names; user declarations are rejected in `collect_names`.
    fn register_intrinsic(
        &mut self,
        name: &str,
        params: Vec<(&str, Direction, Ty)>,
        return_type: Ty,
    ) {
        if self.functions.contains_key(name) {
            return; // duplicate declaration was already reported
        }
        let sig_params = params
            .into_iter()
            .map(|(name, direction, ty)| ParamSig {
                name: name.to_string(),
                direction,
                ty,
            })
            .collect();
        self.functions.insert(
            name.to_string(),
            FunctionSig {
                span: Span { line: 0, column: 0 },
                params: sig_params,
                return_type,
            },
        );
    }

    // ------------------------------------------------------------------
    // Pass 2: signatures
    // ------------------------------------------------------------------
    fn resolve_interfaces(&mut self, program: &Program) {
        for item in &program.items {
            let Item::Interface(decl) = item else {
                continue;
            };
            if !self.is_canonical_interface(&decl.name, decl.span) {
                continue;
            }

            let mut fields = Vec::new();
            let mut field_index = HashMap::new();
            for field in &decl.fields {
                let ty = self.resolve_type(&field.ty, field.span);
                if field_index.contains_key(&field.name) {
                    self.error(
                        field.span,
                        format!("duplicate field '{}' in shape '{}'", field.name, decl.name),
                    );
                    continue;
                }

                field_index.insert(field.name.clone(), fields.len());
                fields.push(FieldInfo {
                    name: field.name.clone(),
                    ty,
                    has_default: field.default.is_some(),
                    span: field.span,
                });
            }

            self.interfaces.insert(
                decl.name.clone(),
                InterfaceInfo {
                    span: decl.span,
                    fields,
                    field_index,
                },
            );
        }
    }

    /// Detect interfaces that contain themselves by value through a chain of
    /// field types. A cycle like `A { b: B }` / `B { a: A }` cannot be laid out
    /// as a C struct, so it is a compile error. Slice fields (`[]B`) do not
    /// create a value dependency because they store a pointer.
    fn check_interface_value_cycles(&mut self, program: &Program) {
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        for item in &program.items {
            let Item::Interface(decl) = item else {
                continue;
            };
            if !self.is_canonical_interface(&decl.name, decl.span) {
                continue;
            }
            let mut deps = Vec::new();
            for field in &decl.fields {
                if let Type::Named(name) = &field.ty
                    && self.interfaces.contains_key(name)
                {
                    deps.push(name.clone());
                }
            }
            graph.insert(decl.name.clone(), deps);
        }

        let mut color: HashMap<String, u8> = HashMap::new(); // 0 white, 1 gray, 2 black
        let mut path: Vec<String> = Vec::new();
        for start in graph.keys() {
            if color.get(start).copied().unwrap_or(0) == 0 {
                self.visit_interface_value_cycle(start, &graph, &mut color, &mut path);
            }
        }
    }

    fn visit_interface_value_cycle(
        &mut self,
        name: &str,
        graph: &HashMap<String, Vec<String>>,
        color: &mut HashMap<String, u8>,
        path: &mut Vec<String>,
    ) {
        color.insert(name.to_string(), 1);
        path.push(name.to_string());
        for dep in graph.get(name).iter().flat_map(|deps| deps.iter()) {
            match color.get(dep).copied().unwrap_or(0) {
                0 => {
                    self.visit_interface_value_cycle(dep, graph, color, path);
                }
                1 => {
                    let cycle_start = path
                        .iter()
                        .position(|n| n == dep)
                        .expect("gray node is on the path");
                    let cycle: Vec<&str> = path[cycle_start..].iter().map(|n| n.as_str()).collect();
                    let cycle_str = cycle.join(" -> ");
                    let info = self.interfaces.get(dep).expect("interface exists");
                    self.error(
                        info.span,
                        format!(
                            "shape '{dep}' contains itself by value through a cycle: {cycle_str} -> {dep}",
                        ),
                    );
                }
                _ => {}
            }
        }
        path.pop();
        color.insert(name.to_string(), 2);
    }

    fn resolve_functions(&mut self, program: &Program) {
        for item in &program.items {
            let Item::Function(decl) = item else {
                continue;
            };
            if !self.is_canonical_function(&decl.name, decl.span) {
                continue;
            }

            let mut params = Vec::new();
            let mut seen = HashSet::new();
            for param in &decl.params {
                let ty = self.resolve_type(&param.ty, param.span);
                if !seen.insert(param.name.clone()) {
                    self.error(
                        param.span,
                        format!(
                            "duplicate parameter '{}' in function '{}'",
                            param.name, decl.name
                        ),
                    );
                    continue;
                }
                params.push(ParamSig {
                    name: param.name.clone(),
                    direction: param.direction,
                    ty,
                });
            }

            let return_type = match &decl.return_type {
                Some(ty) => self.resolve_type(ty, decl.span),
                None => Ty::Void,
            };

            self.functions.insert(
                decl.name.clone(),
                FunctionSig {
                    span: decl.span,
                    params,
                    return_type,
                },
            );
        }
    }

    fn is_canonical_interface(&self, name: &str, span: Span) -> bool {
        self.interfaces.get(name).map(|info| info.span) == Some(span)
    }

    fn is_canonical_function(&self, name: &str, span: Span) -> bool {
        self.functions.get(name).map(|sig| sig.span) == Some(span)
    }

    /// Resolve and type-check top-level `let`/`var` declarations.
    ///
    /// Globals live in the base scope, so every function body sees them. Their
    /// initializers must be compile-time constants: the C backend emits them as
    /// static initializers.
    fn declare_globals(&mut self, program: &Program) {
        self.current_return = Ty::Void;
        for item in &program.items {
            let Item::Global(decl) = item else {
                continue;
            };

            let declared = decl.ty.as_ref().map(|ty| self.resolve_type(ty, decl.span));
            let value_ty = self.check_expr(&decl.value);

            if !is_constant_expr(&decl.value) {
                self.error(
                    decl.value.span,
                    format!(
                        "initializer for global '{}' must be a compile-time constant",
                        decl.name
                    ),
                );
            }

            let resolved = match (declared, value_ty) {
                (Some(declared), value_ty) => {
                    if !self.assignable_expr(&declared, &value_ty, &decl.value) {
                        self.error(
                            decl.value.span,
                            format!(
                                "cannot initialize '{}' of type {} with {}",
                                decl.name, declared, value_ty
                            ),
                        );
                    }
                    self.check_literal_range(&decl.value, &declared);
                    declared
                }
                (None, value_ty) => value_ty.concretize(),
            };

            // The C backend can only emit scalars and byte-string globals as
            // static initializers. Other composite globals need a runtime
            // constructor, which is not implemented yet.
            let supported = matches!(
                resolved,
                Ty::I8
                    | Ty::I16
                    | Ty::I32
                    | Ty::I64
                    | Ty::U8
                    | Ty::U16
                    | Ty::U32
                    | Ty::U64
                    | Ty::F32
                    | Ty::F64
                    | Ty::Bool
                    | Ty::CStr
                    | Ty::Error
            ) || (matches!(&resolved, Ty::Slice(element) if **element == Ty::U8)
                && matches!(&decl.value.kind, ExprKind::String(_)));
            if !supported {
                self.error(
                    decl.span,
                    format!(
                        "global '{}' has type {resolved}; only scalar and [u8] globals are supported",
                        decl.name
                    ),
                );
            }

            self.scopes[0].insert(
                decl.name.clone(),
                VarInfo {
                    ty: resolved,
                    mutable: decl.mutable,
                    initialized: true,
                    direction: None,
                    span: decl.span,
                    moved: HashSet::new(),
                    home_region: None,
                    taints: Vec::new(),
                },
            );
        }
    }

    fn resolve_type(&mut self, ty: &Type, span: Span) -> Ty {
        match ty {
            Type::Named(name) => {
                if let Some(primitive) = Ty::from_name(name) {
                    primitive
                } else if name == "cstr" {
                    Ty::CStr
                } else if self.interfaces.contains_key(name) {
                    Ty::Named(name.clone())
                } else if self.opaques.contains(name) {
                    Ty::Opaque(name.clone())
                } else {
                    self.error(span, format!("unknown type '{name}'"));
                    Ty::Error
                }
            }
            Type::Slice(element) => Ty::Slice(Box::new(self.resolve_type(element, span))),
        }
    }

    /// Like [`types::assignable`], with the single literal coercion the FFI
    /// boundary needs: a string literal may initialize a `cstr`. Arbitrary
    /// `[u8]` values are not NUL-terminated, so they are rejected.
    fn assignable_expr(&self, expected: &Ty, actual: &Ty, expr: &Expr) -> bool {
        if matches!(expected, Ty::CStr) {
            return match actual {
                Ty::CStr => true,
                Ty::Slice(element) => {
                    **element == Ty::U8 && matches!(&expr.kind, ExprKind::String(_))
                }
                _ => false,
            };
        }
        types::assignable(expected, actual)
    }

    // ------------------------------------------------------------------
    // Function-effect prepass
    // ------------------------------------------------------------------
    /// Compute cross-function effects before checking any body.
    ///
    /// Allocation: an allocation in a callee belongs to the nearest `protect`
    /// of its caller, so a non-entry function may contain an unguarded `alloc`
    /// as long as every caller provides a region. Entry functions are `main`
    /// and functions that no function calls, except uncalled `hidden` helpers,
    /// which are only reachable through other Loi code and may rely on a
    /// caller's `protect`.
    ///
    /// Ownership: a parameter is *consumed* when the function moves from it,
    /// directly or through another call. At a call site a consumed `out` or
    /// `inout` argument is moved for the caller.
    fn analyze_function_effects(&mut self, program: &Program) {
        let mut scans: HashMap<String, FunctionScan> = HashMap::new();
        let mut params: HashMap<String, Vec<(String, Direction)>> = HashMap::new();
        let mut spans: Vec<(String, Span, Visibility)> = Vec::new();

        for item in &program.items {
            let Item::Function(decl) = item else {
                continue;
            };
            if !self.is_canonical_function(&decl.name, decl.span) {
                continue;
            }

            let mut scan = FunctionScan::default();
            if let Some(signature) = self.functions.get(&decl.name) {
                scan.returns_composite = is_composite(&signature.return_type);
            }
            scan_block_effects(&decl.body, 0, &mut scan);
            scans.insert(decl.name.clone(), scan);
            params.insert(
                decl.name.clone(),
                decl.params
                    .iter()
                    .map(|param| (param.name.clone(), param.direction))
                    .collect(),
            );
            spans.push((decl.name.clone(), decl.span, decl.visibility));
        }

        let mut direct = HashSet::new();
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        let mut called = HashSet::new();
        for (name, scan) in &scans {
            if scan.direct_alloc {
                direct.insert(name.clone());
            }
            deps.insert(name.clone(), scan.unguarded_deps.clone());
            called.extend(scan.calls.iter().cloned());
        }

        let mut unguarded = direct;
        loop {
            let mut changed = false;
            for (name, callees) in &deps {
                if unguarded.contains(name) {
                    continue;
                }
                if callees.iter().any(|callee| unguarded.contains(callee)) {
                    unguarded.insert(name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        for (name, span, visibility) in &spans {
            // An entry point is main or any visible function that is not called
            // by any other function. Hidden functions are internal and can only
            // be called from within the same module.
            let entry =
                name == "main" || (*visibility == Visibility::Visible && !called.contains(name));
            if entry && unguarded.contains(name) {
                self.error(
                    *span,
                    format!("function '{name}' can allocate outside a 'protect' block"),
                );
            }
        }
        self.unguarded_allocators = unguarded;

        let mut consumption: HashMap<String, Vec<bool>> = HashMap::new();
        for (name, param_list) in &params {
            let scan = &scans[name];
            let flags = param_list
                .iter()
                .map(|(param, direction)| {
                    *direction != Direction::In && scan.moved_roots.contains(param)
                })
                .collect();
            consumption.insert(name.clone(), flags);
        }

        loop {
            let mut changed = false;
            for (name, scan) in &scans {
                let Some(param_list) = params.get(name) else {
                    continue;
                };
                for site in &scan.call_args {
                    let Some(callee_flags) = consumption.get(&site.callee).cloned() else {
                        continue;
                    };
                    let Some(callee_signature) = self.functions.get(&site.callee) else {
                        continue;
                    };
                    for (index, (_, root)) in site.args.iter().enumerate() {
                        let consumed = callee_flags.get(index).copied().unwrap_or(false);
                        if !consumed {
                            continue;
                        }
                        let direction = callee_signature
                            .params
                            .get(index)
                            .map(|param| param.direction)
                            .unwrap_or(Direction::In);
                        if !matches!(direction, Direction::Out | Direction::Inout) {
                            continue;
                        }
                        let Some(root) = root else {
                            continue;
                        };
                        let Some(position) = param_list.iter().position(|(param, _)| param == root)
                        else {
                            continue;
                        };
                        let flags = consumption.get_mut(name).expect("consumption flags");
                        if !flags[position] {
                            flags[position] = true;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        self.param_consumption = consumption;
    }

    // ------------------------------------------------------------------
    // Interface defaults
    // ------------------------------------------------------------------
    fn check_interface_defaults(&mut self, program: &Program) {
        for item in &program.items {
            let Item::Interface(decl) = item else {
                continue;
            };
            if !self.is_canonical_interface(&decl.name, decl.span) {
                continue;
            }

            for field in &decl.fields {
                let Some(default) = &field.default else {
                    continue;
                };
                let Some(info) = self.interface_field(&decl.name, &field.name) else {
                    continue;
                };
                // Skip duplicated fields: only the first declaration is kept.
                if info.span != field.span {
                    continue;
                }

                let value_ty = self.check_expr(default);
                if !self.assignable_expr(&info.ty, &value_ty, default) {
                    self.error(
                        default.span,
                        format!(
                            "default for field '{}' has type {}, expected {}",
                            field.name, value_ty, info.ty
                        ),
                    );
                }
                self.check_literal_range(default, &info.ty);
            }
        }
    }

    fn interface_field(&self, interface: &str, field: &str) -> Option<FieldInfo> {
        let info = self.interfaces.get(interface)?;
        let index = *info.field_index.get(field)?;
        info.fields.get(index).cloned()
    }

    // ------------------------------------------------------------------
    // Function bodies
    // ------------------------------------------------------------------
    fn check_functions(&mut self, program: &Program) {
        for item in &program.items {
            let Item::Function(decl) = item else {
                continue;
            };
            if self.is_canonical_function(&decl.name, decl.span) {
                self.check_function(decl);
            }
        }
    }

    fn check_function(&mut self, decl: &FunctionDecl) {
        if decl.external {
            return;
        }
        let Some(signature) = self.functions.get(&decl.name).cloned() else {
            return;
        };

        self.current_return = signature.return_type.clone();
        self.current_function = decl.name.clone();
        self.scopes.push(HashMap::new());
        let body_region = self.begin_region(RegionKind::Block);

        for (param, signature) in decl.params.iter().zip(signature.params.iter()) {
            let info = VarInfo {
                ty: signature.ty.clone(),
                mutable: signature.direction != Direction::In,
                initialized: signature.direction != Direction::Out,
                direction: Some(signature.direction),
                span: param.span,
                moved: HashSet::new(),
                home_region: None,
                taints: Vec::new(),
            };
            self.scopes
                .last_mut()
                .expect("function scope")
                .entry(param.name.clone())
                .or_insert(info);
        }

        self.reachable = true;
        self.check_statements(&decl.body.statements);
        if self.reachable && !matches!(signature.return_type, Ty::Void | Ty::Error) {
            self.error(
                decl.span,
                format!(
                    "function '{}' must return a value of type {} on all paths",
                    decl.name, signature.return_type
                ),
            );
        }

        self.end_region(body_region);
        self.scopes.pop();
        self.reachable = true;
        self.current_return = Ty::Void;
        self.current_function.clear();
    }

    /// Check a block and report whether control can fall out of it.
    ///
    /// State changes to outer variables persist, but the caller's reachability
    /// is restored so that branch handlers can combine paths explicitly.
    fn check_block(&mut self, block: &Block) -> bool {
        let outer = self.reachable;
        self.reachable = true;
        self.scopes.push(HashMap::new());
        let region = self.begin_region(RegionKind::Block);
        self.check_statements(&block.statements);
        let reachable = self.reachable;
        self.end_region(region);
        self.scopes.pop();
        self.reachable = outer;
        reachable
    }

    fn check_statements(&mut self, statements: &[Stmt]) {
        for statement in statements {
            self.check_statement(statement);
        }
    }

    fn check_statement(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let declared = ty.as_ref().map(|ty| self.resolve_type(ty, statement.span));
                let value_ty = value.as_ref().map(|value| self.check_expr(value));
                let value_regions = value
                    .as_ref()
                    .map(|value| self.value_regions(value))
                    .unwrap_or_default();

                if let (Some(value), Some(value_ty)) = (value.as_ref(), value_ty.as_ref()) {
                    self.reject_implicit_copy(value, value_ty);
                }

                let (resolved, initialized) = match (declared, value_ty) {
                    (Some(declared), Some(value_ty)) => {
                        let actual_expr = value.as_ref().expect("value type");
                        if !self.assignable_expr(&declared, &value_ty, actual_expr) {
                            self.error(
                                actual_expr.span,
                                format!(
                                    "cannot initialize '{}' of type {} with {}",
                                    name, declared, value_ty
                                ),
                            );
                        }
                        if let Some(value) = value.as_ref() {
                            self.check_literal_range(value, &declared);
                        }
                        (declared, true)
                    }
                    (Some(declared), None) => (declared, false),
                    (None, Some(value_ty)) => {
                        let inferred = value_ty.concretize();
                        if let Some(value) = value.as_ref() {
                            self.check_literal_range(value, &inferred);
                        }
                        (inferred, true)
                    }
                    (None, None) => (Ty::Error, false),
                };

                let carries_storage = self.type_contains_slice(&resolved);
                if carries_storage {
                    self.check_region_escape(
                        self.current_region(),
                        &value_regions,
                        value
                            .as_ref()
                            .map(|value| value.span)
                            .unwrap_or(statement.span),
                    );
                }
                self.declare_variable(
                    name,
                    VarInfo {
                        ty: resolved,
                        mutable: *mutable,
                        initialized,
                        direction: None,
                        span: statement.span,
                        moved: HashSet::new(),
                        home_region: self.current_region(),
                        taints: Vec::new(),
                    },
                );
                if carries_storage {
                    self.set_variable_regions(name, &value_regions);
                }
            }
            StmtKind::Assign { target, op, value } => {
                self.check_assignment(target, *op, value);
            }
            StmtKind::Expr(expr) => {
                self.check_expr(expr);
            }
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.check_condition(condition);
                let entry_reachable = self.reachable;
                let before = self.snapshot();
                let then_reachable = self.check_block(then_block);
                let then_state = self.snapshot();
                self.restore(&before);
                let else_reachable = match else_branch {
                    Some(ElseBranch::Block(block)) => self.check_block(block),
                    Some(ElseBranch::If(inner)) => {
                        self.check_statement(inner);
                        self.reachable
                    }
                    None => true,
                };
                let else_state = self.snapshot();
                match (then_reachable, else_reachable) {
                    (true, true) => {
                        self.merge_states(&then_state, &else_state);
                        self.reachable = entry_reachable;
                    }
                    (true, false) => {
                        self.restore(&then_state);
                        self.reachable = entry_reachable;
                    }
                    (false, true) => {
                        self.reachable = entry_reachable;
                    }
                    (false, false) => {
                        self.restore(&before);
                        self.reachable = false;
                    }
                }
            }
            StmtKind::While { condition, body } => {
                self.check_condition(condition);
                let entry_reachable = self.reachable;
                self.loop_depth += 1;
                let before = self.snapshot();
                let body_reachable = self.check_block(body);
                self.loop_depth -= 1;
                let after = self.snapshot();
                self.restore(&before);

                // A `while true` loop always executes at least once. If every
                // path through the body ends in `return` (or another
                // terminating statement), control cannot fall past the loop.
                let condition_is_true = matches!(&condition.kind, ExprKind::Bool(true));
                if condition_is_true && !body_reachable {
                    self.reachable = false;
                } else {
                    self.merge_states(&before, &after);
                    self.reachable = entry_reachable;
                }
            }
            StmtKind::ForRange {
                variable,
                start,
                end,
                body,
            } => {
                let start_ty = self.check_expr(start);
                let end_ty = self.check_expr(end);
                let variable_ty = self.check_range_bounds(&start_ty, &end_ty, start, end);

                let entry_reachable = self.reachable;
                self.loop_depth += 1;
                let before = self.snapshot();
                self.scopes.push(HashMap::new());
                let body_region = self.begin_region(RegionKind::Block);
                if variable != "_" {
                    self.declare_variable(
                        variable,
                        VarInfo {
                            ty: variable_ty,
                            mutable: false,
                            initialized: true,
                            direction: None,
                            span: statement.span,
                            moved: HashSet::new(),
                            home_region: self.current_region(),
                            taints: Vec::new(),
                        },
                    );
                }
                self.check_statements(&body.statements);
                self.end_region(body_region);
                self.scopes.pop();
                self.loop_depth -= 1;
                let after = self.snapshot();
                self.restore(&before);
                self.merge_states(&before, &after);
                self.reachable = entry_reachable;
            }
            StmtKind::ForEach {
                direction,
                variable,
                iterable,
                body,
            } => {
                let iterable_ty = if *direction == Direction::Inout {
                    let info = self.lvalue_info(iterable);
                    self.check_mutable_access(&info, iterable.span);
                    info.ty
                } else {
                    self.check_expr(iterable)
                };
                let element_ty = match &iterable_ty {
                    Ty::Slice(element) => (**element).clone(),
                    Ty::Error => Ty::Error,
                    other => {
                        self.error(
                            iterable.span,
                            format!("cannot iterate over type {other}, expected a slice"),
                        );
                        Ty::Error
                    }
                };

                let entry_reachable = self.reachable;
                self.loop_depth += 1;
                let before = self.snapshot();
                self.scopes.push(HashMap::new());
                let body_region = self.begin_region(RegionKind::Block);
                if variable != "_" {
                    self.declare_variable(
                        variable,
                        VarInfo {
                            ty: element_ty,
                            mutable: *direction == Direction::Inout,
                            initialized: true,
                            direction: Some(*direction),
                            span: statement.span,
                            moved: HashSet::new(),
                            home_region: self.current_region(),
                            taints: Vec::new(),
                        },
                    );
                }
                self.check_statements(&body.statements);
                self.end_region(body_region);
                self.scopes.pop();
                self.loop_depth -= 1;
                let after = self.snapshot();
                self.restore(&before);
                self.merge_states(&before, &after);
                self.reachable = entry_reachable;
            }
            StmtKind::Protect(block) => {
                let entry_reachable = self.reachable;
                let region = self.enter_protect();
                let block_reachable = self.check_block(block);
                self.leave_protect(region);
                self.reachable = entry_reachable && block_reachable;
            }
            StmtKind::Return(value) => {
                self.check_return(value, statement.span);
                self.reachable = false;
            }
            StmtKind::Continue => {
                if self.loop_depth == 0 {
                    self.error(statement.span, "'continue' is only allowed inside a loop");
                }
                self.reachable = false;
            }
            StmtKind::Break => {
                if self.loop_depth == 0 {
                    self.error(statement.span, "'break' is only allowed inside a loop");
                }
                self.reachable = false;
            }
        }
    }

    fn check_range_bounds(&mut self, start_ty: &Ty, end_ty: &Ty, start: &Expr, end: &Expr) -> Ty {
        let mut valid = true;
        if !start_ty.is_integer() && !start_ty.is_error() {
            self.error(
                start.span,
                format!("range start must be an integer, found {start_ty}"),
            );
            valid = false;
        }
        if !end_ty.is_integer() && !end_ty.is_error() {
            self.error(
                end.span,
                format!("range end must be an integer, found {end_ty}"),
            );
            valid = false;
        }
        if !valid || start_ty.is_error() || end_ty.is_error() {
            return Ty::Error;
        }

        match types::common_numeric(start_ty, end_ty) {
            Some(common) if common.is_integer() => {
                let common = common.concretize();
                self.check_literal_range(start, &common);
                self.check_literal_range(end, &common);
                common
            }
            _ => {
                self.error(
                    start.span,
                    format!("range bounds have incompatible types {start_ty} and {end_ty}"),
                );
                Ty::Error
            }
        }
    }

    /// Reject an integer constant that cannot be represented by the expected
    /// integer type. Non-constant expressions are handled by `assignable`.
    fn check_literal_range(&mut self, expr: &Expr, expected: &Ty) {
        let Some(value) = const_int(expr) else {
            return;
        };
        let (min, max) = match expected {
            Ty::I8 => (i8::MIN as i128, i8::MAX as i128),
            Ty::I16 => (i16::MIN as i128, i16::MAX as i128),
            Ty::I32 => (i32::MIN as i128, i32::MAX as i128),
            Ty::I64 => (i64::MIN as i128, i64::MAX as i128),
            Ty::U8 => (0, u8::MAX as i128),
            Ty::U16 => (0, u16::MAX as i128),
            Ty::U32 => (0, u32::MAX as i128),
            Ty::U64 => (0, u64::MAX as i128),
            _ => return,
        };

        if value < min || value > max {
            self.error(
                expr.span,
                format!("integer constant is out of range for {expected}"),
            );
        }
    }

    fn check_condition(&mut self, condition: &Expr) {
        let ty = self.check_expr(condition);
        if !matches!(ty, Ty::Bool | Ty::Error) {
            self.error(
                condition.span,
                format!("condition must be bool, found {ty}"),
            );
        }
    }

    fn check_return(&mut self, value: &Option<Expr>, span: Span) {
        let expected = self.current_return.clone();
        match (value, &expected) {
            (Some(value), Ty::Void) => {
                self.error(
                    value.span,
                    format!(
                        "function '{}' does not return a value",
                        self.current_function
                    ),
                );
                self.check_expr(value);
            }
            (None, Ty::Void) | (None, Ty::Error) => {}
            (None, _) => {
                self.error(
                    span,
                    format!(
                        "function '{}' must return a value of type {}",
                        self.current_function, expected
                    ),
                );
            }
            (Some(_), Ty::Error) => {
                self.check_expr(value.as_ref().expect("return value"));
            }
            (Some(value), _) => {
                let actual = self.check_expr(value);
                if !self.assignable_expr(&expected, &actual, value) {
                    self.error(
                        value.span,
                        format!(
                            "function '{}' returns {}, expected {}",
                            self.current_function, actual, expected
                        ),
                    );
                }
                self.check_literal_range(value, &expected);
                if self.type_contains_slice(&actual) {
                    let regions = self.value_regions(value);
                    let has_block_storage = regions
                        .iter()
                        .any(|region| self.region_kinds.get(*region) == Some(&RegionKind::Block));
                    if has_block_storage {
                        self.error(
                            value.span,
                            "cannot return a slice literal: its storage ends with the current block; bind it to a local slice first",
                        );
                    } else {
                        self.check_region_escape(None, &regions, value.span);
                    }
                }
                if is_composite(&actual)
                    && is_place(value)
                    && let (Some(root), Some(path)) = (place_root(value), place_path(value))
                {
                    let from_in = self
                        .lookup(&root)
                        .map(|var| var.direction == Some(Direction::In))
                        .unwrap_or(false);
                    if from_in {
                        self.error(
                            value.span,
                            format!(
                                "cannot return a value borrowed from '<-' parameter '{root}'; use clone(...)"
                            ),
                        );
                    } else {
                        // Returning a composite transfers ownership to the caller.
                        self.record_move(&root, &path);
                    }
                }
            }
        }
    }

    fn check_assignment(&mut self, target: &Expr, op: AssignOp, value: &Expr) {
        let info = self.lvalue_info(target);
        let value_ty = self.check_expr(value);

        match op {
            AssignOp::Assign => {
                self.reject_implicit_copy(value, &value_ty);
                let value_regions = self.value_regions(value);
                self.check_write(&info, true, target.span);
                if !self.assignable_expr(&info.ty, &value_ty, value) {
                    self.error(
                        value.span,
                        format!("cannot assign {} to {}", value_ty, info.ty),
                    );
                }
                self.check_literal_range(value, &info.ty);
                if self.type_contains_slice(&info.ty) {
                    let destination = self.home_region_of(&info);
                    self.check_region_escape(destination, &value_regions, value.span);
                    self.set_place_regions(&info, &value_regions);
                }
            }
            AssignOp::Add
            | AssignOp::Sub
            | AssignOp::Mul
            | AssignOp::Div
            | AssignOp::Mod
            | AssignOp::BitAnd
            | AssignOp::BitOr
            | AssignOp::BitXor
            | AssignOp::Shl
            | AssignOp::Shr => {
                self.check_mutable_access(&info, target.span);
                let integer_only = matches!(
                    op,
                    AssignOp::Mod
                        | AssignOp::BitAnd
                        | AssignOp::BitOr
                        | AssignOp::BitXor
                        | AssignOp::Shl
                        | AssignOp::Shr
                );
                if integer_only {
                    if !info.ty.is_integer() && !info.ty.is_error() {
                        self.error(
                            target.span,
                            format!(
                                "compound assignment requires an integer target, found {}",
                                info.ty
                            ),
                        );
                        return;
                    }
                    if !value_ty.is_integer() && !value_ty.is_error() {
                        self.error(
                            value.span,
                            format!(
                                "compound assignment requires an integer value, found {value_ty}"
                            ),
                        );
                        return;
                    }
                } else {
                    if !info.ty.is_numeric() && !info.ty.is_error() {
                        self.error(
                            target.span,
                            format!(
                                "compound assignment requires a numeric target, found {}",
                                info.ty
                            ),
                        );
                        return;
                    }
                    if !value_ty.is_numeric() && !value_ty.is_error() {
                        self.error(
                            value.span,
                            format!(
                                "compound assignment requires a numeric value, found {value_ty}"
                            ),
                        );
                        return;
                    }
                }
                if !info.ty.is_error() && !value_ty.is_error() {
                    match types::common_numeric(&info.ty, &value_ty) {
                        Some(common) => {
                            if !types::assignable(&info.ty, &common) {
                                self.error(
                                    value.span,
                                    format!("cannot assign {} to {}", common, info.ty),
                                );
                            }
                            self.check_literal_range(value, &info.ty);
                            if matches!(op, AssignOp::Shl | AssignOp::Shr) {
                                self.check_shift_count(value, &info.ty);
                            }
                        }
                        None => self.error(
                            value.span,
                            format!(
                                "compound assignment has incompatible types {} and {}",
                                info.ty, value_ty
                            ),
                        ),
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------
    fn check_expr(&mut self, expr: &Expr) -> Ty {
        match &expr.kind {
            ExprKind::Int(_) => Ty::IntLiteral,
            ExprKind::Float(_) => Ty::FloatLiteral,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::String(_) => Ty::Slice(Box::new(Ty::U8)),
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => self.check_slice_literal(element_type.as_ref(), elements, expr.span),
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(var) => {
                    let ty = var.ty.clone();
                    let initialized = var.initialized;
                    let moved = path_is_moved(&var.moved, "");
                    if !initialized {
                        self.error(expr.span, format!("use of uninitialized variable '{name}'"));
                    } else if moved {
                        self.error(expr.span, format!("use of moved value '{name}'"));
                    }
                    ty
                }
                None => {
                    self.error(expr.span, format!("unknown variable '{name}'"));
                    Ty::Error
                }
            },
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Neg => {
                    let ty = self.check_expr(operand);
                    if !ty.is_numeric() && !ty.is_error() {
                        self.error(
                            expr.span,
                            format!("unary '-' requires a numeric operand, found {ty}"),
                        );
                    }
                    ty
                }
                UnaryOp::Not => {
                    let ty = self.check_expr(operand);
                    if !matches!(ty, Ty::Bool | Ty::Error) {
                        self.error(
                            expr.span,
                            format!("'not' requires a bool operand, found {ty}"),
                        );
                    }
                    Ty::Bool
                }
                UnaryOp::BitNot => {
                    let ty = self.check_expr(operand);
                    if !ty.is_integer() && !ty.is_error() {
                        self.error(
                            expr.span,
                            format!("'~' requires an integer operand, found {ty}"),
                        );
                    }
                    ty
                }
            },
            ExprKind::Binary { op, left, right } => {
                let left_ty = self.check_expr(left);
                let right_ty = self.check_expr(right);
                if matches!(op, BinaryOp::Div | BinaryOp::Mod) && const_int(right) == Some(0) {
                    self.error(
                        right.span,
                        if *op == BinaryOp::Div {
                            "division by zero"
                        } else {
                            "remainder by zero"
                        },
                    );
                }
                if is_concrete_integer(&right_ty) {
                    self.check_literal_range(left, &right_ty);
                }
                if is_concrete_integer(&left_ty) {
                    self.check_literal_range(right, &left_ty);
                }
                if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                    self.check_shift_count(right, &left_ty);
                }
                self.check_binary(*op, &left_ty, &right_ty, expr.span)
            }
            ExprKind::InterfaceLiteral { name, fields } => {
                self.check_interface_literal(name, fields, expr.span)
            }
            ExprKind::Cast { expr: inner, ty } => {
                let target = self.resolve_type(ty, expr.span);
                let source = self.check_expr(inner);
                if !source.is_error()
                    && !target.is_error()
                    && !(source.is_numeric() && target.is_numeric())
                {
                    self.error(expr.span, format!("cannot cast {source} to {target}"));
                }
                target
            }
            ExprKind::Call { callee, args } => self.check_call(callee, args, expr.span),
            ExprKind::Alloc {
                target,
                element_type,
                count,
            } => self.check_alloc(target, element_type, count, expr.span),
            ExprKind::Move(value) => self.check_move(value),
            ExprKind::Copy(value) => self.check_copy(value),
            ExprKind::Member { base, field } => {
                let base_ty = self.check_base_type(base);
                let ty = self.member_type(&base_ty, field, expr.span);
                self.check_place_state(expr, expr.span);
                ty
            }
            ExprKind::Index { base, index } => {
                let base_ty = self.check_base_type(base);
                let index_ty = self.check_expr(index);
                let ty = self.check_index(&base_ty, index, &index_ty, expr.span);
                self.check_place_state(expr, expr.span);
                ty
            }
        }
    }

    /// Type-check `[a, b]`, `[T: a, b]` or `[T:]`.
    fn check_slice_literal(
        &mut self,
        element_type: Option<&Type>,
        elements: &[Expr],
        span: Span,
    ) -> Ty {
        if let Some(element_type) = element_type {
            let element_ty = self.resolve_type(element_type, span);
            for element in elements {
                let actual = self.check_expr(element);
                if !self.assignable_expr(&element_ty, &actual, element) {
                    self.error(
                        element.span,
                        format!("slice literal element has type {actual}, expected {element_ty}"),
                    );
                }
                self.check_literal_range(element, &element_ty);
                self.reject_implicit_copy(element, &actual);
            }
            return Ty::Slice(Box::new(element_ty));
        }

        let mut types_in_literal: Vec<Ty> = Vec::with_capacity(elements.len());
        let mut inferred: Option<Ty> = None;
        for element in elements {
            let actual = self.check_expr(element);
            types_in_literal.push(actual.clone());
            if actual.is_error() {
                continue;
            }
            inferred = Some(match inferred {
                None => actual,
                Some(previous) => {
                    if let Some(common) = types::common_numeric(&previous, &actual) {
                        common
                    } else if previous == actual {
                        previous
                    } else {
                        self.error(
                            element.span,
                            format!(
                                "slice literal elements have incompatible types {previous} and {actual}"
                            ),
                        );
                        Ty::Error
                    }
                }
            });
        }

        let element_ty = match inferred {
            Some(ty) => ty.concretize(),
            None => {
                if elements.is_empty() {
                    self.error(
                        span,
                        "cannot infer the element type of an empty slice literal; write [T:]",
                    );
                }
                Ty::Error
            }
        };

        for (element, actual) in elements.iter().zip(types_in_literal.iter()) {
            self.check_literal_range(element, &element_ty);
            self.reject_implicit_copy(element, actual);
        }

        Ty::Slice(Box::new(element_ty))
    }

    /// Type of the base of a member/index access. Places are read through
    /// `lvalue_info` so the whole ownership path is considered, but the read
    /// itself is reported by [`Checker::check_place_state`].
    fn check_base_type(&mut self, base: &Expr) -> Ty {
        if is_place(base) {
            self.lvalue_info(base).ty
        } else {
            self.check_expr(base)
        }
    }

    fn check_place_state(&mut self, expr: &Expr, span: Span) {
        let (Some(root), Some(path)) = (place_root(expr), place_path(expr)) else {
            return;
        };
        let Some((initialized, moved)) = self
            .lookup(&root)
            .map(|var| (var.initialized, path_is_moved(&var.moved, &path)))
        else {
            return;
        };
        if !initialized {
            self.error(span, format!("use of uninitialized variable '{root}'"));
        } else if moved {
            self.error(span, format!("use of moved value '{root}'"));
        }
    }

    fn check_interface_literal(&mut self, name: &str, fields: &[FieldInit], span: Span) -> Ty {
        if !self.interfaces.contains_key(name) {
            self.error(span, format!("unknown shape '{name}'"));
            for field in fields {
                self.check_expr(&field.value);
            }
            return Ty::Error;
        }

        let mut seen = HashSet::new();
        for field in fields {
            let value_ty = self.check_expr(&field.value);
            let duplicate = !seen.insert(field.name.clone());
            match self.interface_field(name, &field.name) {
                Some(info) => {
                    if !self.assignable_expr(&info.ty, &value_ty, &field.value) {
                        self.error(
                            field.value.span,
                            format!(
                                "field '{}' has type {}, expected {}",
                                field.name, value_ty, info.ty
                            ),
                        );
                    }
                    self.check_literal_range(&field.value, &info.ty);
                    self.reject_implicit_copy(&field.value, &value_ty);
                }
                None => self.error(
                    field.value.span,
                    format!("unknown field '{}' in literal '{name}'", field.name),
                ),
            }
            if duplicate {
                self.error(
                    field.value.span,
                    format!("duplicate field '{}' in literal '{name}'", field.name),
                );
            }
        }

        let info = self.interfaces.get(name).cloned().expect("known interface");
        for field in &info.fields {
            if !field.has_default && !fields.iter().any(|init| init.name == field.name) {
                self.error(
                    span,
                    format!("missing field '{}' in literal '{name}'", field.name),
                );
            }
        }

        Ty::Named(name.to_string())
    }

    fn check_shift_count(&mut self, value: &Expr, ty: &Ty) {
        let Some(count) = const_int(value) else {
            return;
        };
        let bits = integer_bits(ty).unwrap_or(32);
        if count < 0 || count as u128 >= bits {
            self.error(
                value.span,
                format!("shift count {count} is out of range for {ty} (0..{bits})"),
            );
        }
    }

    fn check_binary(&mut self, op: BinaryOp, left: &Ty, right: &Ty, span: Span) -> Ty {
        match op {
            BinaryOp::Or | BinaryOp::And => {
                if !matches!(left, Ty::Bool | Ty::Error) {
                    self.error(
                        span,
                        format!("operator requires bool operands, found {left}"),
                    );
                }
                if !matches!(right, Ty::Bool | Ty::Error) {
                    self.error(
                        span,
                        format!("operator requires bool operands, found {right}"),
                    );
                }
                Ty::Bool
            }
            BinaryOp::Eq | BinaryOp::Neq => {
                if left.is_error() || right.is_error() {
                    return Ty::Bool;
                }
                // Equality is defined for scalars and for byte slices (which is
                // how strings are represented). Other composites are compared
                // field by field in user code; C cannot compare structs with
                // `==`.
                let byte_slice = |ty: &Ty| matches!(ty, Ty::Slice(element) if **element == Ty::U8);
                let scalar = |ty: &Ty| ty.is_numeric() || matches!(ty, Ty::Bool);
                let comparable = (scalar(left)
                    && scalar(right)
                    && (left == right || types::common_numeric(left, right).is_some()))
                    || (byte_slice(left) && byte_slice(right));
                if !comparable {
                    self.error(
                        span,
                        format!(
                            "cannot compare {left} with {right}; only scalars and [u8] values support == and !="
                        ),
                    );
                }
                Ty::Bool
            }
            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                if left.is_error() || right.is_error() {
                    return Ty::Bool;
                }
                if types::common_numeric(left, right).is_none() {
                    self.error(
                        span,
                        format!("operator requires numeric operands, found {left} and {right}"),
                    );
                }
                Ty::Bool
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                if left.is_error() || right.is_error() {
                    return Ty::Error;
                }
                match types::common_numeric(left, right) {
                    Some(common) => common,
                    None => {
                        self.error(
                            span,
                            format!("operator requires numeric operands, found {left} and {right}"),
                        );
                        Ty::Error
                    }
                }
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                if left.is_error() || right.is_error() {
                    return Ty::Error;
                }
                if !left.is_integer() || !right.is_integer() {
                    self.error(
                        span,
                        format!(
                            "bitwise operator requires integer operands, found {left} and {right}"
                        ),
                    );
                    return Ty::Error;
                }
                types::common_numeric(left, right).unwrap_or(Ty::Error)
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                if left.is_error() || right.is_error() {
                    return Ty::Error;
                }
                if !left.is_integer() || !right.is_integer() {
                    self.error(
                        span,
                        format!("shift requires integer operands, found {left} and {right}"),
                    );
                    return Ty::Error;
                }
                left.clone()
            }
            BinaryOp::Mod => {
                if left.is_error() || right.is_error() {
                    return Ty::Error;
                }
                if !left.is_integer() || !right.is_integer() {
                    self.error(
                        span,
                        format!("'%' requires integer operands, found {left} and {right}"),
                    );
                    return Ty::Error;
                }
                types::common_numeric(left, right).unwrap_or(Ty::Error)
            }
        }
    }

    fn check_call(&mut self, callee: &Expr, args: &[CallArg], span: Span) -> Ty {
        let ExprKind::Ident(name) = &callee.kind else {
            self.error(callee.span, "only named functions can be called");
            for arg in args {
                self.check_unmatched_arg(arg);
            }
            return Ty::Error;
        };

        // `len(xs)` is a compiler intrinsic: slices carry their length, so no
        // library function is needed. The name is reserved (`LOI_SPEC.md` 5.9).
        if name == "len" {
            if args.len() != 1 {
                self.error(
                    span,
                    format!("len expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    self.check_unmatched_arg(arg);
                }
                return Ty::U64;
            }
            let arg = &args[0];
            let ty = self.check_expr(&arg.value);
            if !matches!(ty, Ty::Slice(_) | Ty::Error) {
                self.error(arg.value.span, format!("len expects a slice, found {ty}"));
            }
            return Ty::U64;
        }

        // `print_int` and `print_float` are generic over the numeric width so
        // callers do not have to cast every value to i64/f64 by hand.
        if matches!(name.as_str(), "print_int" | "print_float") {
            if args.len() != 1 {
                self.error(
                    span,
                    format!("{name} expects 1 argument, found {}", args.len()),
                );
                for arg in args {
                    self.check_unmatched_arg(arg);
                }
                return Ty::Void;
            }
            let arg = &args[0];
            if arg.direction != Direction::In {
                self.error(arg.value.span, format!("{name} expects a '<-' argument"));
            }
            let ty = self.check_expr(&arg.value);
            if name == "print_int" && !ty.is_integer() && !ty.is_error() {
                self.error(
                    arg.value.span,
                    format!("print_int expects an integer, found {ty}"),
                );
            }
            if name == "print_float" && !ty.is_float() && !ty.is_error() {
                self.error(
                    arg.value.span,
                    format!("print_float expects a float, found {ty}"),
                );
            }
            return Ty::Void;
        }

        let Some(signature) = self.functions.get(name).cloned() else {
            self.error(callee.span, format!("unknown function '{name}'"));
            for arg in args {
                self.check_unmatched_arg(arg);
            }
            return Ty::Error;
        };

        if args.len() != signature.params.len() {
            self.error(
                span,
                format!(
                    "function '{}' expects {} argument(s), found {}",
                    name,
                    signature.params.len(),
                    args.len()
                ),
            );
        }

        let may_allocate = self.unguarded_allocators.contains(name);
        let mut out_targets = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            match signature.params.get(index) {
                Some(param) => {
                    if let Some(info) = self.check_call_arg(arg, param, may_allocate) {
                        out_targets.push((index, info));
                    }
                }
                None => self.check_unmatched_arg(arg),
            }
        }

        self.check_call_aliasing(name, args, &signature);
        self.apply_parameter_consumption(name, args, &signature);
        self.propagate_call_regions(name, args, &signature, &out_targets);

        signature.return_type
    }

    /// Carry protect-region taints from consumed `inout` arguments to `out`
    /// arguments of the same call.
    ///
    /// A callee that moves data out of an `inout` parameter may store it into
    /// an `out` parameter. Without knowing which output receives it, all
    /// slice-containing `out` targets are conservatively tainted. Escape is
    /// checked against each target's home region.
    fn propagate_call_regions(
        &mut self,
        name: &str,
        args: &[CallArg],
        signature: &FunctionSig,
        out_targets: &[(usize, LvalueInfo)],
    ) {
        if out_targets.is_empty() {
            return;
        }
        let Some(consumed) = self.param_consumption.get(name).cloned() else {
            return;
        };

        let mut transfer = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            if !consumed.get(index).copied().unwrap_or(false) {
                continue;
            }
            let direction = signature
                .params
                .get(index)
                .map(|param| param.direction)
                .unwrap_or(arg.direction);
            if direction == Direction::Inout {
                transfer.extend(self.value_regions(&arg.value));
            }
        }
        transfer.sort_unstable();
        transfer.dedup();
        if transfer.is_empty() {
            return;
        }

        for (index, info) in out_targets {
            if !self.type_contains_slice(&info.ty) {
                continue;
            }
            let Some(arg) = args.get(*index) else {
                continue;
            };
            let destination = self.home_region_of(info);
            self.check_region_escape(destination, &transfer, arg.value.span);
            self.add_place_regions(info, &transfer);
        }
    }

    /// Move `out`/`inout` arguments that the callee consumes.
    ///
    /// Consumption was summarized before body checking. Marking the caller's
    /// place moved keeps ownership tracked across the call, so a value moved
    /// out by the callee cannot be reused by the caller.
    fn apply_parameter_consumption(
        &mut self,
        name: &str,
        args: &[CallArg],
        signature: &FunctionSig,
    ) {
        let Some(consumed) = self.param_consumption.get(name).cloned() else {
            return;
        };
        for (index, arg) in args.iter().enumerate() {
            if !consumed.get(index).copied().unwrap_or(false) {
                continue;
            }
            let direction = signature
                .params
                .get(index)
                .map(|param| param.direction)
                .unwrap_or(arg.direction);
            if !matches!(direction, Direction::Out | Direction::Inout) {
                continue;
            }
            let (Some(root), Some(path)) = (place_root(&arg.value), place_path(&arg.value)) else {
                continue;
            };
            self.record_move(&root, &path);
        }
    }

    /// Enforce exclusive `out`/`inout` arguments.
    ///
    /// Passing the same variable twice as `in` is fine because both are reads.
    /// An `out` or `inout` argument must not share its root variable with any
    /// other argument in the same call.
    fn check_call_aliasing(&mut self, name: &str, args: &[CallArg], signature: &FunctionSig) {
        let all_accesses: Vec<Vec<(String, String)>> = args
            .iter()
            .map(|arg| {
                let mut accesses = Vec::new();
                collect_accesses(&arg.value, &mut accesses);
                accesses
            })
            .collect();

        for (index, arg) in args.iter().enumerate() {
            let direction = signature
                .params
                .get(index)
                .map(|param| param.direction)
                .unwrap_or(arg.direction);
            if !matches!(direction, Direction::Out | Direction::Inout) {
                continue;
            }
            let Some(root) = place_root(&arg.value) else {
                continue;
            };
            let Some(path) = place_path(&arg.value) else {
                continue;
            };
            let conflict = all_accesses
                .iter()
                .enumerate()
                .any(|(other_index, accesses)| {
                    other_index != index
                        && accesses.iter().any(|(other_root, other_path)| {
                            *other_root == root && paths_overlap(other_path, &path)
                        })
                });
            if conflict {
                self.error(
                    arg.value.span,
                    format!(
                        "cannot pass '{root}' as '{}' in the call to '{name}' because it is also used by another argument",
                        direction_name(direction)
                    ),
                );
            }
        }
    }

    fn check_unmatched_arg(&mut self, arg: &CallArg) {
        if arg.direction == Direction::In {
            self.check_expr(&arg.value);
        } else {
            let _ = self.lvalue_info(&arg.value);
        }
    }

    fn check_call_arg(
        &mut self,
        arg: &CallArg,
        param: &ParamSig,
        may_allocate: bool,
    ) -> Option<LvalueInfo> {
        if arg.direction != param.direction {
            self.error(
                arg.value.span,
                format!(
                    "argument '{}' expects direction '{}', found '{}'",
                    param.name,
                    direction_name(param.direction),
                    direction_name(arg.direction)
                ),
            );
        }

        match param.direction {
            Direction::In => {
                let actual = self.check_expr(&arg.value);
                if !self.assignable_expr(&param.ty, &actual, &arg.value) {
                    self.error(
                        arg.value.span,
                        format!(
                            "argument '{}' has type {}, expected {}",
                            param.name, actual, param.ty
                        ),
                    );
                }
                self.check_literal_range(&arg.value, &param.ty);
                None
            }
            Direction::Out => {
                let info = self.lvalue_info(&arg.value);
                self.check_write(&info, true, arg.value.span);
                if !info.ty.is_error() && !param.ty.is_error() && info.ty != param.ty {
                    self.error(
                        arg.value.span,
                        format!(
                            "argument '{}' has type {}, expected {}",
                            param.name, info.ty, param.ty
                        ),
                    );
                }
                if may_allocate
                    && self.type_contains_slice(&info.ty)
                    && let Some(region) = self.current_protect_region()
                {
                    let destination = self.home_region_of(&info);
                    self.check_region_escape(destination, &[region], arg.value.span);
                    self.set_place_regions(&info, &[region]);
                }
                Some(info)
            }
            Direction::Inout => {
                let info = self.lvalue_info(&arg.value);
                self.check_mutable_access(&info, arg.value.span);
                if !info.ty.is_error() && !param.ty.is_error() && info.ty != param.ty {
                    self.error(
                        arg.value.span,
                        format!(
                            "argument '{}' has type {}, expected {}",
                            param.name, info.ty, param.ty
                        ),
                    );
                }
                None
            }
        }
    }

    fn check_alloc(&mut self, target: &Expr, element_type: &Type, count: &Expr, span: Span) -> Ty {
        let info = self.lvalue_info(target);
        self.check_write(&info, true, target.span);
        let element = self.resolve_type(element_type, span);

        match &info.ty {
            Ty::Slice(actual) => {
                if !actual.is_error() && !element.is_error() && **actual != element {
                    self.error(
                        span,
                        format!(
                            "alloc element type {} does not match slice element type {}",
                            element, actual
                        ),
                    );
                }
            }
            Ty::Error => {}
            other => self.error(
                target.span,
                format!("alloc target must be a slice, found {other}"),
            ),
        }

        let count_ty = self.check_expr(count);
        if !count_ty.is_integer() && !count_ty.is_error() {
            self.error(
                count.span,
                format!("alloc count must be an integer, found {count_ty}"),
            );
        }

        if self.type_contains_slice(&info.ty)
            && let Some(region) = self.current_protect_region()
        {
            let destination = self.home_region_of(&info);
            self.check_region_escape(destination, &[region], target.span);
            self.set_place_regions(&info, &[region]);
        }

        Ty::Void
    }

    fn check_move(&mut self, value: &Expr) -> Ty {
        if !is_place(value) {
            self.error(value.span, "move requires a variable, field or element");
            return Ty::Error;
        }

        let info = self.lvalue_info(value);
        let Some(root) = info.root.clone() else {
            return info.ty;
        };
        let Some(path) = info.path.clone() else {
            return info.ty;
        };
        let Some(var) = self.lookup(&root) else {
            return info.ty;
        };

        let direction = var.direction;
        let initialized = var.initialized;
        let moved = path_is_moved(&var.moved, &path);

        if direction == Some(Direction::In) {
            self.error(
                value.span,
                format!("cannot give from '<-' parameter '{root}'"),
            );
        } else if !initialized {
            self.error(
                value.span,
                format!("cannot give from uninitialized variable '{root}'"),
            );
        } else if moved {
            self.error(value.span, format!("use of moved value '{root}'"));
        } else {
            self.record_move(&root, &path);
        }
        info.ty
    }

    fn check_copy(&mut self, value: &Expr) -> Ty {
        let ty = self.check_expr(value);
        if self.type_contains_slice(&ty) {
            self.error(
                value.span,
                format!("cannot copy a value of type {ty} because it contains a slice"),
            );
        }
        ty
    }

    // ------------------------------------------------------------------
    // Lvalues, members and indexes
    // ------------------------------------------------------------------
    fn lvalue_info(&mut self, expr: &Expr) -> LvalueInfo {
        match &expr.kind {
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(info) => LvalueInfo {
                    ty: info.ty.clone(),
                    root: Some(name.clone()),
                    path: Some(String::new()),
                },
                None => {
                    self.error(expr.span, format!("unknown variable '{name}'"));
                    LvalueInfo {
                        ty: Ty::Error,
                        root: None,
                        path: None,
                    }
                }
            },
            ExprKind::Member { base, field } => {
                let base_info = self.lvalue_info(base);
                let ty = self.member_type(&base_info.ty, field, expr.span);
                LvalueInfo {
                    ty,
                    root: base_info.root,
                    path: base_info.path.map(|path| append_path(&path, field)),
                }
            }
            ExprKind::Index { base, index } => {
                let base_info = self.lvalue_info(base);
                let index_ty = self.check_expr(index);
                let ty = self.check_index(&base_info.ty, index, &index_ty, expr.span);
                LvalueInfo {
                    ty,
                    root: base_info.root,
                    path: base_info.path.map(|path| append_path(&path, "[]")),
                }
            }
            _ => {
                self.error(expr.span, "invalid assignment target");
                LvalueInfo {
                    ty: Ty::Error,
                    root: None,
                    path: None,
                }
            }
        }
    }

    fn member_type(&mut self, base: &Ty, field: &str, span: Span) -> Ty {
        match base {
            Ty::Slice(_) if field == "len" => Ty::U64,
            Ty::Named(name) => match self.interfaces.get(name) {
                Some(info) => match info.field_index.get(field) {
                    Some(&index) => info.fields[index].ty.clone(),
                    None => {
                        self.error(span, format!("shape '{name}' has no field '{field}'"));
                        Ty::Error
                    }
                },
                None => Ty::Error,
            },
            Ty::Error => Ty::Error,
            other => {
                self.error(
                    span,
                    format!("cannot access field '{field}' on type {other}"),
                );
                Ty::Error
            }
        }
    }

    fn check_index(&mut self, base: &Ty, index: &Expr, index_ty: &Ty, span: Span) -> Ty {
        let element = match base {
            Ty::Slice(element) => (**element).clone(),
            Ty::Error => Ty::Error,
            other => {
                self.error(span, format!("cannot index type {other}"));
                Ty::Error
            }
        };
        if !index_ty.is_integer() && !index_ty.is_error() {
            self.error(
                index.span,
                format!("index must be an integer, found {index_ty}"),
            );
        }
        // A negative slice index can never be valid. The runtime helper would
        // catch it as a huge `size_t` and abort, but a compile-time diagnostic
        // is clearer and does not depend on the runtime check.
        if const_int(index).is_some_and(|value| value < 0) {
            self.error(index.span, "slice index cannot be negative");
        }
        element
    }

    // ------------------------------------------------------------------
    // Ownership, definite assignment and branch state
    // ------------------------------------------------------------------
    fn require_lvalue(&mut self, info: &LvalueInfo, span: Span) -> Option<(String, String)> {
        let Some(name) = info.root.clone() else {
            self.error(span, "assignment target is not a variable");
            return None;
        };
        let Some(path) = info.path.clone() else {
            self.error(span, "assignment target is not a variable");
            return None;
        };
        Some((name, path))
    }

    /// Check a write to a place and update the variable's ownership state.
    ///
    /// A whole-variable write reinitializes every moved path. A field or element
    /// write is allowed only when the written path is alive, or when it exactly
    /// reinitializes a moved path (for example writing back `w.particles` after
    /// moving it out).
    fn check_write(&mut self, info: &LvalueInfo, initializing: bool, span: Span) -> bool {
        let Some((name, path)) = self.require_lvalue(info, span) else {
            return false;
        };
        let Some(var) = self.lookup(&name) else {
            return false;
        };

        let direction = var.direction;
        let mutable = var.mutable;
        let initialized = var.initialized;
        let moved_here = path_is_moved(&var.moved, &path);
        let reinitializes = !path.is_empty()
            && var
                .moved
                .iter()
                .any(|moved| !moved.is_empty() && path_is_ancestor(&path, moved));

        if direction == Some(Direction::In) {
            self.error(span, format!("cannot write to '<-' parameter '{name}'"));
            return false;
        }

        if !initialized && !initializing {
            self.error(span, format!("use of uninitialized variable '{name}'"));
            return false;
        }

        if initialized && !mutable {
            self.error(
                span,
                format!("cannot assign to immutable variable '{name}'"),
            );
            return false;
        }

        if moved_here && !path.is_empty() && !reinitializes {
            self.error(span, format!("cannot write to moved value '{name}'"));
            return false;
        }

        self.mark_written(&name, &path);
        true
    }

    /// Check a read/write borrow of a place (`inout` argument, `for inout`).
    fn check_mutable_access(&mut self, info: &LvalueInfo, span: Span) -> bool {
        let Some((name, path)) = self.require_lvalue(info, span) else {
            return false;
        };
        let Some(var) = self.lookup(&name) else {
            return false;
        };

        let direction = var.direction;
        let mutable = var.mutable;
        let initialized = var.initialized;
        let moved = path_is_moved(&var.moved, &path);

        if direction == Some(Direction::In) {
            self.error(span, format!("cannot write to '<-' parameter '{name}'"));
            return false;
        }
        if !initialized {
            self.error(span, format!("use of uninitialized variable '{name}'"));
            return false;
        }
        if moved {
            self.error(span, format!("use of moved value '{name}'"));
            return false;
        }
        if !mutable {
            self.error(
                span,
                format!("cannot assign to immutable variable '{name}'"),
            );
            return false;
        }
        true
    }

    fn mark_written(&mut self, name: &str, path: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.initialized = true;
                if path.is_empty() {
                    info.moved.clear();
                } else {
                    info.moved.retain(|moved| !path_is_ancestor(path, moved));
                }
                info.taints
                    .retain(|taint| !paths_overlap(&taint.path, path));
                return;
            }
        }
    }

    fn record_move(&mut self, name: &str, path: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.moved.retain(|moved| !path_is_ancestor(path, moved));
                info.moved.insert(path.to_string());
                return;
            }
        }
    }

    // ------------------------------------------------------------------
    // protect regions and escape analysis
    // ------------------------------------------------------------------
    fn current_region(&self) -> Option<usize> {
        self.region_stack.last().copied()
    }

    /// Innermost active `protect` region, ignoring lexical blocks nested inside
    /// it. `alloc` outside every `protect` is not tracked as arena storage: it
    /// is either rejected by the unguarded-allocator analysis or belongs to the
    /// caller's arena.
    fn current_protect_region(&self) -> Option<usize> {
        self.region_stack
            .iter()
            .rev()
            .copied()
            .find(|region| self.region_kinds.get(*region) == Some(&RegionKind::Protect))
    }

    fn begin_region(&mut self, kind: RegionKind) -> usize {
        let id = self.region_parents.len();
        self.region_parents.push(self.current_region());
        self.region_kinds.push(kind);
        self.region_stack.push(id);
        id
    }

    /// Close `region`, asserting that it is the innermost active region.
    ///
    /// Taints that point into this region are dropped: any value that could
    /// still be reached after the block ends was already rejected as an escape
    /// at the assignment that created it.
    fn end_region(&mut self, region: usize) {
        debug_assert_eq!(self.current_region(), Some(region));
        self.region_stack.pop();
        for scope in self.scopes.iter_mut().rev() {
            for info in scope.values_mut() {
                info.taints.retain(|taint| taint.region != region);
            }
        }
    }

    /// True when `home` is `ancestor` or a nested region inside it.
    ///
    /// The function scope is represented by `None` and is never inside a
    /// lexical `protect` region.
    fn region_within(&self, home: Option<usize>, ancestor: usize) -> bool {
        let Some(mut region) = home else {
            return false;
        };
        loop {
            if region == ancestor {
                return true;
            }
            match self.region_parents.get(region).copied().flatten() {
                Some(parent) => region = parent,
                None => return false,
            }
        }
    }

    /// Reject storing block- or `protect`-local storage outside the region
    /// that owns it.
    fn check_region_escape(&mut self, destination: Option<usize>, regions: &[usize], span: Span) {
        for region in regions {
            if !self.region_within(destination, *region) {
                let message = match self.region_kinds.get(*region) {
                    Some(RegionKind::Protect) => {
                        "value allocated inside 'protect' escapes the block".to_string()
                    }
                    _ => "slice literal storage escapes the block that created it; keep it in a variable declared in the same block".to_string(),
                };
                self.error(span, message);
                return;
            }
        }
    }

    /// Allocate a fresh protect region for a `protect` body.
    fn enter_protect(&mut self) -> usize {
        self.begin_region(RegionKind::Protect)
    }

    fn leave_protect(&mut self, region: usize) {
        self.end_region(region);
    }

    /// Regions of protect-local allocations reachable through an expression.
    fn value_regions(&self, expr: &Expr) -> Vec<usize> {
        let mut regions = Vec::new();
        self.collect_value_regions(expr, &mut regions);
        regions.sort_unstable();
        regions.dedup();
        regions
    }

    fn collect_value_regions(&self, expr: &Expr, regions: &mut Vec<usize>) {
        match &expr.kind {
            ExprKind::Ident(name) => {
                if let Some(var) = self.lookup(name) {
                    regions.extend(var.taints.iter().map(|taint| taint.region));
                }
            }
            ExprKind::Member { base, .. } | ExprKind::Index { base, .. } => {
                if let (Some(root), Some(path)) = (place_root(expr), place_path(expr)) {
                    if let Some(var) = self.lookup(&root) {
                        for taint in &var.taints {
                            if paths_overlap(&taint.path, &path) {
                                regions.push(taint.region);
                            }
                        }
                    }
                } else {
                    self.collect_value_regions(base, regions);
                    if let ExprKind::Index { index, .. } = &expr.kind {
                        self.collect_value_regions(index, regions);
                    }
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Cast { expr: operand, .. } => {
                self.collect_value_regions(operand, regions);
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_value_regions(left, regions);
                self.collect_value_regions(right, regions);
            }
            ExprKind::Call { callee, args } => {
                if let ExprKind::Ident(name) = &callee.kind
                    && matches!(name.as_str(), "read_file" | "read_stdin" | "read_line")
                    && let Some(region) = self.current_protect_region()
                {
                    regions.push(region);
                }
                // A callee's allocations belong to the caller's protect region,
                // not to a lexical protect inside the callee. Escape through a
                // callee is diagnosed at the callee's `return`/`out`.
                //
                // Composite results may also carry data moved out of a consumed
                // `inout` argument; propagate those regions conservatively.
                if let ExprKind::Ident(name) = &callee.kind
                    && let Some(signature) = self.functions.get(name)
                    && self.type_contains_slice(&signature.return_type)
                    && let Some(consumed) = self.param_consumption.get(name)
                {
                    for (index, arg) in args.iter().enumerate() {
                        if !consumed.get(index).copied().unwrap_or(false) {
                            continue;
                        }
                        let direction = signature
                            .params
                            .get(index)
                            .map(|param| param.direction)
                            .unwrap_or(arg.direction);
                        if direction == Direction::Inout {
                            self.collect_value_regions(&arg.value, regions);
                        }
                    }
                }
            }
            ExprKind::Alloc { .. } => {}
            ExprKind::Move(value) | ExprKind::Copy(value) => {
                self.collect_value_regions(value, regions);
            }
            ExprKind::InterfaceLiteral { fields, .. } => {
                for field in fields {
                    self.collect_value_regions(&field.value, regions);
                }
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {}
            ExprKind::SliceLiteral { elements, .. } => {
                // The backing array has automatic storage in the enclosing
                // lexical block. String literals are static and deliberately
                // do not reach this branch.
                if let Some(region) = self.current_region() {
                    regions.push(region);
                }
                for element in elements {
                    self.collect_value_regions(element, regions);
                }
            }
        }
    }

    /// Attach the regions of `value` to a destination place.
    fn set_place_regions(&mut self, place: &LvalueInfo, regions: &[usize]) {
        let Some(root) = place.root.as_deref() else {
            return;
        };
        let Some(path) = place.path.as_deref() else {
            return;
        };
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(root) {
                info.taints
                    .retain(|taint| !paths_overlap(&taint.path, path));
                for region in regions {
                    info.taints.push(RegionTaint {
                        path: path.to_string(),
                        region: *region,
                    });
                }
                return;
            }
        }
    }

    /// Add regions to a place without discarding regions already attached.
    fn add_place_regions(&mut self, place: &LvalueInfo, regions: &[usize]) {
        let Some(root) = place.root.as_deref() else {
            return;
        };
        let Some(path) = place.path.as_deref() else {
            return;
        };
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(root) {
                for region in regions {
                    if !info
                        .taints
                        .iter()
                        .any(|taint| taint.path == path && taint.region == *region)
                    {
                        info.taints.push(RegionTaint {
                            path: path.to_string(),
                            region: *region,
                        });
                    }
                }
                return;
            }
        }
    }

    fn set_variable_regions(&mut self, name: &str, regions: &[usize]) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.taints.clear();
                for region in regions {
                    info.taints.push(RegionTaint {
                        path: String::new(),
                        region: *region,
                    });
                }
                return;
            }
        }
    }

    fn home_region_of(&self, info: &LvalueInfo) -> Option<usize> {
        info.root
            .as_deref()
            .and_then(|root| self.lookup(root))
            .and_then(|var| var.home_region)
    }

    fn reject_implicit_copy(&mut self, value: &Expr, value_ty: &Ty) {
        if is_place(value) && is_composite(value_ty) {
            self.error(
                value.span,
                format!("cannot copy {value_ty} implicitly; use give(...) or clone(...)"),
            );
        }
    }

    fn snapshot(&self) -> Vec<HashMap<String, ControlState>> {
        self.scopes
            .iter()
            .map(|scope| {
                scope
                    .iter()
                    .map(|(name, info)| {
                        (
                            name.clone(),
                            ControlState {
                                initialized: info.initialized,
                                moved: info.moved.clone(),
                                taints: info.taints.clone(),
                            },
                        )
                    })
                    .collect()
            })
            .collect()
    }

    fn restore(&mut self, snapshot: &[HashMap<String, ControlState>]) {
        for (scope, saved) in self.scopes.iter_mut().zip(snapshot) {
            for (name, state) in saved {
                if let Some(info) = scope.get_mut(name) {
                    info.initialized = state.initialized;
                    info.moved = state.moved.clone();
                    info.taints = state.taints.clone();
                }
            }
        }
    }

    /// Join two control-flow paths.
    ///
    /// A variable is initialized only if it is initialized on both paths. Move
    /// state is the union of both paths, which is conservative: a value moved on
    /// either path is treated as dead afterwards.
    fn merge_states(
        &mut self,
        left: &[HashMap<String, ControlState>],
        right: &[HashMap<String, ControlState>],
    ) {
        for (index, scope) in self.scopes.iter_mut().enumerate() {
            for (name, info) in scope.iter_mut() {
                let Some(left_state) = left.get(index).and_then(|saved| saved.get(name)) else {
                    continue;
                };
                let Some(right_state) = right.get(index).and_then(|saved| saved.get(name)) else {
                    continue;
                };
                info.initialized = left_state.initialized && right_state.initialized;
                let mut moved = left_state.moved.clone();
                moved.extend(right_state.moved.iter().cloned());
                info.moved = moved;
                let mut taints = left_state.taints.clone();
                for taint in &right_state.taints {
                    if !taints.iter().any(|existing| {
                        existing.path == taint.path && existing.region == taint.region
                    }) {
                        taints.push(taint.clone());
                    }
                }
                info.taints = taints;
            }
        }
    }

    // ------------------------------------------------------------------
    // Scope helpers
    // ------------------------------------------------------------------
    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn declare_variable(&mut self, name: &str, info: VarInfo) {
        if name == "_" {
            return;
        }
        let duplicate = self
            .scopes
            .last()
            .map(|scope| scope.contains_key(name))
            .unwrap_or(false);
        if duplicate {
            self.error(info.span, format!("duplicate declaration of '{name}'"));
            return;
        }
        self.scopes
            .last_mut()
            .expect("declaration outside a scope")
            .insert(name.to_string(), info);
    }

    fn type_contains_slice(&self, ty: &Ty) -> bool {
        self.type_contains_slice_inner(ty, &mut HashSet::new())
    }

    fn type_contains_slice_inner(&self, ty: &Ty, visited: &mut HashSet<String>) -> bool {
        match ty {
            Ty::Slice(_) => true,
            Ty::Named(name) => {
                if !visited.insert(name.clone()) {
                    return false;
                }
                match self.interfaces.get(name) {
                    Some(info) => info
                        .fields
                        .iter()
                        .any(|field| self.type_contains_slice_inner(&field.ty, visited)),
                    None => false,
                }
            }
            _ => false,
        }
    }
}

/// Names owned by the compiler. They are the closed intrinsic set from
/// `LOI_SPEC.md` 5.9; user code may not redeclare them.
fn is_intrinsic_name(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "eprint"
            | "panic"
            | "sqrt"
            | "floor"
            | "ceil"
            | "fabs"
            | "pow"
            | "fmod"
            | "len"
            | "read_file"
            | "write_file"
            | "read_stdin"
            | "read_line"
            | "print_int"
            | "print_float"
    )
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::In => "<-",
        Direction::Out => "->",
        Direction::Inout => "<->",
    }
}

/// Scan a function body for allocation and ownership effects.
///
/// `depth` counts enclosing lexical `protect` blocks. Allocations and calls at
/// depth zero are effects that a caller must cover with a `protect`.
fn scan_block_effects(block: &Block, depth: usize, scan: &mut FunctionScan) {
    for statement in &block.statements {
        scan_statement_effects(statement, depth, scan);
    }
}

fn scan_statement_effects(statement: &Stmt, depth: usize, scan: &mut FunctionScan) {
    match &statement.kind {
        StmtKind::Let { value, .. } => {
            if let Some(value) = value {
                scan_expr_effects(value, depth, scan);
            }
        }
        StmtKind::Assign { target, value, .. } => {
            scan_expr_effects(target, depth, scan);
            scan_expr_effects(value, depth, scan);
        }
        StmtKind::Expr(expr) => scan_expr_effects(expr, depth, scan),
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            scan_expr_effects(condition, depth, scan);
            scan_block_effects(then_block, depth, scan);
            match else_branch {
                Some(ElseBranch::Block(block)) => scan_block_effects(block, depth, scan),
                Some(ElseBranch::If(inner)) => scan_statement_effects(inner, depth, scan),
                None => {}
            }
        }
        StmtKind::While { condition, body } => {
            scan_expr_effects(condition, depth, scan);
            scan_block_effects(body, depth, scan);
        }
        StmtKind::ForRange {
            start, end, body, ..
        } => {
            scan_expr_effects(start, depth, scan);
            scan_expr_effects(end, depth, scan);
            scan_block_effects(body, depth, scan);
        }
        StmtKind::ForEach { iterable, body, .. } => {
            scan_expr_effects(iterable, depth, scan);
            scan_block_effects(body, depth, scan);
        }
        StmtKind::Protect(block) => scan_block_effects(block, depth + 1, scan),
        StmtKind::Return(value) => {
            if let Some(value) = value {
                if scan.returns_composite
                    && let Some(root) = place_root(value)
                {
                    scan.moved_roots.insert(root);
                }
                scan_expr_effects(value, depth, scan);
            }
        }
        StmtKind::Continue => {}
        StmtKind::Break => {}
    }
}

fn scan_expr_effects(expr: &Expr, depth: usize, scan: &mut FunctionScan) {
    match &expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => {}
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                scan_expr_effects(element, depth, scan);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                scan_expr_effects(&field.value, depth, scan);
            }
        }
        ExprKind::Unary { operand, .. } => scan_expr_effects(operand, depth, scan),
        ExprKind::Binary { left, right, .. } => {
            scan_expr_effects(left, depth, scan);
            scan_expr_effects(right, depth, scan);
        }
        ExprKind::Cast { expr, .. } => scan_expr_effects(expr, depth, scan),
        ExprKind::Call { callee, args } => {
            if let ExprKind::Ident(name) = &callee.kind {
                scan.calls.insert(name.clone());
                if depth == 0 {
                    scan.unguarded_deps.push(name.clone());
                    if matches!(name.as_str(), "read_file" | "read_stdin" | "read_line") {
                        scan.direct_alloc = true;
                    }
                }
                scan.call_args.push(CallSiteScan {
                    callee: name.clone(),
                    args: args
                        .iter()
                        .map(|arg| (arg.direction, place_root(&arg.value)))
                        .collect(),
                });
            } else {
                scan_expr_effects(callee, depth, scan);
            }
            for arg in args {
                scan_expr_effects(&arg.value, depth, scan);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            if depth == 0 {
                scan.direct_alloc = true;
            }
            scan_expr_effects(target, depth, scan);
            scan_expr_effects(count, depth, scan);
        }
        ExprKind::Move(value) => {
            if let Some(root) = place_root(value) {
                scan.moved_roots.insert(root);
            }
            scan_expr_effects(value, depth, scan);
        }
        ExprKind::Copy(value) => scan_expr_effects(value, depth, scan),
        ExprKind::Member { base, .. } => scan_expr_effects(base, depth, scan),
        ExprKind::Index { base, index } => {
            scan_expr_effects(base, depth, scan);
            scan_expr_effects(index, depth, scan);
        }
    }
}

/// Append one component to a root-relative ownership path.
///
/// The empty path is the whole variable. `field` adds `.field`, and components
/// that already start with `[` (element accesses) are appended verbatim.
fn append_path(base: &str, component: &str) -> String {
    if component.starts_with('[') {
        format!("{base}{component}")
    } else if base.is_empty() {
        component.to_string()
    } else {
        format!("{base}.{component}")
    }
}

/// True when `ancestor` is the same path as `path` or contains it, respecting
/// path-component boundaries (`a` contains `a.b` and `a[]`, but not `ab`).
fn path_is_ancestor(ancestor: &str, path: &str) -> bool {
    if ancestor == path {
        return true;
    }
    if ancestor.is_empty() {
        return true;
    }
    let Some(rest) = path.strip_prefix(ancestor) else {
        return false;
    };
    rest.starts_with('.') || rest.starts_with('[')
}

fn paths_overlap(left: &str, right: &str) -> bool {
    path_is_ancestor(left, right) || path_is_ancestor(right, left)
}

fn path_is_moved(moved: &HashSet<String>, path: &str) -> bool {
    moved.iter().any(|moved| paths_overlap(moved, path))
}

/// Evaluates an integer expression made only of literals, unary negation and
/// the arithmetic operators. Results beyond `i128` are saturated, which keeps
/// them outside the range of every supported integer type. Division or
/// remainder by a constant zero returns `None`; the checker reports that
/// separately at the operator. Any non-constant expression returns `None`.
fn const_int(expr: &Expr) -> Option<i128> {
    match &expr.kind {
        ExprKind::Int(text) => parse_integer_literal(text),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => Some(const_int(operand)?.saturating_neg()),
        ExprKind::Binary { op, left, right } => {
            let left = const_int(left)?;
            let right = const_int(right)?;
            match op {
                BinaryOp::Add => Some(left.checked_add(right).unwrap_or(if right >= 0 {
                    i128::MAX
                } else {
                    i128::MIN
                })),
                BinaryOp::Sub => Some(left.checked_sub(right).unwrap_or(if right < 0 {
                    i128::MAX
                } else {
                    i128::MIN
                })),
                BinaryOp::Mul => Some(left.checked_mul(right).unwrap_or(
                    if (left < 0) ^ (right < 0) {
                        i128::MIN
                    } else {
                        i128::MAX
                    },
                )),
                BinaryOp::Div => {
                    if right == 0 {
                        None
                    } else {
                        Some(
                            left.checked_div(right)
                                .unwrap_or(if (left < 0) ^ (right < 0) {
                                    i128::MIN
                                } else {
                                    i128::MAX
                                }),
                        )
                    }
                }
                BinaryOp::Mod => {
                    if right == 0 {
                        None
                    } else {
                        // The only overflow is `i128::MIN % -1`, whose value is
                        // mathematically zero.
                        Some(left.checked_rem(right).unwrap_or(0))
                    }
                }
                BinaryOp::BitAnd => Some(left & right),
                BinaryOp::BitOr => Some(left | right),
                BinaryOp::BitXor => Some(left ^ right),
                BinaryOp::Shl => {
                    if !(0..128).contains(&right) {
                        None
                    } else {
                        Some(left.wrapping_shl(right as u32))
                    }
                }
                BinaryOp::Shr => {
                    if !(0..128).contains(&right) {
                        None
                    } else {
                        Some(left.wrapping_shr(right as u32))
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse an integer literal text, supporting decimal, hexadecimal (`0x`/`0X`),
/// and binary (`0b`/`0B`) prefixes. Underscores are ignored. Returns `i128::MAX`
/// on parse failure so the value is out of range of every supported type.
fn parse_integer_literal(text: &str) -> Option<i128> {
    let text = text.replace('_', "");
    let (radix, digits) = if let Some(rest) = text.strip_prefix("0x").or(text.strip_prefix("0X")) {
        (16, rest)
    } else if let Some(rest) = text.strip_prefix("0b").or(text.strip_prefix("0B")) {
        (2, rest)
    } else {
        (10, text.as_str())
    };
    i128::from_str_radix(digits, radix).ok().or(Some(i128::MAX))
}

fn is_concrete_integer(ty: &Ty) -> bool {
    ty.is_integer() && !matches!(ty, Ty::IntLiteral | Ty::Error)
}

fn integer_bits(ty: &Ty) -> Option<u128> {
    match ty {
        Ty::I8 | Ty::U8 => Some(8),
        Ty::I16 | Ty::U16 => Some(16),
        Ty::I32 | Ty::U32 | Ty::IntLiteral => Some(32),
        Ty::I64 | Ty::U64 => Some(64),
        _ => None,
    }
}

/// Whether an expression can appear in a static global initializer.
fn is_constant_expr(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => true,
        ExprKind::Unary { operand, .. } => is_constant_expr(operand),
        ExprKind::Binary { left, right, .. } => is_constant_expr(left) && is_constant_expr(right),
        ExprKind::Cast { expr, .. } => is_constant_expr(expr),
        ExprKind::Copy(value) => is_constant_expr(value),
        ExprKind::InterfaceLiteral { fields, .. } => {
            fields.iter().all(|field| is_constant_expr(&field.value))
        }
        ExprKind::SliceLiteral { elements, .. } => elements.iter().all(is_constant_expr),
        _ => false,
    }
}

fn is_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Ident(_) => true,
        ExprKind::Member { base, .. } => is_place(base),
        ExprKind::Index { base, .. } => is_place(base),
        _ => false,
    }
}

fn place_root(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(name) => Some(name.clone()),
        ExprKind::Member { base, .. } => place_root(base),
        ExprKind::Index { base, .. } => place_root(base),
        _ => None,
    }
}

fn place_path(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(_) => Some(String::new()),
        ExprKind::Member { base, field } => place_path(base).map(|path| append_path(&path, field)),
        ExprKind::Index { base, index } => {
            let component = match const_int(index) {
                Some(value) => format!("[{value}]"),
                None => "[]".to_string(),
            };
            place_path(base).map(|path| append_path(&path, &component))
        }
        _ => None,
    }
}

fn is_composite(ty: &Ty) -> bool {
    matches!(ty, Ty::Named(_) | Ty::Slice(_))
}

/// Collect the places read by an expression as `(root variable, relative path)`.
///
/// Function names in callee position are not variables, so only call arguments
/// contribute. Element indexes contribute their own reads. Used to enforce
/// exclusive `out`/`inout` arguments without rejecting disjoint fields of the
/// same variable.
fn collect_accesses(expr: &Expr, accesses: &mut Vec<(String, String)>) {
    match &expr.kind {
        ExprKind::Ident(name) => accesses.push((name.clone(), String::new())),
        ExprKind::Unary { operand, .. } => collect_accesses(operand, accesses),
        ExprKind::Binary { left, right, .. } => {
            collect_accesses(left, accesses);
            collect_accesses(right, accesses);
        }
        ExprKind::Cast { expr, .. } => collect_accesses(expr, accesses),
        ExprKind::Call { args, .. } => {
            for arg in args {
                collect_accesses(&arg.value, accesses);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            collect_accesses(target, accesses);
            collect_accesses(count, accesses);
        }
        ExprKind::Move(value) | ExprKind::Copy(value) => collect_accesses(value, accesses),
        ExprKind::Member { .. } | ExprKind::Index { .. } => {
            if let (Some(root), Some(path)) = (place_root(expr), place_path(expr)) {
                accesses.push((root, path));
                collect_index_reads(expr, accesses);
            } else if let ExprKind::Index { base, index } = &expr.kind {
                collect_accesses(base, accesses);
                collect_accesses(index, accesses);
            } else if let ExprKind::Member { base, .. } = &expr.kind {
                collect_accesses(base, accesses);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                collect_accesses(&field.value, accesses);
            }
        }
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                collect_accesses(element, accesses);
            }
        }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {}
    }
}

fn collect_index_reads(expr: &Expr, accesses: &mut Vec<(String, String)>) {
    match &expr.kind {
        ExprKind::Member { base, .. } => collect_index_reads(base, accesses),
        ExprKind::Index { base, index } => {
            collect_index_reads(base, accesses);
            collect_accesses(index, accesses);
        }
        _ => {}
    }
}
