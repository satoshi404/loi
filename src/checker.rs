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
use crate::diagnostic::Diagnostic;
use crate::types::{self, Ty};

/// Check a complete program, returning every diagnostic found.
pub fn check(program: &Program) -> Vec<Diagnostic> {
    let (program, mono_errors, source_names) = crate::monomorphize::monomorphize_for_check(program);
    let has_mono_errors = !mono_errors.is_empty();
    let mut diagnostics = mono_errors
        .into_iter()
        .map(|error| Diagnostic::from_error(error.message, error.span))
        .collect::<Vec<_>>();
    if has_mono_errors {
        // Generic errors (missing arguments, recursion limits, name
        // collisions) leave the program partly rewritten. Checking it would
        // only add cascades about unknown generated names.
        return diagnostics;
    }
    for error in crate::const_eval::validate_comptime_functions(&program) {
        diagnostics.push(Diagnostic::from_error(error.message, error.span));
    }
    for error in crate::const_eval::check_comptime_expressions(&program) {
        diagnostics.push(Diagnostic::from_error(error.message, error.span));
    }
    let mut checker = Checker::new();
    checker.collect_names(&program);
    checker.resolve_interfaces(&program);
    checker.resolve_marks_and_choices(&program);
    checker.check_interface_value_cycles(&program);
    checker.resolve_functions(&program);
    checker.analyze_function_effects(&program);
    checker.check_interface_defaults(&program);
    checker.declare_globals(&program);
    checker.check_functions(&program);
    diagnostics.extend(checker.diagnostics);
    show_source_names(&mut diagnostics, &source_names);
    diagnostics
}

/// Replace monomorphized identifiers in diagnostics with the spelling the user
/// wrote. Generated C symbols stay mangled; only messages are rewritten.
fn show_source_names(
    diagnostics: &mut [Diagnostic],
    source_names: &std::collections::HashMap<String, String>,
) {
    if source_names.is_empty() {
        return;
    }
    // Longest first so `Vec__i32__i64` is not split into `Vec<i32>__i64`.
    let mut names = source_names.iter().collect::<Vec<_>>();
    names.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
    for diagnostic in diagnostics {
        let rewrite = |text: &str| {
            let mut text = text.to_string();
            for (mangled, source) in &names {
                text = replace_identifier(&text, mangled, source);
            }
            text
        };
        diagnostic.message = rewrite(&diagnostic.message);
        if let Some(label) = &diagnostic.label {
            diagnostic.label = Some(rewrite(label));
        }
        for secondary in &mut diagnostic.secondary {
            secondary.label = rewrite(&secondary.label);
        }
        if let Some(help) = &diagnostic.help {
            diagnostic.help = Some(rewrite(help));
        }
        if let Some(note) = &diagnostic.note {
            diagnostic.note = Some(rewrite(note));
        }
    }
}

fn replace_identifier(text: &str, needle: &str, replacement: &str) -> String {
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if text[index..].starts_with(needle) {
            let before = index == 0 || !is_identifier_byte(bytes[index - 1]);
            let end = index + needle.len();
            let after = end == text.len() || !is_identifier_byte(bytes[end]);
            if before && after {
                result.push_str(replacement);
                index = end;
                continue;
            }
        }
        let character = text[index..].chars().next().expect("valid char boundary");
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
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

/// A `mark` declaration after variant resolution.
#[derive(Debug, Clone)]
struct MarkInfo {
    variants: HashMap<String, Span>,
}

/// A `choice` declaration after variant resolution.
#[derive(Debug, Clone)]
struct ChoiceInfo {
    variants: HashMap<String, ChoiceVariantInfo>,
}

#[derive(Debug, Clone)]
struct ChoiceVariantInfo {
    ty: Ty,
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
    /// Paths moved out of this variable, with the move site. `""` means the
    /// whole variable.
    moved: HashMap<String, Span>,
    /// Lexical block or `protect` region where this variable's storage lives;
    /// `None` means a parameter or the caller.
    home_region: Option<usize>,
    /// `protect` allocations or block-local slice literals reachable through
    /// this variable.
    taints: Vec<RegionTaint>,
    /// Top-level bindings are stored in the base scope. Their move state is
    /// never recorded: ownership is tracked per function body.
    is_global: bool,
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
    /// An `alloc` or slice-cloning `clone` outside every lexical `protect`.
    direct_alloc: bool,
    /// A slice-cloning `clone` anywhere in this function.
    clone_direct: bool,
    /// Calls made outside every lexical `protect`.
    unguarded_deps: Vec<String>,
    /// Every called function name.
    calls: HashSet<String>,
    /// Root-relative places consumed by `give(...)` in this body.
    moved_places: HashSet<(String, String)>,
    /// Call sites with the direction and place of every argument.
    call_args: Vec<CallSiteScan>,
    /// Whether returning a place transfers ownership (composite return type).
    returns_composite: bool,
    /// `<-` parameter name to its parameter index. Used to record which parts
    /// of a borrowed parameter the body actually reads.
    read_params: HashMap<String, usize>,
    /// Per `<-` parameter index, the root-relative places the body reads
    /// directly (excluding whole-value `<-` arguments refined by the fixpoint).
    direct_reads: Vec<HashSet<String>>,
}

impl FunctionScan {
    fn record_reads(&mut self, expr: &Expr) {
        collect_direct_reads(expr, &self.read_params, &mut self.direct_reads);
    }
}

struct CallSiteScan {
    callee: String,
    args: Vec<(Direction, Option<(String, String)>)>,
}

struct LvalueInfo {
    ty: Ty,
    root: Option<String>,
    /// Path relative to the root variable. `Some("")` is the whole variable.
    path: Option<String>,
    /// The place contains an unknown name, so later errors about this place
    /// would only cascade from the first diagnostic.
    poisoned: bool,
}

#[derive(Clone, Copy)]
enum NameSpace {
    Variable,
    Function,
    Type,
}

#[derive(Debug, Clone)]
struct ControlState {
    initialized: bool,
    moved: HashMap<String, Span>,
    taints: Vec<RegionTaint>,
}

struct Checker {
    interfaces: HashMap<String, InterfaceInfo>,
    /// Payload-free tag sets declared with `mark`.
    marks: HashMap<String, MarkInfo>,
    /// Tagged unions declared with `choice`.
    choices: HashMap<String, ChoiceInfo>,
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
    /// Function names whose result carries storage allocated by a recursive
    /// `clone` of a slice-containing type. Used to taint call results with the
    /// caller's active `protect` region.
    clone_allocators: HashSet<String>,
    /// Per function, the root-relative places each parameter may lose through
    /// the call. `""` means the whole parameter.
    param_consumption: HashMap<String, Vec<HashSet<String>>>,
    /// Per function, the root-relative places each `<-` parameter may be read
    /// through. `""` means the whole parameter. A partially consumed composite
    /// may be passed to a `<-` flow whose reads avoid the consumed paths.
    param_reads: HashMap<String, Vec<HashSet<String>>>,
    /// Names declared by top-level `let`/`var`, for duplicate diagnostics.
    global_names: HashSet<String>,
    /// Parent region for each lexical block or `protect` region.
    region_parents: Vec<Option<usize>>,
    /// Kind of each region, indexed by region id.
    region_kinds: Vec<RegionKind>,
    /// Where each region started, for escape diagnostics.
    region_origins: Vec<Option<Span>>,
    /// Regions currently active, innermost last.
    region_stack: Vec<usize>,
}

impl Checker {
    fn new() -> Self {
        Self {
            interfaces: HashMap::new(),
            marks: HashMap::new(),
            choices: HashMap::new(),
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
            clone_allocators: HashSet::new(),
            param_consumption: HashMap::new(),
            param_reads: HashMap::new(),
            global_names: HashSet::new(),
            region_parents: Vec::new(),
            region_kinds: Vec::new(),
            region_origins: Vec::new(),
            region_stack: Vec::new(),
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::from_error(message, span));
    }

    /// Report an unknown name and suggest the closest declaration when one is
    /// close enough. Message text is unchanged; the suggestion is a help line.
    fn error_unknown(
        &mut self,
        span: Span,
        message: impl Into<String>,
        name: &str,
        namespace: NameSpace,
    ) {
        let mut diagnostic = Diagnostic::from_error(message, span);
        if let Some(suggestion) = self.name_suggestion(name, namespace) {
            diagnostic = diagnostic.with_help(format!("did you mean '{suggestion}'?"));
        }
        self.diagnostics.push(diagnostic);
    }

    /// Suggest only names that could have been meant in this position:
    /// variables for variables, flows for calls and shapes for types. Scopes
    /// are visited innermost first, so a nearby local wins over a global or a
    /// shape with the same spelling.
    fn name_suggestion(&self, name: &str, namespace: NameSpace) -> Option<String> {
        let mut seen = HashSet::new();
        let candidates: Vec<&str> = match namespace {
            NameSpace::Variable => self
                .scopes
                .iter()
                .rev()
                .flat_map(|scope| scope.keys())
                .filter(|candidate| seen.insert(candidate.as_str()))
                .map(String::as_str)
                .collect(),
            NameSpace::Function => self.functions.keys().map(String::as_str).collect(),
            NameSpace::Type => self
                .interfaces
                .keys()
                .chain(self.marks.keys())
                .chain(self.choices.keys())
                .chain(self.opaques.iter())
                .map(String::as_str)
                .collect(),
        };
        crate::diagnostic::suggest_name(name, candidates).map(str::to_string)
    }

    /// Report a use after move and point at the move that killed the value.
    fn error_moved(&mut self, span: Span, message: impl Into<String>, root: &str, path: &str) {
        let moved_here = self.moved_span(root, path);
        let mut diagnostic = Diagnostic::from_error(message, span).with_label("used after move");
        if let Some(moved_here) = moved_here {
            diagnostic = diagnostic.with_secondary(moved_here, "moved here");
        }
        self.diagnostics.push(diagnostic);
    }

    fn moved_span(&self, root: &str, path: &str) -> Option<Span> {
        self.lookup(root).and_then(|var| {
            var.moved
                .iter()
                .find(|(moved, _)| paths_overlap(moved, path))
                .map(|(_, span)| *span)
        })
    }

    /// Report a write to read-only storage and point at the declaration.
    fn error_readonly(&mut self, span: Span, message: impl Into<String>, name: &str, label: &str) {
        let declared = self.lookup(name).map(|var| var.span);
        let mut diagnostic = Diagnostic::from_error(message, span).with_label("cannot write here");
        if let Some(declared) = declared {
            diagnostic = diagnostic.with_secondary(declared, label);
        }
        self.diagnostics.push(diagnostic);
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
                        || self.marks.contains_key(&decl.name)
                        || self.choices.contains_key(&decl.name)
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
                    if self.marks.contains_key(&decl.name) || self.choices.contains_key(&decl.name)
                    {
                        self.error(
                            decl.span,
                            format!(
                                "global '{}' conflicts with a type of the same name",
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
                        || self.marks.contains_key(&decl.name)
                        || self.choices.contains_key(&decl.name)
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
                Item::Mark(decl) => {
                    self.register_sum_type_name(&decl.name, decl.span, "mark");
                }
                Item::Choice(decl) => {
                    self.register_sum_type_name(&decl.name, decl.span, "choice");
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
                    } else if self.marks.contains_key(&decl.name)
                        || self.choices.contains_key(&decl.name)
                    {
                        self.error(
                            decl.span,
                            format!(
                                "function '{}' conflicts with a type of the same name",
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

    /// Register a `mark` or `choice` name during `collect_names`. Variant
    /// bodies are resolved later, after `shape` fields are known.
    fn register_sum_type_name(&mut self, name: &str, span: Span, keyword: &str) {
        if is_intrinsic_name(name) {
            self.error(span, format!("intrinsic '{name}' cannot be redeclared"));
        } else if self.interfaces.contains_key(name)
            || self.marks.contains_key(name)
            || self.choices.contains_key(name)
            || self.opaques.contains(name)
            || self.functions.contains_key(name)
            || self.global_names.contains(name)
        {
            self.error(span, format!("duplicate type '{name}'"));
        } else if keyword == "mark" {
            self.marks.insert(
                name.to_string(),
                MarkInfo {
                    variants: HashMap::new(),
                },
            );
        } else {
            self.choices.insert(
                name.to_string(),
                ChoiceInfo {
                    variants: HashMap::new(),
                },
            );
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
                span: Span::new(0, 0),
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
                let ty = self.resolve_type(&field.ty);
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

    /// Resolve the variants of every `mark` and `choice` declaration. Names are
    /// registered in `collect_names` so shape fields may already refer to them;
    /// this pass fills in the variant tables and validates choice payloads.
    fn resolve_marks_and_choices(&mut self, program: &Program) {
        for item in &program.items {
            let Item::Mark(decl) = item else {
                continue;
            };
            let Some(mut info) = self.marks.get(&decl.name).cloned() else {
                continue;
            };
            for variant in &decl.variants {
                if info.variants.contains_key(&variant.name) {
                    self.error(
                        variant.span,
                        format!(
                            "duplicate variant '{}' in mark '{}'",
                            variant.name, decl.name
                        ),
                    );
                } else {
                    info.variants.insert(variant.name.clone(), variant.span);
                }
            }
            self.marks.insert(decl.name.clone(), info);
        }

        for item in &program.items {
            let Item::Choice(decl) = item else {
                continue;
            };
            let Some(mut info) = self.choices.get(&decl.name).cloned() else {
                continue;
            };
            for variant in &decl.variants {
                let ty = self.resolve_type(&variant.ty);
                if !ty.is_error() && !self.valid_choice_payload(&ty) {
                    self.error(
                        variant.ty.span(),
                        format!(
                            "choice '{}' variant '{}' cannot hold {ty}; payloads must be scalars, marks or slice-free shapes",
                            decl.name, variant.name
                        ),
                    );
                }
                if info.variants.contains_key(&variant.name) {
                    self.error(
                        variant.span,
                        format!(
                            "duplicate variant '{}' in choice '{}'",
                            variant.name, decl.name
                        ),
                    );
                } else {
                    info.variants
                        .insert(variant.name.clone(), ChoiceVariantInfo { ty });
                }
            }
            self.choices.insert(decl.name.clone(), info);
        }
    }

    /// Whether `ty` may be stored in a choice payload. Slices and nested
    /// choices would add ownership and layout complexity to the C union, so
    /// they are rejected until a dedicated design exists.
    fn valid_choice_payload(&self, ty: &Ty) -> bool {
        match ty {
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
            | Ty::Mark(_) => true,
            Ty::Named(name) => self.interfaces.contains_key(name) && !self.type_contains_slice(ty),
            _ => false,
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
                if let Type::Named(name, _) = &field.ty
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
                let ty = self.resolve_type(&param.ty);
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
                Some(ty) => self.resolve_type(ty),
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
        let mut evaluator = crate::const_eval::ConstEvaluator::new(program);
        for item in &program.items {
            let Item::Global(decl) = item else {
                continue;
            };

            let declared = decl.ty.as_ref().map(|ty| self.resolve_type(ty));
            let declared_span = decl
                .ty
                .as_ref()
                .map(|ty| ty.span())
                .unwrap_or(decl.value.span);
            let value_ty = self.check_expr(&decl.value);

            if let Err(error) = evaluator.eval_global(decl) {
                let message = if error.message.contains("overflow") {
                    format!(
                        "initializer for global '{}' is invalid: {}",
                        decl.name, error.message
                    )
                } else {
                    format!(
                        "initializer for global '{}' must be a compile-time constant: {}",
                        decl.name, error.message
                    )
                };
                if error.message.contains("comptime call depth exceeded") {
                    // A depth-limit failure describes the whole initializer.
                    // The recursive call is already the primary location of
                    // the first diagnostic, so keep it secondary here and
                    // show where the affected global is declared.
                    self.diagnostics.push(
                        Diagnostic::from_error(message, decl.span)
                            .with_label("non-constant global initializer")
                            .with_secondary(error.span, "recursive comptime call"),
                    );
                } else {
                    self.error(error.span, message);
                }
            }

            let resolved = match (declared, value_ty) {
                (Some(declared), value_ty) => {
                    if !self.assignable_expr(&declared, &value_ty, &decl.value) {
                        self.error(
                            declared_span,
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

            self.scopes[0].insert(
                decl.name.clone(),
                VarInfo {
                    ty: resolved,
                    mutable: decl.mutable,
                    initialized: true,
                    direction: None,
                    span: decl.span,
                    moved: HashMap::new(),
                    home_region: None,
                    taints: Vec::new(),
                    is_global: true,
                },
            );
        }
    }

    /// Resolve a syntactic type to a semantic type, reporting type diagnostics
    /// on the span of the type itself rather than the enclosing declaration.
    fn resolve_type(&mut self, ty: &Type) -> Ty {
        let span = ty.span();
        match ty {
            Type::Named(name, _) => {
                if let Some(primitive) = Ty::from_name(name) {
                    primitive
                } else if name == "cstr" {
                    Ty::CStr
                } else if self.interfaces.contains_key(name) {
                    Ty::Named(name.clone())
                } else if self.marks.contains_key(name) {
                    Ty::Mark(name.clone())
                } else if self.choices.contains_key(name) {
                    Ty::Choice(name.clone())
                } else if self.opaques.contains(name) {
                    Ty::Opaque(name.clone())
                } else {
                    self.error_unknown(
                        span,
                        format!("unknown type '{name}'"),
                        name,
                        NameSpace::Type,
                    );
                    Ty::Error
                }
            }
            Type::Slice(element, _) => Ty::Slice(Box::new(self.resolve_type(element))),
            Type::Generic(name, _, _) => {
                self.error(
                    span,
                    format!("generic type '{name}' must be instantiated before checking"),
                );
                Ty::Error
            }
        }
    }

    /// Side-effect-free type resolution for the effect prepass and region
    /// tracking. Unknown names become `Ty::Error`; diagnostics are produced by
    /// the real body check, so this helper must never emit any.
    fn resolve_type_pure(&self, ty: &Type) -> Ty {
        match ty {
            Type::Named(name, _) => {
                if let Some(primitive) = Ty::from_name(name) {
                    primitive
                } else if name == "cstr" {
                    Ty::CStr
                } else if self.interfaces.contains_key(name) {
                    Ty::Named(name.clone())
                } else if self.marks.contains_key(name) {
                    Ty::Mark(name.clone())
                } else if self.choices.contains_key(name) {
                    Ty::Choice(name.clone())
                } else if self.opaques.contains(name) {
                    Ty::Opaque(name.clone())
                } else {
                    Ty::Error
                }
            }
            Type::Slice(element, _) => Ty::Slice(Box::new(self.resolve_type_pure(element))),
            Type::Generic(name, _, _) => {
                let _ = name;
                Ty::Error
            }
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
    /// as long as every caller provides a region. `main` is the only entry: an
    /// uncalled helper may rely on its caller's region regardless of
    /// visibility. This lets exported library allocators be `open` while Loi
    /// callers are still checked: an unguarded call propagates the effect to
    /// `main`, which is rejected. A foreign caller of an exported allocator
    /// must provide the `protect` region by contract.
    ///
    /// Ownership: a parameter is *consumed* when the function moves from it,
    /// directly or through another call. Consumption is path-sensitive: moving
    /// a field does not kill the rest of the value. At a call site only the
    /// consumed paths of an `out` or `inout` argument are moved for the caller.
    fn analyze_function_effects(&mut self, program: &Program) {
        let mut scans: HashMap<String, FunctionScan> = HashMap::new();
        let mut params: HashMap<String, Vec<(String, Direction, Ty)>> = HashMap::new();
        let mut spans: Vec<(String, Span)> = Vec::new();

        for item in &program.items {
            let Item::Function(decl) = item else {
                continue;
            };
            if !self.is_canonical_function(&decl.name, decl.span) {
                continue;
            }

            let mut scan = FunctionScan::default();
            let mut locals = HashMap::new();
            if let Some(signature) = self.functions.get(&decl.name) {
                scan.returns_composite = is_composite(&signature.return_type);
                for (index, param) in signature.params.iter().enumerate() {
                    locals.insert(param.name.clone(), param.ty.clone());
                    if param.direction == Direction::In {
                        scan.read_params.insert(param.name.clone(), index);
                    }
                }
                scan.direct_reads = vec![HashSet::new(); signature.params.len()];
            }
            for item in &program.items {
                if let Item::Global(global) = item {
                    let ty = global
                        .ty
                        .as_ref()
                        .map(|ty| self.resolve_type_pure(ty))
                        .unwrap_or_else(|| self.scan_expr_ty(&global.value, &HashMap::new()));
                    locals.insert(global.name.clone(), ty);
                }
            }
            scan_block_effects(self, &decl.body, 0, &mut scan, &mut locals);
            scans.insert(decl.name.clone(), scan);
            let param_types = self
                .functions
                .get(&decl.name)
                .map(|signature| signature.params.clone())
                .unwrap_or_default();
            params.insert(
                decl.name.clone(),
                decl.params
                    .iter()
                    .enumerate()
                    .map(|(index, param)| {
                        let ty = param_types
                            .get(index)
                            .map(|sig| sig.ty.clone())
                            .unwrap_or(Ty::Error);
                        (param.name.clone(), param.direction, ty)
                    })
                    .collect(),
            );
            spans.push((decl.name.clone(), decl.span));
        }

        let mut direct = HashSet::new();
        let mut clone_direct = HashSet::new();
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        let mut called = HashSet::new();
        for (name, scan) in &scans {
            if scan.direct_alloc {
                direct.insert(name.clone());
            }
            if scan.clone_direct {
                clone_direct.insert(name.clone());
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

        for (name, span) in &spans {
            // `main` is the only entry point for this check. An uncalled helper
            // can only run when another Loi function calls it, so its caller's
            // `protect` is available even when the helper is exported.
            let entry = name == "main";
            if entry && unguarded.contains(name) {
                self.error(
                    *span,
                    format!("function '{name}' can allocate outside a 'protect' block"),
                );
            }
        }
        self.unguarded_allocators = unguarded;

        // A flow whose result may carry clone-allocated storage is itself a
        // clone allocator when it calls such a flow. This mirrors the
        // unguarded-allocation propagation but uses all calls, not just calls
        // outside `protect`: a clone inside a nested `protect` still allocates.
        let mut clone_allocators = clone_direct;
        loop {
            let mut changed = false;
            for (name, scan) in &scans {
                if clone_allocators.contains(name) {
                    continue;
                }
                if scan
                    .calls
                    .iter()
                    .any(|callee| clone_allocators.contains(callee))
                {
                    clone_allocators.insert(name.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.clone_allocators = clone_allocators;

        let mut consumption: HashMap<String, Vec<HashSet<String>>> = HashMap::new();
        for (name, param_list) in &params {
            let scan = &scans[name];
            let mut paths: Vec<HashSet<String>> =
                param_list.iter().map(|_| HashSet::new()).collect();
            for (root, path) in &scan.moved_places {
                let Some((index, (_, direction, ty))) = param_list
                    .iter()
                    .enumerate()
                    .find(|(_, (param, _, _))| param == root)
                else {
                    continue;
                };
                if *direction == Direction::In || !self.move_consumes_place(ty, path) {
                    continue;
                }
                insert_consumed_path(&mut paths[index], path.clone());
            }
            consumption.insert(name.clone(), paths);
        }

        loop {
            let mut changed = false;
            for (name, scan) in &scans {
                let Some(param_list) = params.get(name) else {
                    continue;
                };
                for site in &scan.call_args {
                    let Some(callee_paths) = consumption.get(&site.callee).cloned() else {
                        continue;
                    };
                    let Some(callee_signature) = self.functions.get(&site.callee) else {
                        continue;
                    };
                    for (index, arg) in site.args.iter().enumerate() {
                        let Some(consumed_paths) = callee_paths.get(index) else {
                            continue;
                        };
                        if consumed_paths.is_empty() {
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
                        let (_, Some((arg_root, arg_path))) = &arg else {
                            continue;
                        };
                        let Some((position, (_, caller_direction, _))) = param_list
                            .iter()
                            .enumerate()
                            .find(|(_, (param, _, _))| param == arg_root)
                        else {
                            continue;
                        };
                        if *caller_direction == Direction::In {
                            continue;
                        }
                        let caller_paths = consumption.get_mut(name).expect("consumption paths");
                        for relative in consumed_paths {
                            let combined = join_paths(arg_path, relative);
                            changed |= insert_consumed_path(&mut caller_paths[position], combined);
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        self.param_consumption = consumption;

        // Read views: summarize which paths of each `<-` parameter a flow may
        // read. Direct reads come from the body scan; whole-value `<-`
        // arguments are refined here from the callee's summary, so a flow that
        // forwards `p` to a field-limited reader keeps that field precision.
        let mut reads: HashMap<String, Vec<HashSet<String>>> = HashMap::new();
        for (name, param_list) in &params {
            let scan = &scans[name];
            let mut paths: Vec<HashSet<String>> =
                param_list.iter().map(|_| HashSet::new()).collect();
            for (index, direct) in scan.direct_reads.iter().enumerate() {
                paths[index].extend(direct.iter().cloned());
            }
            reads.insert(name.clone(), paths);
        }

        loop {
            let mut changed = false;
            for (name, scan) in &scans {
                let Some(param_list) = params.get(name) else {
                    continue;
                };
                for site in &scan.call_args {
                    for (index, arg) in site.args.iter().enumerate() {
                        if arg.0 != Direction::In {
                            continue;
                        }
                        let Some((arg_root, arg_path)) = &arg.1 else {
                            continue;
                        };
                        let Some((caller_index, (_, caller_direction, _))) = param_list
                            .iter()
                            .enumerate()
                            .find(|(_, (param, _, _))| param == arg_root)
                        else {
                            continue;
                        };
                        if *caller_direction != Direction::In {
                            continue;
                        }
                        // An unknown callee (intrinsic or external flow) may
                        // read the whole borrowed value: stay conservative.
                        let effective = reads
                            .get(&site.callee)
                            .and_then(|callee| callee.get(index).cloned())
                            .unwrap_or_else(|| HashSet::from([String::new()]));
                        let caller_paths =
                            &mut reads.get_mut(name).expect("read paths")[caller_index];
                        for relative in &effective {
                            let combined = join_paths(arg_path, relative);
                            changed |= insert_read_path(caller_paths, combined);
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        self.param_reads = reads;
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
        let body_region = self.begin_region(RegionKind::Block, Some(decl.span));

        for (param, signature) in decl.params.iter().zip(signature.params.iter()) {
            let info = VarInfo {
                ty: signature.ty.clone(),
                mutable: signature.direction != Direction::In,
                initialized: signature.direction != Direction::Out,
                direction: Some(signature.direction),
                span: param.span,
                moved: HashMap::new(),
                home_region: None,
                taints: Vec::new(),
                is_global: false,
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
        let region = self.begin_region(RegionKind::Block, None);
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
                let declared = ty.as_ref().map(|ty| self.resolve_type(ty));
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
                                ty.as_ref().expect("declared type").span(),
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
                        moved: HashMap::new(),
                        home_region: self.current_region(),
                        taints: Vec::new(),
                        is_global: false,
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
                let body_region = self.begin_region(RegionKind::Block, Some(statement.span));
                if variable != "_" {
                    self.declare_variable(
                        variable,
                        VarInfo {
                            ty: variable_ty,
                            mutable: false,
                            initialized: true,
                            direction: None,
                            span: statement.span,
                            moved: HashMap::new(),
                            home_region: self.current_region(),
                            taints: Vec::new(),
                            is_global: false,
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
                let body_region = self.begin_region(RegionKind::Block, Some(statement.span));
                if variable != "_" {
                    self.declare_variable(
                        variable,
                        VarInfo {
                            ty: element_ty,
                            mutable: *direction == Direction::Inout,
                            initialized: true,
                            direction: Some(*direction),
                            span: statement.span,
                            moved: HashMap::new(),
                            home_region: self.current_region(),
                            taints: Vec::new(),
                            is_global: false,
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
                let region = self.enter_protect(statement.span);
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
                        self.record_move(&root, &path, value.span);
                    }
                }
            }
        }
    }

    fn check_assignment(&mut self, target: &Expr, op: AssignOp, value: &Expr) {
        if self.is_choice_payload_place(target) {
            self.error(
                target.span,
                "cannot assign to a choice payload; construct a new value with Name.Variant(value)",
            );
            self.check_expr(value);
            return;
        }
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
                        self.error_moved(
                            expr.span,
                            format!("use of moved value '{name}'"),
                            name,
                            "",
                        );
                    }
                    ty
                }
                None => {
                    self.error_unknown(
                        expr.span,
                        format!("unknown variable '{name}'"),
                        name,
                        NameSpace::Variable,
                    );
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
                if *op == BinaryOp::Is {
                    return self.check_is_expr(left, right);
                }
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
            ExprKind::InterfaceLiteral { name, fields, .. } => {
                self.check_interface_literal(name, fields, expr.span)
            }
            ExprKind::Cast { expr: inner, ty } => {
                let target = self.resolve_type(ty);
                let source = self.check_expr(inner);
                if !source.is_error()
                    && !target.is_error()
                    && !(source.is_numeric() && target.is_numeric())
                {
                    self.error(ty.span(), format!("cannot cast {source} to {target}"));
                }
                target
            }
            ExprKind::Call { callee, args, .. } => self.check_call(callee, args, expr.span),
            ExprKind::Alloc {
                target,
                element_type,
                count,
            } => self.check_alloc(target, element_type, count, expr.span),
            ExprKind::Move(value) => self.check_move(value),
            ExprKind::Copy(value) => self.check_copy(value),
            ExprKind::Comptime(inner) => self.check_expr(inner),
            ExprKind::Member { base, field } => {
                // `Name.Variant` is a variant reference, not a field access.
                // Marks use it as a value; choice variants are only valid as
                // constructors (`Name.Variant(value)`) or `is` operands.
                if let ExprKind::Ident(name) = &base.kind {
                    if let Some(info) = self.marks.get(name) {
                        if info.variants.contains_key(field) {
                            return Ty::Mark(name.clone());
                        }
                        self.error(expr.span, format!("mark '{name}' has no variant '{field}'"));
                        return Ty::Error;
                    }
                    if self.choices.contains_key(name) {
                        self.error(
                            expr.span,
                            format!(
                                "choice variant '{name}.{field}' must be constructed with a value: {name}.{field}(value)"
                            ),
                        );
                        return Ty::Error;
                    }
                }
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
            let element_ty = self.resolve_type(element_type);
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
            self.error_moved(span, format!("use of moved value '{root}'"), &root, &path);
        }
    }

    fn check_interface_literal(&mut self, name: &str, fields: &[FieldInit], span: Span) -> Ty {
        if !self.interfaces.contains_key(name) {
            self.error_unknown(
                span,
                format!("unknown shape '{name}'"),
                name,
                NameSpace::Type,
            );
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

    /// Check `value is Name.Variant`. The right operand is a variant reference,
    /// not an expression that can be evaluated on its own, so it is inspected
    /// directly instead of going through `check_expr`.
    fn check_is_expr(&mut self, left: &Expr, right: &Expr) -> Ty {
        let left_ty = self.check_expr(left);

        let ExprKind::Member { base, field } = &right.kind else {
            self.error(
                right.span,
                "'is' expects a choice variant on the right, such as 'Result.Ok'",
            );
            self.check_expr(right);
            return Ty::Bool;
        };
        let ExprKind::Ident(choice_name) = &base.kind else {
            self.error(
                right.span,
                "'is' expects a choice variant on the right, such as 'Result.Ok'",
            );
            self.check_expr(right);
            return Ty::Bool;
        };

        let Ty::Choice(actual) = &left_ty else {
            if !left_ty.is_error() {
                self.error(
                    left.span,
                    format!("'is' requires a choice value on the left, found {left_ty}"),
                );
            }
            return Ty::Bool;
        };
        if actual != choice_name {
            self.error(
                right.span,
                format!("variant '{choice_name}.{field}' does not belong to choice '{actual}'"),
            );
            return Ty::Bool;
        }
        let Some(info) = self.choices.get(actual) else {
            return Ty::Bool;
        };
        if !info.variants.contains_key(field) {
            self.error(
                right.span,
                format!("choice '{actual}' has no variant '{field}'"),
            );
        }
        Ty::Bool
    }

    fn check_binary(&mut self, op: BinaryOp, left: &Ty, right: &Ty, span: Span) -> Ty {
        match op {
            // `is` is checked by `check_is_expr`, which needs the right-hand
            // expression to find the choice variant. This arm is unreachable
            // for well-formed ASTs.
            BinaryOp::Is => {
                self.error(span, "'is' expects a choice variant on the right");
                Ty::Bool
            }
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
                let same_mark = matches!((left, right), (Ty::Mark(a), Ty::Mark(b)) if a == b);
                let comparable = (scalar(left)
                    && scalar(right)
                    && (left == right || types::common_numeric(left, right).is_some()))
                    || (byte_slice(left) && byte_slice(right))
                    || same_mark;
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
        // `Name.Variant(value)` constructs a choice value. It is not a call to
        // a named flow, so handle it before the ordinary call rules.
        if let ExprKind::Member { base, field } = &callee.kind
            && let ExprKind::Ident(choice_name) = &base.kind
            && self.choices.contains_key(choice_name)
        {
            return self.check_choice_constructor(choice_name, field, args, span);
        }

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

        // `printf` is variadic: the first argument is a literal format string
        // whose conversions decide how many values follow and of which kind.
        if name == "printf" {
            return self.check_printf_call(args, span);
        }

        let Some(signature) = self.functions.get(name).cloned() else {
            self.error_unknown(
                callee.span,
                format!("unknown function '{name}'"),
                name,
                NameSpace::Function,
            );
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
                    if let Some(info) = self.check_call_arg(arg, param, may_allocate, name, index) {
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

    /// Check `Name.Variant(value)`, the constructor of a `choice`. Exactly one
    /// `<-` argument is required and its type must match the variant payload.
    fn check_choice_constructor(
        &mut self,
        choice_name: &str,
        variant_name: &str,
        args: &[CallArg],
        span: Span,
    ) -> Ty {
        let Some(info) = self.choices.get(choice_name).cloned() else {
            return Ty::Error;
        };
        let Some(variant) = info.variants.get(variant_name) else {
            self.error(
                span,
                format!("choice '{choice_name}' has no variant '{variant_name}'"),
            );
            for arg in args {
                self.check_unmatched_arg(arg);
            }
            return Ty::Error;
        };

        if args.len() != 1 {
            self.error(
                span,
                format!(
                    "choice variant '{choice_name}.{variant_name}' expects 1 argument, found {}",
                    args.len()
                ),
            );
            for arg in args {
                self.check_unmatched_arg(arg);
            }
            return Ty::Choice(choice_name.to_string());
        }

        let arg = &args[0];
        if arg.direction != Direction::In {
            self.error(
                arg.value.span,
                format!("choice variant '{choice_name}.{variant_name}' expects a '<-' argument"),
            );
        }
        let actual = self.check_expr(&arg.value);
        if !self.assignable_expr(&variant.ty, &actual, &arg.value) {
            self.error(
                arg.value.span,
                format!(
                    "argument of '{choice_name}.{variant_name}' has type {actual}, expected {}",
                    variant.ty
                ),
            );
        }
        self.check_literal_range(&arg.value, &variant.ty);
        Ty::Choice(choice_name.to_string())
    }

    /// Check a variadic `printf(...)` call.
    ///
    /// The format must be a string literal so the conversions are known while
    /// checking. Each conversion consumes exactly one following value; the
    /// kind of that value is validated against the conversion. Callers never
    /// need casts because the C backend casts each vararg to what the
    /// conversion reads.
    fn check_printf_call(&mut self, args: &[CallArg], span: Span) -> Ty {
        let Some(fmt) = args.first() else {
            self.error(span, "printf expects a format string argument");
            return Ty::Void;
        };
        if fmt.direction != Direction::In {
            self.error(fmt.value.span, "printf expects a '<-' format string");
        }

        let ExprKind::String(text) = &fmt.value.kind else {
            self.error(fmt.value.span, "printf format must be a string literal");
            self.check_expr(&fmt.value);
            for arg in &args[1..] {
                self.check_unmatched_arg(arg);
            }
            return Ty::Void;
        };

        let pieces = match crate::format::parse_format(text) {
            Ok(pieces) => pieces,
            Err(message) => {
                self.error(fmt.value.span, format!("invalid format string: {message}"));
                for arg in &args[1..] {
                    self.check_unmatched_arg(arg);
                }
                return Ty::Void;
            }
        };
        let specs = pieces
            .iter()
            .filter_map(|piece| match piece {
                crate::format::FormatPiece::Spec(spec) => Some(spec),
                crate::format::FormatPiece::Text(_) => None,
            })
            .collect::<Vec<_>>();

        if specs.len() != args.len() - 1 {
            self.error(
                span,
                format!(
                    "printf format expects {} argument(s), found {}",
                    specs.len(),
                    args.len() - 1
                ),
            );
        }

        for (index, arg) in args[1..].iter().enumerate() {
            let Some(spec) = specs.get(index) else {
                self.check_unmatched_arg(arg);
                continue;
            };
            if arg.direction != Direction::In {
                self.error(
                    arg.value.span,
                    format!("printf expects a '<-' argument for '{}'", spec.raw),
                );
            }
            let ty = self.check_expr(&arg.value);
            let Some(kind) = spec.kind() else {
                continue;
            };
            let is_string = matches!(kind, crate::format::FormatKind::Str)
                && matches!(arg.value.kind, ExprKind::String(_));
            if !is_string && !ty.is_error() && !crate::format::type_matches(kind, &ty) {
                let expected = match kind {
                    crate::format::FormatKind::SignedInt => "an integer",
                    crate::format::FormatKind::UnsignedInt => "an integer",
                    crate::format::FormatKind::Float => "a float",
                    crate::format::FormatKind::Str => "a string literal or cstr",
                    crate::format::FormatKind::Char => "an integer",
                };
                self.error(
                    arg.value.span,
                    format!("format '{}' expects {expected}, found {ty}", spec.raw),
                );
            }
        }

        Ty::Void
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
            if consumed.get(index).is_none_or(|paths| paths.is_empty()) {
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
            let Some(relative_paths) = consumed.get(index) else {
                continue;
            };
            if relative_paths.is_empty() {
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
            if self.lookup(&root).is_some_and(|var| var.is_global) {
                self.error(
                    arg.value.span,
                    format!("cannot give from global '{root}'; use clone(...)"),
                );
                continue;
            }
            for relative in relative_paths {
                self.record_move(&root, &join_paths(&path, relative), arg.value.span);
            }
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

    /// Type-check a `<-` argument. A partially consumed composite may still be
    /// passed by read-only view when the callee's read summary avoids every
    /// consumed path; the normal whole-place rule is kept otherwise.
    fn check_read_argument(&mut self, value: &Expr, callee: &str, param_index: usize) -> Ty {
        let info = if is_place(value) {
            Some(self.lvalue_info(value))
        } else {
            None
        };
        let Some(info) = info else {
            return self.check_expr(value);
        };
        let (Some(root), Some(path)) = (info.root.clone(), info.path.clone()) else {
            return self.check_expr(value);
        };
        let Some(var) = self.lookup(&root) else {
            return self.check_expr(value);
        };
        let Some(reads) = self
            .param_reads
            .get(callee)
            .and_then(|params| params.get(param_index))
        else {
            return self.check_expr(value);
        };
        if var.moved.is_empty() || var.is_global {
            return self.check_expr(value);
        }
        if !var.initialized {
            self.error(
                value.span,
                format!("use of uninitialized variable '{root}'"),
            );
            return info.ty;
        }
        for read in reads {
            let effective = join_paths(&path, read);
            if path_is_moved(&var.moved, &effective) {
                self.error_moved(
                    value.span,
                    format!("use of moved value '{root}'"),
                    &root,
                    &effective,
                );
                return info.ty;
            }
        }
        info.ty
    }

    fn check_call_arg(
        &mut self,
        arg: &CallArg,
        param: &ParamSig,
        may_allocate: bool,
        callee: &str,
        param_index: usize,
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
                let actual = self.check_read_argument(&arg.value, callee, param_index);
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
        let element = self.resolve_type(element_type);

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
        let is_global = var.is_global;

        if is_global {
            self.error(
                value.span,
                format!("cannot give from global '{root}'; use clone(...)"),
            );
        } else if direction == Some(Direction::In) {
            self.error_readonly(
                value.span,
                format!("cannot give from '<-' parameter '{root}'"),
                &root,
                "declared as a '<-' parameter here",
            );
        } else if !initialized {
            self.error(
                value.span,
                format!("cannot give from uninitialized variable '{root}'"),
            );
        } else if moved {
            self.error_moved(
                value.span,
                format!("use of moved value '{root}'"),
                &root,
                &path,
            );
        } else if path.is_empty() || is_composite(&info.ty) {
            self.record_move(&root, &path, value.span);
        }
        info.ty
    }

    /// `clone(x)` is a deep copy. The source is read (so moved/uninitialized
    /// places are still rejected by `check_expr`) and stays live. When the type
    /// contains slices, the clone allocates fresh arena storage; region escape
    /// and the unguarded-allocation effect are handled by `collect_value_regions`
    /// and the effect prepass respectively.
    fn check_copy(&mut self, value: &Expr) -> Ty {
        self.check_expr(value)
    }

    // ------------------------------------------------------------------
    // Lvalues, members and indexes
    // ------------------------------------------------------------------
    /// Type of a place computed without mutating checker state. Used to detect
    /// writes into a choice payload before `lvalue_info` would accept them.
    fn static_place_ty(&self, expr: &Expr) -> Option<Ty> {
        match &expr.kind {
            ExprKind::Ident(name) => self.lookup(name).map(|info| info.ty.clone()),
            ExprKind::Member { base, field } => {
                let base_ty = self.static_place_ty(base)?;
                match &base_ty {
                    Ty::Named(name) => self.interfaces.get(name).and_then(|info| {
                        info.field_index
                            .get(field)
                            .map(|&index| info.fields[index].ty.clone())
                    }),
                    Ty::Choice(name) => self.choices.get(name).and_then(|info| {
                        info.variants.get(field).map(|variant| variant.ty.clone())
                    }),
                    Ty::Slice(_) if field == "len" => Some(Ty::U64),
                    _ => None,
                }
            }
            ExprKind::Index { base, .. } => match self.static_place_ty(base)? {
                Ty::Slice(element) => Some(*element),
                _ => None,
            },
            _ => None,
        }
    }

    /// Lightweight type resolution shared by the effect prepass and region
    /// tracking. `locals` supplies parameters and declarations discovered by
    /// the prepass; during body checking it is empty and `self.lookup` resolves
    /// every name. Unknown expressions resolve to `Ty::Error`, which is never
    /// reported as containing a slice, so this helper stays diagnostic-free.
    fn scan_expr_ty(&self, expr: &Expr, locals: &HashMap<String, Ty>) -> Ty {
        match &expr.kind {
            ExprKind::Int(_) => Ty::IntLiteral,
            ExprKind::Float(_) => Ty::FloatLiteral,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::String(_) => Ty::Slice(Box::new(Ty::U8)),
            ExprKind::Ident(name) => locals
                .get(name)
                .cloned()
                .or_else(|| self.lookup(name).map(|info| info.ty.clone()))
                .unwrap_or(Ty::Error),
            ExprKind::Member { base, field } => match self.scan_expr_ty(base, locals) {
                Ty::Named(name) => self
                    .interfaces
                    .get(&name)
                    .and_then(|info| {
                        info.field_index
                            .get(field)
                            .map(|&index| info.fields[index].ty.clone())
                    })
                    .unwrap_or(Ty::Error),
                Ty::Choice(name) => self
                    .choices
                    .get(&name)
                    .and_then(|info| info.variants.get(field).map(|variant| variant.ty.clone()))
                    .unwrap_or(Ty::Error),
                Ty::Slice(_) if field == "len" => Ty::U64,
                _ => Ty::Error,
            },
            ExprKind::Index { base, .. } => match self.scan_expr_ty(base, locals) {
                Ty::Slice(element) => *element,
                _ => Ty::Error,
            },
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Ident(name) => self
                    .functions
                    .get(name)
                    .map(|signature| signature.return_type.clone())
                    .unwrap_or(Ty::Error),
                _ => Ty::Error,
            },
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                let element = element_type
                    .as_ref()
                    .map(|ty| self.resolve_type_pure(ty))
                    .or_else(|| {
                        elements
                            .first()
                            .map(|first| self.scan_expr_ty(first, locals))
                    })
                    .unwrap_or(Ty::Error);
                Ty::Slice(Box::new(element))
            }
            ExprKind::InterfaceLiteral { name, .. } => Ty::Named(name.clone()),
            ExprKind::Cast { ty, .. } => self.resolve_type_pure(ty),
            ExprKind::Unary { operand, .. } => self.scan_expr_ty(operand, locals),
            ExprKind::Binary { op, left, .. } => match op {
                BinaryOp::Is => Ty::Bool,
                _ => self.scan_expr_ty(left, locals),
            },
            ExprKind::Move(inner)
            | ExprKind::Copy(inner)
            | ExprKind::Comptime(inner)
            | ExprKind::Alloc { target: inner, .. } => self.scan_expr_ty(inner, locals),
        }
    }

    /// Whether `expr` reads the payload of a choice, as in `result.Ok`. Such a
    /// place is a checked read-only view: assigning to it would bypass the
    /// runtime variant check, so the checker rejects it.
    fn is_choice_payload_place(&self, expr: &Expr) -> bool {
        let ExprKind::Member { base, .. } = &expr.kind else {
            return false;
        };
        matches!(self.static_place_ty(base), Some(Ty::Choice(_)))
    }

    fn lvalue_info(&mut self, expr: &Expr) -> LvalueInfo {
        match &expr.kind {
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(info) => LvalueInfo {
                    ty: info.ty.clone(),
                    root: Some(name.clone()),
                    path: Some(String::new()),
                    poisoned: false,
                },
                None => {
                    self.error_unknown(
                        expr.span,
                        format!("unknown variable '{name}'"),
                        name,
                        NameSpace::Variable,
                    );
                    LvalueInfo {
                        ty: Ty::Error,
                        root: None,
                        path: None,
                        poisoned: true,
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
                    poisoned: base_info.poisoned,
                }
            }
            ExprKind::Index { base, index } => {
                let base_info = self.lvalue_info(base);
                let index_ty = self.check_expr(index);
                let ty = self.check_index(&base_info.ty, index, &index_ty, expr.span);
                LvalueInfo {
                    ty,
                    root: base_info.root,
                    path: base_info
                        .path
                        .map(|path| append_path(&path, &index_path(index))),
                    poisoned: base_info.poisoned,
                }
            }
            _ => {
                self.error(expr.span, "invalid assignment target");
                LvalueInfo {
                    ty: Ty::Error,
                    root: None,
                    path: None,
                    poisoned: true,
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
            Ty::Choice(name) => match self.choices.get(name) {
                Some(info) => match info.variants.get(field) {
                    Some(variant) => variant.ty.clone(),
                    None => {
                        self.error(span, format!("choice '{name}' has no variant '{field}'"));
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
        if info.poisoned {
            return None;
        }
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
                .keys()
                .any(|moved| !moved.is_empty() && path_is_ancestor(&path, moved));

        if direction == Some(Direction::In) {
            self.error_readonly(
                span,
                format!("cannot write to '<-' parameter '{name}'"),
                &name,
                "declared as a '<-' parameter here",
            );
            return false;
        }

        if !initialized && !initializing {
            self.error(span, format!("use of uninitialized variable '{name}'"));
            return false;
        }

        if initialized && !mutable {
            self.error_readonly(
                span,
                format!("cannot assign to immutable variable '{name}'"),
                &name,
                "declared immutable here",
            );
            return false;
        }

        if moved_here && !path.is_empty() && !reinitializes {
            self.error_moved(
                span,
                format!("cannot write to moved value '{name}'"),
                &name,
                &path,
            );
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
            self.error_readonly(
                span,
                format!("cannot write to '<-' parameter '{name}'"),
                &name,
                "declared as a '<-' parameter here",
            );
            return false;
        }
        if !initialized {
            self.error(span, format!("use of uninitialized variable '{name}'"));
            return false;
        }
        if moved {
            self.error_moved(span, format!("use of moved value '{name}'"), &name, &path);
            return false;
        }
        if !mutable {
            self.error_readonly(
                span,
                format!("cannot assign to immutable variable '{name}'"),
                &name,
                "declared immutable here",
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
                    info.moved.retain(|moved, _| !path_is_ancestor(path, moved));
                }
                info.taints
                    .retain(|taint| !paths_overlap(&taint.path, path));
                return;
            }
        }
    }

    fn record_move(&mut self, name: &str, path: &str, span: Span) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                if info.is_global {
                    return;
                }
                info.moved.retain(|moved, _| !path_is_ancestor(path, moved));
                info.moved.insert(path.to_string(), span);
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

    fn begin_region(&mut self, kind: RegionKind, origin: Option<Span>) -> usize {
        let id = self.region_parents.len();
        self.region_parents.push(self.current_region());
        self.region_kinds.push(kind);
        self.region_origins.push(origin);
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
                let (message, label) = match self.region_kinds.get(*region) {
                    Some(RegionKind::Protect) => (
                        "value allocated inside 'protect' escapes the block".to_string(),
                        "allocated in this 'protect' block",
                    ),
                    _ => (
                        "slice literal storage escapes the block that created it; keep it in a variable declared in the same block".to_string(),
                        "allocated in this block",
                    ),
                };
                let origin = self.region_origins.get(*region).copied().flatten();
                let mut diagnostic =
                    Diagnostic::from_error(message, span).with_label("escapes here");
                if let Some(origin) = origin {
                    diagnostic = diagnostic.with_secondary(origin, label);
                }
                self.diagnostics.push(diagnostic);
                return;
            }
        }
    }

    /// Allocate a fresh protect region for a `protect` body.
    fn enter_protect(&mut self, span: Span) -> usize {
        self.begin_region(RegionKind::Protect, Some(span))
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
            ExprKind::Call { callee, args, .. } => {
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
                // A callee that recursively clones slice-containing values
                // allocates its result in the caller's active arena, which
                // must be a `protect` region that outlives the result.
                if let ExprKind::Ident(name) = &callee.kind
                    && self.clone_allocators.contains(name)
                    && let Some(region) = self.current_protect_region()
                {
                    regions.push(region);
                }
                if let ExprKind::Ident(name) = &callee.kind
                    && let Some(signature) = self.functions.get(name)
                    && self.type_contains_slice(&signature.return_type)
                    && let Some(consumed) = self.param_consumption.get(name)
                {
                    for (index, arg) in args.iter().enumerate() {
                        if consumed.get(index).is_none_or(|paths| paths.is_empty()) {
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
            ExprKind::Move(value) => self.collect_value_regions(value, regions),
            ExprKind::Copy(value) => {
                // A deep clone owns fresh storage and never aliases the source,
                // so it does not inherit the source's region taints. The new
                // storage belongs to the innermost `protect` region active at
                // the clone site.
                let value_ty = self.scan_expr_ty(value, &HashMap::new());
                if self.type_contains_slice(&value_ty)
                    && let Some(region) = self.current_protect_region()
                {
                    regions.push(region);
                }
            }
            ExprKind::InterfaceLiteral { fields, .. } => {
                for field in fields {
                    self.collect_value_regions(&field.value, regions);
                }
            }
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {}
            ExprKind::SliceLiteral { elements, .. } => {
                // A non-empty literal's backing array has automatic storage in
                // the enclosing lexical block, so it must not escape. An empty
                // literal allocates nothing: the C backend emits `{ NULL, 0 }`,
                // which is safe to store anywhere (runtime indexing checks the
                // length before dereferencing). Only element expressions can
                // still carry regions. String literals are static and
                // deliberately do not reach this branch.
                if !elements.is_empty()
                    && let Some(region) = self.current_region()
                {
                    regions.push(region);
                }
                for element in elements {
                    self.collect_value_regions(element, regions);
                }
            }
            ExprKind::Comptime(inner) => self.collect_value_regions(inner, regions),
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

    /// Whether giving a place transfers ownership out of its root.
    ///
    /// Giving a whole variable keeps the historical "explicit transfer" rule,
    /// even for scalars. Giving a sub-place only consumes when that sub-place
    /// owns non-copyable data; scalar fields and elements are copied implicitly.
    fn move_consumes_place(&self, ty: &Ty, path: &str) -> bool {
        if path.is_empty() {
            return true;
        }
        match self.type_at_path(ty, path) {
            Some(actual) => is_composite(&actual),
            None => true,
        }
    }

    /// Resolve the type of a root-relative ownership path.
    fn type_at_path(&self, ty: &Ty, path: &str) -> Option<Ty> {
        let mut current = ty.clone();
        for component in parse_path(path) {
            current = match component {
                PathComponent::Field(field) => match &current {
                    Ty::Slice(_) if field == "len" => Ty::U64,
                    Ty::Named(name) => {
                        let info = self.interfaces.get(name)?;
                        let index = *info.field_index.get(&field)?;
                        info.fields[index].ty.clone()
                    }
                    Ty::Choice(name) => self.choices.get(name)?.variants.get(&field)?.ty.clone(),
                    _ => return None,
                },
                PathComponent::Index(_) => match current {
                    Ty::Slice(element) => *element,
                    _ => return None,
                },
            };
        }
        Some(current)
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
                for (path, span) in &right_state.moved {
                    moved.entry(path.clone()).or_insert(*span);
                }
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
            | "printf"
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
fn scan_block_effects(
    checker: &Checker,
    block: &Block,
    depth: usize,
    scan: &mut FunctionScan,
    locals: &mut HashMap<String, Ty>,
) {
    for statement in &block.statements {
        scan_statement_effects(checker, statement, depth, scan, locals);
    }
}

fn scan_statement_effects(
    checker: &Checker,
    statement: &Stmt,
    depth: usize,
    scan: &mut FunctionScan,
    locals: &mut HashMap<String, Ty>,
) {
    match &statement.kind {
        StmtKind::Let {
            name, ty, value, ..
        } => {
            if let Some(value) = value {
                scan.record_reads(value);
                scan_expr_effects(checker, value, depth, scan, locals);
            }
            let inferred = ty
                .as_ref()
                .map(|ty| checker.resolve_type_pure(ty))
                .or_else(|| {
                    value
                        .as_ref()
                        .map(|value| checker.scan_expr_ty(value, locals))
                });
            if let Some(ty) = inferred {
                locals.insert(name.clone(), ty);
            }
        }
        StmtKind::Assign { target, value, .. } => {
            scan.record_reads(target);
            scan.record_reads(value);
            scan_expr_effects(checker, target, depth, scan, locals);
            scan_expr_effects(checker, value, depth, scan, locals);
        }
        StmtKind::Expr(expr) => {
            scan.record_reads(expr);
            scan_expr_effects(checker, expr, depth, scan, locals);
        }
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            scan.record_reads(condition);
            scan_expr_effects(checker, condition, depth, scan, locals);
            scan_block_effects(checker, then_block, depth, scan, locals);
            match else_branch {
                Some(ElseBranch::Block(block)) => {
                    scan_block_effects(checker, block, depth, scan, locals)
                }
                Some(ElseBranch::If(inner)) => {
                    scan_statement_effects(checker, inner, depth, scan, locals)
                }
                None => {}
            }
        }
        StmtKind::While { condition, body } => {
            scan.record_reads(condition);
            scan_expr_effects(checker, condition, depth, scan, locals);
            scan_block_effects(checker, body, depth, scan, locals);
        }
        StmtKind::ForRange {
            variable,
            start,
            end,
            body,
        } => {
            scan.record_reads(start);
            scan.record_reads(end);
            scan_expr_effects(checker, start, depth, scan, locals);
            scan_expr_effects(checker, end, depth, scan, locals);
            locals.insert(variable.clone(), Ty::U64);
            scan_block_effects(checker, body, depth, scan, locals);
        }
        StmtKind::ForEach {
            variable,
            iterable,
            body,
            ..
        } => {
            scan.record_reads(iterable);
            scan_expr_effects(checker, iterable, depth, scan, locals);
            let element = match checker.scan_expr_ty(iterable, locals) {
                Ty::Slice(element) => *element,
                _ => Ty::Error,
            };
            locals.insert(variable.clone(), element);
            scan_block_effects(checker, body, depth, scan, locals);
        }
        StmtKind::Protect(block) => scan_block_effects(checker, block, depth + 1, scan, locals),
        StmtKind::Return(value) => {
            if let Some(value) = value {
                if scan.returns_composite
                    && let (Some(root), Some(path)) = (place_root(value), place_path(value))
                {
                    scan.moved_places.insert((root, path));
                }
                scan.record_reads(value);
                scan_expr_effects(checker, value, depth, scan, locals);
            }
        }
        StmtKind::Continue => {}
        StmtKind::Break => {}
    }
}

fn scan_expr_effects(
    checker: &Checker,
    expr: &Expr,
    depth: usize,
    scan: &mut FunctionScan,
    locals: &mut HashMap<String, Ty>,
) {
    match &expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => {}
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                scan_expr_effects(checker, element, depth, scan, locals);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                scan_expr_effects(checker, &field.value, depth, scan, locals);
            }
        }
        ExprKind::Unary { operand, .. } => scan_expr_effects(checker, operand, depth, scan, locals),
        ExprKind::Binary { left, right, .. } => {
            scan_expr_effects(checker, left, depth, scan, locals);
            scan_expr_effects(checker, right, depth, scan, locals);
        }
        ExprKind::Cast { expr, .. } => scan_expr_effects(checker, expr, depth, scan, locals),
        ExprKind::Call { callee, args, .. } => {
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
                        .map(|arg| {
                            let place = place_root(&arg.value).zip(place_path(&arg.value));
                            (arg.direction, place)
                        })
                        .collect(),
                });
            } else {
                scan_expr_effects(checker, callee, depth, scan, locals);
            }
            for arg in args {
                scan_expr_effects(checker, &arg.value, depth, scan, locals);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            if depth == 0 {
                scan.direct_alloc = true;
            }
            scan_expr_effects(checker, target, depth, scan, locals);
            scan_expr_effects(checker, count, depth, scan, locals);
        }
        ExprKind::Move(value) => {
            if let (Some(root), Some(path)) = (place_root(value), place_path(value)) {
                scan.moved_places.insert((root, path));
            }
            scan_expr_effects(checker, value, depth, scan, locals);
        }
        ExprKind::Copy(value) => {
            let value_ty = checker.scan_expr_ty(value, locals);
            if checker.type_contains_slice(&value_ty) {
                scan.clone_direct = true;
                if depth == 0 {
                    scan.direct_alloc = true;
                }
            }
            scan_expr_effects(checker, value, depth, scan, locals);
        }
        ExprKind::Member { base, .. } => scan_expr_effects(checker, base, depth, scan, locals),
        ExprKind::Index { base, index } => {
            scan_expr_effects(checker, base, depth, scan, locals);
            scan_expr_effects(checker, index, depth, scan, locals);
        }
        ExprKind::Comptime(inner) => scan_expr_effects(checker, inner, depth, scan, locals),
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathComponent {
    Field(String),
    /// `None` is a runtime index (`[]`); it overlaps any constant index.
    Index(Option<i128>),
}

fn parse_path(path: &str) -> Vec<PathComponent> {
    let bytes = path.as_bytes();
    let mut components = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'.' => index += 1,
            b'[' => {
                let Some(relative_close) = path[index + 1..].find(']') else {
                    break;
                };
                let close = index + 1 + relative_close;
                components.push(PathComponent::Index(path[index + 1..close].parse().ok()));
                index = close + 1;
            }
            _ => {
                let start = index;
                while index < bytes.len() && bytes[index] != b'.' && bytes[index] != b'[' {
                    index += 1;
                }
                components.push(PathComponent::Field(path[start..index].to_string()));
            }
        }
    }
    components
}

/// True when `ancestor` is the same path as `path` or contains it, respecting
/// path-component boundaries. A runtime index (`[]`) contains every constant
/// element of the same slice but is not itself contained by one element.
fn path_is_ancestor(ancestor: &str, path: &str) -> bool {
    if ancestor == path || ancestor.is_empty() {
        return true;
    }
    let ancestor = parse_path(ancestor);
    let path = parse_path(path);
    if ancestor.len() > path.len() {
        return false;
    }
    ancestor
        .into_iter()
        .zip(path)
        .all(|(left, right)| match (left, right) {
            (PathComponent::Field(left), PathComponent::Field(right)) => left == right,
            (PathComponent::Index(None), PathComponent::Index(_)) => true,
            (PathComponent::Index(Some(left)), PathComponent::Index(Some(right))) => left == right,
            _ => false,
        })
}

/// Append one root-relative path after another. Both forms already carry their
/// own separators (`.field` or `[index]`).
fn join_paths(prefix: &str, relative: &str) -> String {
    if relative.is_empty() {
        prefix.to_string()
    } else if prefix.is_empty() {
        relative.to_string()
    } else {
        format!("{prefix}{relative}")
    }
}

/// Add a consumed path to a summary, collapsing all paths into `""` when the
/// whole parameter is consumed.
fn insert_consumed_path(paths: &mut HashSet<String>, path: String) -> bool {
    if paths.contains("") {
        return false;
    }
    if path.is_empty() {
        paths.clear();
        paths.insert(String::new());
        true
    } else {
        paths.insert(path)
    }
}

/// Add a read path to a summary. A whole-value read subsumes every narrower
/// path, so the set collapses to `""` just like consumed paths do.
fn insert_read_path(paths: &mut HashSet<String>, path: String) -> bool {
    insert_consumed_path(paths, path)
}

fn paths_overlap(left: &str, right: &str) -> bool {
    path_is_ancestor(left, right) || path_is_ancestor(right, left)
}

fn path_is_moved(moved: &HashMap<String, Span>, path: &str) -> bool {
    moved.keys().any(|moved| paths_overlap(moved, path))
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
            place_path(base).map(|path| append_path(&path, &index_path(index)))
        }
        _ => None,
    }
}

/// Ownership-path component for an index expression. A constant index keeps
/// its value; an unknown index is a wildcard that overlaps every element.
fn index_path(index: &Expr) -> String {
    match const_int(index) {
        Some(value) => format!("[{value}]"),
        None => "[]".to_string(),
    }
}

fn is_composite(ty: &Ty) -> bool {
    matches!(ty, Ty::Named(_) | Ty::Choice(_) | Ty::Slice(_))
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
        ExprKind::Comptime(inner) => collect_accesses(inner, accesses),
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

/// Collect the root-relative places an expression reads directly from `<-`
/// parameters. Unlike [`collect_accesses`], a whole-value `<-` argument is not
/// recorded: the interprocedural read-summary fixpoint refines it to the fields
/// the callee actually touches. Index expressions are still read directly.
fn collect_direct_reads(
    expr: &Expr,
    read_params: &HashMap<String, usize>,
    reads: &mut [HashSet<String>],
) {
    match &expr.kind {
        ExprKind::Ident(name) => {
            if let Some(&index) = read_params.get(name) {
                insert_read_path(&mut reads[index], String::new());
            }
        }
        ExprKind::Member { .. } | ExprKind::Index { .. } => {
            if let (Some(root), Some(path)) = (place_root(expr), place_path(expr))
                && let Some(&index) = read_params.get(&root)
            {
                insert_read_path(&mut reads[index], path);
            }
            collect_direct_index_reads(expr, read_params, reads);
        }
        ExprKind::Unary { operand, .. } => collect_direct_reads(operand, read_params, reads),
        ExprKind::Binary { left, right, .. } => {
            collect_direct_reads(left, read_params, reads);
            collect_direct_reads(right, read_params, reads);
        }
        ExprKind::Cast { expr, .. } => collect_direct_reads(expr, read_params, reads),
        ExprKind::Call { args, .. } => {
            for arg in args {
                let whole_borrowed_arg = place_root(&arg.value)
                    .zip(place_path(&arg.value))
                    .is_some_and(|(root, path)| path.is_empty() && read_params.contains_key(&root));
                if !whole_borrowed_arg {
                    collect_direct_reads(&arg.value, read_params, reads);
                }
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            collect_direct_index_reads(target, read_params, reads);
            collect_direct_reads(count, read_params, reads);
        }
        ExprKind::Move(value) | ExprKind::Copy(value) => {
            collect_direct_reads(value, read_params, reads)
        }
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                collect_direct_reads(element, read_params, reads);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                collect_direct_reads(&field.value, read_params, reads);
            }
        }
        ExprKind::Comptime(inner) => collect_direct_reads(inner, read_params, reads),
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {}
    }
}

fn collect_direct_index_reads(
    expr: &Expr,
    read_params: &HashMap<String, usize>,
    reads: &mut [HashSet<String>],
) {
    match &expr.kind {
        ExprKind::Member { base, .. } => collect_direct_index_reads(base, read_params, reads),
        ExprKind::Index { base, index } => {
            collect_direct_index_reads(base, read_params, reads);
            collect_direct_reads(index, read_params, reads);
        }
        _ => {}
    }
}
