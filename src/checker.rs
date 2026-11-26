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

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::flow::{
    Borrow, BorrowMode, DataPath, FlowEffect, FlowSummary, IndexKey, LoanDestination, LoanSource,
    PathSegment,
};
use crate::types::{self, CallbackSig, ComparisonEnv, Ty};

fn is_callback_void(ty: &Type) -> bool {
    matches!(ty, Type::Named(name, _) if name == "void")
}

/// Check a complete program, returning every diagnostic found.
pub fn check(program: &Program) -> Vec<Diagnostic> {
    let program = match crate::expand::expand_program(program) {
        Ok(program) => program,
        Err(errors) => return errors,
    };
    let (program, mono_errors, source_names) =
        crate::monomorphize::monomorphize_for_check(&program);
    let has_mono_errors = !mono_errors.is_empty();
    let mut diagnostics = mono_errors
        .into_iter()
        .map(|error| Diagnostic::from_error(error.message, error.span))
        .collect::<Vec<_>>();
    if has_mono_errors {
        // Generic errors (missing arguments, recursion limits, name
        // collisions) leave the program partly rewritten. Checking it would
        // only add cascades about unknown generated names. Bound diagnostics
        // already carry source spelling from the monomorphizer; rewriting all
        // mono errors here would hide the generated name in collision reports.
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
    checker.analyze_flow_summaries(&program);
    checker.check_interface_defaults(&program);
    checker.declare_globals(&program);
    checker.check_functions(&program);
    diagnostics.extend(checker.diagnostics);
    show_source_names(&mut diagnostics, &source_names);
    diagnostics
}

/// Check a program and also attach source-level warnings from that program.
pub fn check_with_warnings(program: &Program) -> Vec<Diagnostic> {
    check_with_warnings_from(program, program)
}

/// Check `program` and attach warnings from `warning_program`.
///
/// Warnings are computed from the unflattened AST of one source file, so their
/// spans and names match the text the user wrote and imported modules cannot
/// report warnings against the wrong file. They are intended for the CLI and
/// editor tooling; `check` keeps returning errors only for embedders that rely
/// on that shape.
pub fn check_with_warnings_from(program: &Program, warning_program: &Program) -> Vec<Diagnostic> {
    let errors = check(program);
    // Do not pile style warnings on top of a program that already has errors:
    // unused bindings in broken code are usually a consequence of the error.
    if errors.iter().any(|diagnostic| diagnostic.is_error()) {
        return errors;
    }
    // Lint the expanded form so spliced statements count as uses and bindings.
    let lint_program =
        crate::expand::expand_program(warning_program).unwrap_or_else(|_| warning_program.clone());
    let mut diagnostics = crate::lint::warnings(&lint_program);
    diagnostics.extend(errors);
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
    /// For a variadic parameter this is the slice `[]T` seen by the flow body.
    ty: Ty,
    variadic: bool,
}

#[derive(Debug, Clone)]
struct FunctionSig {
    span: Span,
    params: Vec<ParamSig>,
    return_type: Ty,
    external: bool,
}

#[derive(Debug, Clone)]
struct VarInfo {
    ty: Ty,
    mutable: bool,
    initialized: bool,
    direction: Option<Direction>,
    span: Span,
    /// Paths moved out of this variable, with the move site. A path without
    /// segments means the whole variable.
    moved: HashMap<DataPath, Span>,
    /// Lexical block or `protect` region where this variable's storage lives;
    /// `None` means a parameter or the caller.
    home_region: Option<usize>,
    /// `protect` allocations or block-local slice literals reachable through
    /// this variable.
    taints: Vec<RegionTaint>,
    /// Storage borrowed by slice views held in this variable. Each loan keeps
    /// the source place alive for as long as the view variable is in scope.
    views: Vec<ViewLoan>,
    /// Top-level bindings are stored in the base scope. Their move state is
    /// never recorded: ownership is tracked per function body.
    is_global: bool,
}

#[derive(Debug, Clone)]
struct RegionTaint {
    path: DataPath,
    region: usize,
}

/// A slice view that aliases storage owned by another place.
#[derive(Debug, Clone)]
struct ViewLoan {
    /// Root variable whose storage is borrowed.
    source: String,
    /// Typed root-relative path of the borrowed place. A path without segments
    /// means the whole root.
    path: DataPath,
    /// Holder-relative typed path of the slot storing this view. Kept separate
    /// from `path`: a view of `x` stored in `s.a` borrows `x` but lives in `a`.
    slot: DataPath,
    /// Where the view was created, used for the diagnostic label.
    span: Span,
}

/// A `<-` source and the root-relative place whose storage was borrowed.
///
/// Keeping the path here is what lets a durable `Borrow` name `p.left` instead
/// of collapsing every loan to the whole parameter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ViewSource {
    index: usize,
    path: DataPath,
}

impl ViewSource {
    fn whole(index: usize) -> Self {
        Self {
            index,
            path: DataPath::default(),
        }
    }

    fn extended_by(&self, relative: &DataPath) -> Self {
        Self {
            index: self.index,
            path: self.path.append_relative(relative),
        }
    }

    /// Re-root this source on the parameter name used by the caller.
    fn data_path(&self, root: &str) -> DataPath {
        self.path.clone().with_root(root)
    }
}

/// A view reaching a `->` parameter together with the holder's slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ViewOutLoan {
    slot: DataPath,
    source: ViewSource,
}

/// Mutable state used while summarizing view flow inside one function scan.
///
/// It is temporary scan state, not a durable contract: the durable result is
/// recorded as `Borrow` effects in `FlowSummary`.
#[derive(Debug, Default)]
struct ViewScanState {
    /// Sources whose views a local may carry.
    locals: HashMap<String, BTreeSet<ViewSource>>,
    /// Sources reaching each `->` parameter, with the slot that stores them.
    out: HashMap<String, BTreeSet<ViewOutLoan>>,
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
    /// Typed places consumed by `give(...)` in this body, paired with root name.
    moved_places: HashSet<(String, DataPath)>,
    /// Typed places assigned in this body, paired with root name.
    written_places: HashSet<(String, DataPath)>,
    /// Typed places initialized by `alloc(-> place, ...)`, paired with root name.
    initialized_places: HashSet<(String, DataPath)>,
    /// Whether a value reaches a `=>` result in this body.
    produces_result: bool,
    /// Places where this body delivers a value to the caller: writes and
    /// allocations into a `->` parameter and the `=>` result. Routed to
    /// `ResultEffects` by the fixpoint as `PRODUCE` effects.
    produced_places: HashSet<(String, DataPath)>,
    /// Call sites with the direction and typed place of every argument.
    call_args: Vec<CallSiteScan>,
    /// Whether returning a place transfers ownership (composite return type).
    returns_composite: bool,
    /// `<-` parameter name to its parameter index. Used to record which parts
    /// of a borrowed parameter the body actually reads.
    read_params: HashMap<String, usize>,
    /// Per `<-` parameter index, the typed places the body reads directly
    /// (excluding whole-value `<-` arguments refined by the fixpoint).
    direct_reads: Vec<HashSet<DataPath>>,
    /// Sources that reached a `=>` result in the current scan iteration.
    view_returned: BTreeSet<ViewSource>,
    /// View sources carried by locals and `->` parameters in this iteration.
    views: ViewScanState,
}

impl FunctionScan {
    fn record_reads(&mut self, expr: &Expr) {
        collect_direct_reads(expr, &self.read_params, &mut self.direct_reads);
    }
}

struct CallSiteScan {
    callee: String,
    args: Vec<(Direction, Option<(String, DataPath)>)>,
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
    moved: HashMap<DataPath, Span>,
    taints: Vec<RegionTaint>,
    views: Vec<ViewLoan>,
}

/// One active `guard` binding in a function body.
struct GuardCleanup {
    name: String,
    release: String,
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
    /// Control-state snapshots taken at `break`/`continue` sites. A loop merges
    /// these into its fall-through state: a value moved on a path that leaves
    /// the loop early is still moved after the loop. Without this, a `give`
    /// inside an `if` whose branch ends in `break` is silently discarded by
    /// the `if` join.
    loop_exit_states: Vec<Vec<HashMap<String, ControlState>>>,
    /// Active linear-resource guards, innermost last.
    guards: Vec<GuardCleanup>,
    /// Single durable contract for every canonical function. Legacy maps for
    /// consumption, reads, views and allocation are derived through the
    /// compatibility accessors on `FlowSummary`.
    flow_summaries: HashMap<String, FlowSummary>,
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
    /// True while checking the argument expression of `print`, `eprint` or
    /// `panic`. Only the root call may be `fmt::(...)`: it allocates through
    /// the same runtime helper but never escapes there, so it does not require
    /// a caller `protect` region.
    fmt_direct_arg: bool,
}

impl ComparisonEnv for Checker {
    fn shape_fields(&self, name: &str) -> Option<Vec<Ty>> {
        self.interfaces
            .get(name)
            .map(|decl| decl.fields.iter().map(|field| field.ty.clone()).collect())
    }

    fn choice_payloads(&self, name: &str) -> Option<Vec<Ty>> {
        self.choices.get(name).map(|decl| {
            decl.variants
                .values()
                .map(|variant| variant.ty.clone())
                .collect()
        })
    }
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
            loop_exit_states: Vec::new(),
            guards: Vec::new(),
            flow_summaries: HashMap::new(),
            global_names: HashSet::new(),
            region_parents: Vec::new(),
            region_kinds: Vec::new(),
            region_origins: Vec::new(),
            region_stack: Vec::new(),
            fmt_direct_arg: false,
        }
    }

    fn summary(&self, name: &str) -> Option<&FlowSummary> {
        self.flow_summaries.get(name)
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
        let path = relative_data_path(path);
        self.lookup(root).and_then(|var| {
            var.moved
                .iter()
                .find(|(moved, _)| moved.may_overlap(&path))
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
                    if is_intrinsic_name(&decl.name) {
                        self.error(
                            decl.span,
                            format!("intrinsic '{}' cannot be redeclared", decl.name),
                        );
                    } else if self.interfaces.contains_key(&decl.name) {
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
                                external: decl.external,
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
                variadic: false,
            })
            .collect();
        self.functions.insert(
            name.to_string(),
            FunctionSig {
                span: Span::new(0, 0),
                params: sig_params,
                return_type,
                external: false,
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
            let variadic_count = decl.params.iter().filter(|param| param.variadic).count();
            let variadic_index = decl.params.iter().position(|param| param.variadic);
            if variadic_count > 1 {
                self.error(
                    decl.span,
                    format!(
                        "function '{}' has more than one variadic parameter",
                        decl.name
                    ),
                );
            }
            if let Some(index) = variadic_index
                && index + 1 != decl.params.len()
            {
                self.error(
                    decl.params[index].span,
                    format!(
                        "variadic parameter '{}' must be the last parameter",
                        decl.params[index].name
                    ),
                );
            }
            for param in &decl.params {
                let ty = self.resolve_type(&param.ty);
                if matches!(ty, Ty::Callback(_)) && param.direction != Direction::In {
                    self.error(
                        param.span,
                        format!(
                            "callback parameter '{}' must use '<-'; callbacks are input-only",
                            param.name
                        ),
                    );
                }
                if param.variadic {
                    if param.direction != Direction::In {
                        self.error(
                            param.span,
                            format!(
                                "variadic parameter '{}' must use '...', not a direction",
                                param.name
                            ),
                        );
                    }
                    if decl.external {
                        self.error(
                            param.span,
                            format!(
                                "extern flow '{}' cannot have a variadic parameter",
                                decl.name
                            ),
                        );
                    }
                    let scalar = ty.is_numeric()
                        || matches!(ty, Ty::Bool | Ty::CStr | Ty::Opaque(_) | Ty::Error);
                    if !scalar {
                        self.error(
                            param.span,
                            format!(
                                "variadic parameter '{}' must have a scalar or cstr element type, found {ty}",
                                param.name
                            ),
                        );
                    }
                }
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
                    ty: if param.variadic {
                        Ty::Slice(Box::new(ty))
                    } else {
                        ty
                    },
                    variadic: param.variadic,
                });
            }

            let return_type = match &decl.return_type {
                Some(ty) => self.resolve_type(ty),
                None => Ty::Void,
            };
            if decl.external && matches!(return_type, Ty::Callback(_)) {
                self.error(
                    decl.return_type
                        .as_ref()
                        .map(|ty| ty.span())
                        .unwrap_or(decl.span),
                    format!(
                        "extern flow '{}' cannot return callback; callbacks are input-only",
                        decl.name
                    ),
                );
            }

            self.functions.insert(
                decl.name.clone(),
                FunctionSig {
                    span: decl.span,
                    params,
                    return_type,
                    external: decl.external,
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
                    views: Vec::new(),
                    is_global: true,
                },
            );
        }
    }

    fn resolve_callback_sig(&mut self, params: &[Type], result: Option<&Type>) -> CallbackSig {
        let mut resolved = Vec::new();
        for param in params {
            if is_callback_void(param) {
                self.error(
                    param.span(),
                    "callback parameter type cannot be void; omit the parameter instead",
                );
                resolved.push(Ty::Error);
            } else {
                resolved.push(self.resolve_type(param));
            }
        }
        let result = match result {
            None => Ty::Void,
            Some(ty) => self.resolve_type(ty),
        };
        CallbackSig {
            params: resolved,
            result: Box::new(result),
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
                } else if name == "callback" {
                    Ty::Callback(CallbackSig::fixed())
                } else if name == "void" {
                    self.error(span, "'void' is only allowed as a callback result");
                    Ty::Error
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
            Type::Callback { params, result, .. } => {
                Ty::Callback(self.resolve_callback_sig(params, result.as_deref()))
            }
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
                } else if name == "callback" {
                    Ty::Callback(CallbackSig::fixed())
                } else if name == "void" {
                    Ty::Void
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
            Type::Callback { params, result, .. } => {
                let params = params
                    .iter()
                    .map(|param| self.resolve_type_pure(param))
                    .collect();
                let result = match result.as_deref() {
                    None => Ty::Void,
                    Some(ty) => self.resolve_type_pure(ty),
                };
                Ty::Callback(CallbackSig {
                    params,
                    result: Box::new(result),
                })
            }
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
    /// Compute one durable `FlowSummary` per canonical function before bodies
    /// are checked.
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
    ///
    /// A single visitor over each body records allocation, consumption, reads
    /// and borrowed-view effects. One monotone fixpoint joins those effects
    /// along calls, including recursion, directly into `FlowSummary`.
    /// `FunctionScan` and `ViewScanState` exist only while a body is scanned.
    fn analyze_flow_summaries(&mut self, program: &Program) {
        let globals: Vec<(String, Ty)> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Global(global) => {
                    let ty = global
                        .ty
                        .as_ref()
                        .map(|ty| self.resolve_type_pure(ty))
                        .unwrap_or_else(|| self.scan_expr_ty(&global.value, &HashMap::new()));
                    Some((global.name.clone(), ty))
                }
                _ => None,
            })
            .collect();

        let mut bodies: HashMap<String, &Block> = HashMap::new();
        let mut params: HashMap<String, Vec<(String, Direction, Ty)>> = HashMap::new();
        let mut local_types: HashMap<String, Vec<(String, Ty)>> = HashMap::new();
        let mut spans: Vec<(String, Span)> = Vec::new();

        for item in &program.items {
            let Item::Function(decl) = item else {
                continue;
            };
            if !self.is_canonical_function(&decl.name, decl.span) {
                continue;
            }
            let Some(signature) = self.functions.get(&decl.name) else {
                continue;
            };
            let param_list = decl
                .params
                .iter()
                .enumerate()
                .map(|(index, param)| {
                    let ty = signature
                        .params
                        .get(index)
                        .map(|sig| sig.ty.clone())
                        .unwrap_or(Ty::Error);
                    (param.name.clone(), param.direction, ty)
                })
                .collect::<Vec<_>>();
            bodies.insert(decl.name.clone(), &decl.body);
            local_types.insert(
                decl.name.clone(),
                signature
                    .params
                    .iter()
                    .map(|param| (param.name.clone(), param.ty.clone()))
                    .collect(),
            );
            params.insert(decl.name.clone(), param_list);
            spans.push((decl.name.clone(), decl.span));
        }

        let mut names: Vec<String> = params.keys().cloned().collect();
        names.sort();
        let mut summaries: HashMap<String, FlowSummary> = params
            .iter()
            .map(|(name, list)| (name.clone(), FlowSummary::with_params(list.len())))
            .collect();

        loop {
            let mut changed = false;
            for name in &names {
                let param_list = params.get(name).expect("function parameters");
                let signature = self.functions.get(name);
                let borrowed_params: HashMap<String, usize> = param_list
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, direction, _))| *direction == Direction::In)
                    .map(|(index, (param, _, _))| (param.clone(), index))
                    .collect();
                let out_params: HashSet<String> = param_list
                    .iter()
                    .filter(|(_, direction, _)| *direction == Direction::Out)
                    .map(|(param, _, _)| param.clone())
                    .collect();
                let mut scan = FunctionScan {
                    returns_composite: signature
                        .map(|sig| is_composite(&sig.return_type))
                        .unwrap_or(false),
                    read_params: borrowed_params.clone(),
                    direct_reads: vec![HashSet::new(); param_list.len()],
                    ..FunctionScan::default()
                };
                let mut locals: HashMap<String, Ty> = local_types
                    .get(name)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                locals.extend(globals.clone());
                let body = bodies.get(name).expect("function body");
                {
                    let ctx = FlowScanCtx {
                        params: &borrowed_params,
                        out_params: &out_params,
                        summaries: &summaries,
                    };
                    scan.view_returned =
                        scan_block_effects(self, &ctx, body, 0, &mut scan, &mut locals);
                }

                let param_names: Vec<String> = param_list
                    .iter()
                    .map(|(param, _, _)| param.clone())
                    .collect();
                let mut next = summaries
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| FlowSummary::with_params(param_list.len()));

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
                    next.record_param_effect(
                        index,
                        FlowEffect::Move(path.clone().with_root(param_names[index].clone())),
                    );
                }

                let external = signature.is_some_and(|sig| sig.external);
                if external {
                    // An extern body is empty by construction; silence is not
                    // evidence of no effects. Flag the uncertainty and keep a
                    // conservative whole-value effect for every direction.
                    next.unknown_effects = true;
                    for (index, (_, direction, _)) in param_list.iter().enumerate() {
                        let whole = DataPath::new(param_names[index].clone());
                        let effect = match direction {
                            Direction::In => FlowEffect::Read(whole),
                            Direction::Out => FlowEffect::Init(whole),
                            Direction::Inout => FlowEffect::Write(whole),
                        };
                        next.record_param_effect(index, effect);
                    }
                }
                for (index, direct) in scan.direct_reads.iter().enumerate() {
                    for relative in direct {
                        next.record_param_effect(
                            index,
                            FlowEffect::Read(
                                relative.clone().with_root(param_names[index].clone()),
                            ),
                        );
                    }
                }

                for (root, path) in &scan.written_places {
                    let Some((index, (_, direction, _))) = param_list
                        .iter()
                        .enumerate()
                        .find(|(_, (param, _, _))| param == root)
                    else {
                        continue;
                    };
                    if *direction == Direction::In {
                        continue;
                    }
                    next.record_param_effect(
                        index,
                        FlowEffect::Write(path.clone().with_root(param_names[index].clone())),
                    );
                }
                for (root, path) in &scan.initialized_places {
                    let Some((index, (_, direction, _))) = param_list
                        .iter()
                        .enumerate()
                        .find(|(_, (param, _, _))| param == root)
                    else {
                        continue;
                    };
                    if *direction == Direction::In {
                        continue;
                    }
                    let place = path.clone().with_root(param_names[index].clone());
                    let effect = if *direction == Direction::Out {
                        FlowEffect::Init(place)
                    } else {
                        // An `alloc` into an existing `<->` place overwrites it.
                        FlowEffect::Write(place)
                    };
                    next.record_param_effect(index, effect);
                }
                if scan.produces_result {
                    next.result.produces = true;
                }
                for (root, path) in &scan.produced_places {
                    next.record_result_effect(FlowEffect::Produce(
                        path.clone().with_root(root.clone()),
                    ));
                }

                if scan.direct_alloc {
                    next.allocations = true;
                    next.unguarded_allocations = true;
                }
                if scan.clone_direct {
                    next.allocations = true;
                    next.clone_allocations = true;
                }
                for callee in &scan.unguarded_deps {
                    if summaries
                        .get(callee)
                        .is_some_and(|summary| summary.unguarded_allocations)
                    {
                        next.allocations = true;
                        next.unguarded_allocations = true;
                    }
                }
                for callee in &scan.calls {
                    if summaries
                        .get(callee)
                        .is_some_and(|summary| summary.clone_allocations)
                    {
                        next.allocations = true;
                        next.clone_allocations = true;
                    }
                }

                for site in &scan.call_args {
                    let Some(callee_summary) = summaries.get(&site.callee) else {
                        continue;
                    };
                    let Some(callee_signature) = self.functions.get(&site.callee) else {
                        continue;
                    };
                    for (index, arg) in site.args.iter().enumerate() {
                        let callee_moves: Vec<DataPath> = callee_summary
                            .params
                            .get(index)
                            .map(|param| param.moves.iter().cloned().collect())
                            .unwrap_or_default();
                        if callee_moves.is_empty() {
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
                        for callee_path in callee_moves {
                            let base = arg_path.clone().with_root(param_names[position].clone());
                            next.record_param_effect(
                                position,
                                FlowEffect::Move(base.append_relative(&callee_path)),
                            );
                        }
                    }
                }

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
                        let effective: Vec<DataPath> = summaries
                            .get(&site.callee)
                            .and_then(|summary| summary.params.get(index))
                            .map(|param| param.reads.iter().cloned().collect())
                            .unwrap_or_else(|| vec![DataPath::new(String::new())]);
                        for callee_path in effective {
                            let base = arg_path
                                .clone()
                                .with_root(param_names[caller_index].clone());
                            next.record_param_effect(
                                caller_index,
                                FlowEffect::Read(base.append_relative(&callee_path)),
                            );
                        }
                    }
                }

                for site in &scan.call_args {
                    let Some(callee_summary) = summaries.get(&site.callee) else {
                        // Intrinsics are known compiler operations; any other
                        // name without a summary is genuinely unknown.
                        if !is_intrinsic_name(&site.callee) {
                            next.unknown_effects = true;
                            for arg in &site.args {
                                if !matches!(arg.0, Direction::Out | Direction::Inout) {
                                    continue;
                                }
                                let Some((arg_root, _)) = &arg.1 else {
                                    continue;
                                };
                                let Some((caller_index, (_, caller_direction, _))) = param_list
                                    .iter()
                                    .enumerate()
                                    .find(|(_, (param, _, _))| param == arg_root)
                                else {
                                    continue;
                                };
                                if *caller_direction == Direction::In {
                                    continue;
                                }
                                let whole = DataPath::new(param_names[caller_index].clone());
                                let effect = if arg.0 == Direction::Out {
                                    FlowEffect::Init(whole)
                                } else {
                                    FlowEffect::Write(whole)
                                };
                                next.record_param_effect(caller_index, effect);
                            }
                        }
                        continue;
                    };
                    let Some(callee_signature) = self.functions.get(&site.callee) else {
                        continue;
                    };
                    for (index, arg) in site.args.iter().enumerate() {
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
                        if *caller_direction == Direction::In && arg.0 != Direction::In {
                            continue;
                        }
                        let Some(callee_param) = callee_summary.params.get(index) else {
                            continue;
                        };
                        let callee_direction = callee_signature
                            .params
                            .get(index)
                            .map(|param| param.direction)
                            .unwrap_or(Direction::In);
                        let paths = |set: &crate::flow::DataPathSet| -> Vec<DataPath> {
                            set.iter().cloned().collect()
                        };
                        let mut effects: Vec<FlowEffect> = Vec::new();
                        if *caller_direction != Direction::In
                            && !matches!(callee_direction, Direction::In)
                        {
                            for path in paths(&callee_param.writes) {
                                effects.push(FlowEffect::Write(path));
                            }
                            for path in paths(&callee_param.inits) {
                                let effect = if *caller_direction == Direction::Out {
                                    FlowEffect::Init(path)
                                } else {
                                    FlowEffect::Write(path)
                                };
                                effects.push(effect);
                            }
                        }
                        for path in paths(&callee_param.escapes) {
                            effects.push(FlowEffect::Escape(path));
                        }
                        for borrow in &callee_param.borrows {
                            if borrow.escaped
                                && let Some(path) = borrow.source_path()
                            {
                                effects.push(FlowEffect::Escape(path.clone()));
                            }
                        }
                        for effect in effects {
                            for path in effect.paths() {
                                let base = arg_path
                                    .clone()
                                    .with_root(param_names[caller_index].clone());
                                let mapped_path = base.append_relative(path);
                                let mapped = match &effect {
                                    FlowEffect::Write(_) => FlowEffect::Write(mapped_path),
                                    FlowEffect::Init(_) => FlowEffect::Init(mapped_path),
                                    FlowEffect::Escape(_) => FlowEffect::Escape(mapped_path),
                                    _ => unreachable!("only mappable effects are queued"),
                                };
                                next.record_param_effect(caller_index, mapped);
                            }
                        }
                    }
                    if callee_summary.unknown_effects {
                        next.unknown_effects = true;
                    }
                    if callee_summary.result.produces {
                        next.result.produces = true;
                    }
                    // A produced path names the callee's `->` destination (or
                    // "result"). Re-root it on the caller argument so the
                    // durable contract describes the place this flow writes.
                    for produced in callee_summary.result.paths.iter() {
                        let mapped = if produced.root == "result" {
                            Some(produced.clone())
                        } else {
                            let Some(arg) = callee_signature
                                .params
                                .iter()
                                .position(|param| param.name == produced.root)
                                .and_then(|index| site.args.get(index))
                            else {
                                continue;
                            };
                            if !matches!(arg.0, Direction::Out | Direction::Inout) {
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
                            if *caller_direction == Direction::In {
                                continue;
                            }
                            let base = arg_path
                                .clone()
                                .with_root(param_names[caller_index].clone());
                            Some(base.append_relative(produced))
                        };
                        if let Some(mapped) = mapped {
                            next.record_result_effect(FlowEffect::Produce(mapped));
                        }
                    }
                }

                if signature.is_some_and(|sig| self.type_contains_slice(&sig.return_type)) {
                    for source in &scan.view_returned {
                        let source_name =
                            param_names.get(source.index).cloned().unwrap_or_default();
                        let source_path = source.data_path(&source_name);
                        next.record_param_effect(
                            source.index,
                            FlowEffect::Escape(source_path.clone()),
                        );
                        next.record_param_effect(
                            source.index,
                            FlowEffect::Borrow(Borrow {
                                source: LoanSource::Param {
                                    index: source.index,
                                    path: source_path,
                                },
                                holder: DataPath::new("result"),
                                destination: LoanDestination::Result,
                                mode: BorrowMode::Shared,
                                escaped: true,
                            }),
                        );
                    }
                }
                for (holder_name, loans) in &scan.views.out {
                    let Some((destination, _)) = param_list
                        .iter()
                        .enumerate()
                        .find(|(_, (param, _, _))| param == holder_name)
                    else {
                        continue;
                    };
                    if !signature
                        .and_then(|sig| sig.params.get(destination))
                        .is_some_and(|param| self.type_contains_slice(&param.ty))
                    {
                        continue;
                    }
                    for loan in loans {
                        let source_name = param_names
                            .get(loan.source.index)
                            .cloned()
                            .unwrap_or_default();
                        let source_path = loan.source.data_path(&source_name);
                        next.record_param_effect(
                            loan.source.index,
                            FlowEffect::Escape(source_path.clone()),
                        );
                        next.record_param_effect(
                            loan.source.index,
                            FlowEffect::Borrow(Borrow {
                                source: LoanSource::Param {
                                    index: loan.source.index,
                                    path: source_path,
                                },
                                holder: loan.slot.clone().with_root(holder_name.clone()),
                                destination: LoanDestination::OutParam(destination),
                                mode: BorrowMode::Shared,
                                escaped: true,
                            }),
                        );
                    }
                }

                if next != *summaries.get(name).expect("existing summary") {
                    changed = true;
                    *summaries.get_mut(name).expect("existing summary") = next;
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
            if name == "main"
                && summaries
                    .get(name)
                    .is_some_and(|summary| summary.unguarded_allocations)
            {
                self.error(
                    *span,
                    format!("function '{name}' can allocate outside a 'protect' block"),
                );
            }
        }

        self.flow_summaries = summaries;
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
                views: Vec::new(),
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
        self.guards.clear();
    }

    // ------------------------------------------------------------------
    // Linear resources and `guard` blocks
    // ------------------------------------------------------------------

    /// Resolve `ty`'s convention-named release flow and validate its signature.
    /// Emits a diagnostic and returns `None` when `ty` is not a linear resource.
    fn checked_release_flow(&mut self, ty: &Ty, span: Span) -> Option<String> {
        let Ty::Named(shape) = ty else {
            self.error(span, "guard requires a named shape with a release flow");
            return None;
        };
        let release = types::release_flow_name(ty).expect("named shape has a release name");
        let Some(sig) = self.functions.get(&release) else {
            self.error(
                span,
                format!("shape '{shape}' has no resource release flow '{release}'"),
            );
            return None;
        };
        let valid = sig.params.len() == 1
            && sig.params[0].direction == Direction::Inout
            && &sig.params[0].ty == ty
            && sig.return_type == Ty::Void;
        if !valid {
            self.error(
                span,
                format!(
                    "invalid resource release flow '{release}' for shape '{shape}'; expected flow {release}(<-> value: {shape}) with no return"
                ),
            );
            return None;
        }
        Some(release)
    }

    /// True when `name` is an active `guard` binding and cannot be moved out.
    fn is_active_guard(&self, name: &str) -> bool {
        self.guards.iter().any(|guard| guard.name == name)
    }

    /// Find a guarded resource passed to its own release flow, which would be
    /// a double release because `guard` inserts that call automatically.
    fn guarded_release_target(&self, callee: &str, args: &[CallArg]) -> Option<String> {
        self.guards.iter().find_map(|guard| {
            if guard.release != callee {
                return None;
            }
            args.iter()
                .find_map(|arg| place_root(&arg.value))
                .filter(|root| root == &guard.name)
        })
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
                        views: Vec::new(),
                        is_global: false,
                    },
                );
                if carries_storage {
                    self.set_variable_regions(name, &value_regions);
                }
                if let Some(value) = value.as_ref() {
                    let loans = self.value_views(value);
                    if !loans.is_empty() {
                        self.set_variable_views(name, &loans);
                    }
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
                let exit_start = self.loop_exit_states.len();
                self.loop_depth += 1;
                let before = self.snapshot();
                let body_reachable = self.check_block(body);
                self.loop_depth -= 1;
                self.join_loop_exits(exit_start);
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
                let exit_start = self.loop_exit_states.len();
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
                            views: Vec::new(),
                            is_global: false,
                        },
                    );
                }
                self.check_statements(&body.statements);
                self.end_region(body_region);
                self.scopes.pop();
                self.loop_depth -= 1;
                self.join_loop_exits(exit_start);
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
                let exit_start = self.loop_exit_states.len();
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
                            views: Vec::new(),
                            is_global: false,
                        },
                    );
                }
                self.check_statements(&body.statements);
                self.end_region(body_region);
                self.scopes.pop();
                self.loop_depth -= 1;
                self.join_loop_exits(exit_start);
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
            StmtKind::Guard {
                name,
                ty,
                value,
                body,
            } => {
                let entry_reachable = self.reachable;
                let value_ty = self.check_expr(value);
                let value_regions = self.value_regions(value);
                let resolved = match ty.as_ref() {
                    Some(ty) => {
                        let declared = self.resolve_type(ty);
                        if !self.assignable_expr(&declared, &value_ty, value) {
                            self.error(
                                ty.span(),
                                format!(
                                    "cannot initialize guarded resource '{}' of type {} with {}",
                                    name, declared, value_ty
                                ),
                            );
                        }
                        declared
                    }
                    None => value_ty.concretize(),
                };
                let release = self.checked_release_flow(&resolved, statement.span);

                // The binding lives in its own scope so it disappears when the
                // guard ends; `check_block` adds a nested scope for the body.
                self.scopes.push(HashMap::new());
                self.declare_variable(
                    name,
                    VarInfo {
                        ty: resolved.clone(),
                        mutable: false,
                        initialized: true,
                        direction: None,
                        span: statement.span,
                        moved: HashMap::new(),
                        home_region: self.current_region(),
                        taints: Vec::new(),
                        views: Vec::new(),
                        is_global: false,
                    },
                );
                if self.type_contains_slice(&resolved) {
                    self.set_variable_regions(name, &value_regions);
                }
                let loans = self.value_views(value);
                if !loans.is_empty() {
                    self.set_variable_views(name, &loans);
                }
                if let Some(release) = release.clone() {
                    self.guards.push(GuardCleanup {
                        name: name.clone(),
                        release,
                    });
                }
                let block_reachable = self.check_block(body);
                if release.is_some() {
                    self.guards.pop();
                }
                self.scopes.pop();
                self.reachable = entry_reachable && block_reachable;
            }
            StmtKind::Return(value) => {
                self.check_return(value, statement.span);
                self.reachable = false;
            }
            StmtKind::Continue => {
                if self.loop_depth == 0 {
                    self.error(statement.span, "'continue' is only allowed inside a loop");
                } else {
                    self.loop_exit_states.push(self.snapshot());
                }
                self.reachable = false;
            }
            StmtKind::Break => {
                if self.loop_depth == 0 {
                    self.error(statement.span, "'break' is only allowed inside a loop");
                } else {
                    self.loop_exit_states.push(self.snapshot());
                }
                self.reachable = false;
            }
            StmtKind::Splice(_) => {
                self.error(
                    statement.span,
                    "'splice' expects a comptime expression of type 'Code'",
                );
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
                    if self.is_active_guard(&root) {
                        self.error(
                            value.span,
                            format!("cannot return guarded resource '{root}'"),
                        );
                    } else if from_in && matches!(actual, Ty::Slice(_)) {
                        // A slice is a view over its storage. Returning one that
                        // points into a `<-` parameter keeps the caller's storage
                        // alive for the result's lexical lifetime; the caller
                        // inherits the loan at the call site.
                        self.add_loan(&root, &path, value.span);
                    } else if from_in {
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
                let loans = self.value_views(value);
                self.set_place_views(&info, &loans);
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
        // The immediate `fmt::(...)` directive is only legal as the root call
        // of an output argument. Any wrapper expression consumes that slot.
        if !matches!(expr.kind, ExprKind::Call { .. }) {
            self.fmt_direct_arg = false;
        }
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
            // Placement was validated during expansion. The nodes remain in
            // comptime flow bodies as data, so the checker only needs types.
            ExprKind::Quote(_) => Ty::Code,
            ExprKind::Unquote(inner) => self.check_expr(inner),
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
                // Equality is structural: two values of the same type are
                // equal when every field, payload or element is equal. The
                // backend emits a type-directed helper for composites; only
                // scalars can use C's native `==`.
                let left_caps = types::comparison_caps(left, self);
                let right_caps = types::comparison_caps(right, self);
                let numeric_pair = left.is_numeric()
                    && right.is_numeric()
                    && types::common_numeric(left, right).is_some();
                let comparable = (left == right && left_caps.eq && right_caps.eq) || numeric_pair;
                if !comparable {
                    self.error(
                        span,
                        format!(
                            "cannot compare {left} with {right}; both operands must have the same comparable type"
                        ),
                    );
                }
                Ty::Bool
            }
            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                if left.is_error() || right.is_error() {
                    return Ty::Bool;
                }
                // Ordering is total for numbers, bool, marks, cstr, shapes,
                // choices and slices. Opaque handles and callbacks are not
                // orderable; composites use a lexicographic helper.
                let left_caps = types::comparison_caps(left, self);
                let right_caps = types::comparison_caps(right, self);
                let numeric_pair = left.is_numeric()
                    && right.is_numeric()
                    && types::common_numeric(left, right).is_some();
                let orderable = (left == right && left_caps.ord && right_caps.ord) || numeric_pair;
                if !orderable {
                    self.error(
                        span,
                        format!(
                            "cannot order {left} with {right}; both operands must have the same orderable type"
                        ),
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
        // Only the outermost call of an output argument may be `fmt::(...)`.
        // Clear the slot before any nested call is checked.
        let direct_fmt = self.fmt_direct_arg;
        self.fmt_direct_arg = false;

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

        // `view(xs, start, end)` is a zero-copy subslice intrinsic. It has no
        // fixed element type, so it is checked here instead of being registered
        // with a concrete signature in `collect_names`.
        if name == "view" {
            return self.check_view_call(args, span);
        }

        // `callback(flow)` builds a C callback value from a Loi flow. The
        // signature is inferred from the flow and checked at each use site.
        if name == "callback" {
            return self.check_callback_call(args, span);
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
        // `fmt::(...)` is the immediate spelling of `format(...)`, allowed only
        // where the produced bytes are consumed at once by an output intrinsic.
        if name == "fmt::" {
            if !direct_fmt {
                self.error(
                    callee.span,
                    "fmt::(...) must be used directly inside print, eprint or panic",
                );
            }
            return self.check_format_call(args, span);
        }
        if name == "format" {
            return self.check_format_call(args, span);
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

        // A guarded resource's release flow is inserted on every exit path by
        // `guard`; calling it explicitly would release the resource twice.
        if let Some(resource) = self.guarded_release_target(name, args) {
            self.error(
                span,
                format!(
                    "cannot call '{name}' on guarded resource '{resource}'; it is released automatically by 'guard'"
                ),
            );
        }

        let variadic_index = signature.params.iter().position(|param| param.variadic);
        let required = variadic_index.unwrap_or(signature.params.len());
        if variadic_index.is_some() {
            if args.len() < required {
                self.error(
                    span,
                    format!(
                        "function '{}' expects at least {} argument(s), found {}",
                        name,
                        required,
                        args.len()
                    ),
                );
            }
        } else if args.len() != signature.params.len() {
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

        let may_allocate = self
            .summary(name)
            .is_some_and(|summary| summary.unguarded_allocations);
        let mut out_targets = Vec::new();
        let output_arg = matches!(name.as_str(), "print" | "eprint" | "panic");
        // A variadic parameter is seen by the body as `[]T`, but each call-site
        // argument is checked against `T` so literals get the right range and
        // coercion diagnostics.
        let vararg_param = variadic_index.map(|index| {
            let mut param = signature.params[index].clone();
            if let Ty::Slice(element) = &param.ty {
                param.ty = (**element).clone();
            }
            param.variadic = false;
            param
        });
        for (index, arg) in args.iter().enumerate() {
            if let Some(param) = vararg_param.as_ref()
                && index >= required
            {
                self.fmt_direct_arg = output_arg;
                let info = self.check_call_arg(arg, param, may_allocate, name, index);
                self.fmt_direct_arg = false;
                if let Some(info) = info {
                    out_targets.push((index, info));
                }
                continue;
            }
            match signature.params.get(index) {
                Some(param) => {
                    self.fmt_direct_arg = output_arg;
                    let info = self.check_call_arg(arg, param, may_allocate, name, index);
                    self.fmt_direct_arg = false;
                    if let Some(info) = info {
                        out_targets.push((index, info));
                    }
                }
                None => self.check_unmatched_arg(arg),
            }
        }

        // Ownership summaries are indexed by declared parameter. Trailing
        // arguments have no declared slot, so the region passes only see the
        // fixed prefix.
        let fixed_signature = match variadic_index {
            Some(index) => FunctionSig {
                params: signature.params[..index].to_vec(),
                ..signature.clone()
            },
            None => signature.clone(),
        };
        self.check_call_aliasing(name, args, &fixed_signature);
        self.apply_parameter_consumption(name, args, &fixed_signature);
        self.propagate_call_regions(name, args, &fixed_signature, &out_targets);
        self.propagate_call_views(name, args, &out_targets);

        signature.return_type
    }

    /// Check the `view` intrinsic: `view(xs, start, end)` returns a zero-copy
    /// subslice of `xs`. The caller's storage stays borrowed for as long as the
    /// resulting value is live; no allocation or element copy takes place.
    fn check_view_call(&mut self, args: &[CallArg], span: Span) -> Ty {
        if args.len() != 3 {
            self.error(
                span,
                format!("view expects 3 argument(s), found {}", args.len()),
            );
            for arg in args {
                self.check_unmatched_arg(arg);
            }
            return Ty::Error;
        }
        for arg in args {
            if arg.direction != Direction::In {
                self.error(arg.value.span, "view expects a '<-' argument");
            }
        }
        let source = self.check_expr(&args[0].value);
        let start = self.check_expr(&args[1].value);
        let end = self.check_expr(&args[2].value);
        let Ty::Slice(element) = source else {
            if !source.is_error() {
                self.error(
                    args[0].value.span,
                    format!("view expects a slice, found {source}"),
                );
            }
            return Ty::Error;
        };
        for (arg, ty) in [(&args[1], &start), (&args[2], &end)] {
            if !ty.is_integer() && !ty.is_error() {
                self.error(
                    arg.value.span,
                    format!("view expects an integer index, found {ty}"),
                );
            }
            self.check_literal_range(&arg.value, &Ty::U64);
        }
        Ty::Slice(element)
    }

    /// Check `callback(flow)`. The value is a C function pointer whose signature
    /// comes from the target flow. Only non-`extern` Loi flows with all `<-`
    /// parameters can be used; the flow symbol itself is passed to C, so no
    /// thunk is generated.
    fn check_callback_call(&mut self, args: &[CallArg], span: Span) -> Ty {
        if args.len() != 1 {
            self.error(
                span,
                format!("callback expects 1 argument, found {}", args.len()),
            );
            for arg in args {
                self.check_unmatched_arg(arg);
            }
            return Ty::Error;
        }
        let arg = &args[0];
        if arg.direction != Direction::In {
            self.error(arg.value.span, "callback expects a '<-' argument");
        }
        let ExprKind::Ident(flow) = &arg.value.kind else {
            self.error(arg.value.span, "callback expects a flow name");
            return Ty::Error;
        };
        let Some(signature) = self.functions.get(flow).cloned() else {
            self.error(
                arg.value.span,
                format!("unknown flow '{flow}' for callback"),
            );
            return Ty::Error;
        };
        if signature.params.iter().any(|param| param.variadic) {
            self.error(
                arg.value.span,
                format!("variadic flow '{flow}' cannot be used as a callback"),
            );
            return Ty::Error;
        }
        let callback_sig = CallbackSig {
            params: signature
                .params
                .iter()
                .map(|param| param.ty.clone())
                .collect(),
            result: Box::new(signature.return_type.clone()),
        };
        let fixed_shape = signature.params.len() == 2
            && signature.params.iter().all(|param| param.ty == Ty::I32)
            && signature.return_type == Ty::I32;
        if signature.external {
            let message = if fixed_shape {
                format!(
                    "extern flow '{flow}' cannot be used as a callback; declare a Loi flow with signature flow {flow}(<- a: i32, <- b: i32) => i32"
                )
            } else {
                format!(
                    "extern flow '{flow}' cannot be used as a callback; declare a Loi flow with signature {}",
                    callback_sig.display()
                )
            };
            self.error(arg.value.span, message);
            return Ty::Error;
        }
        let all_input = signature
            .params
            .iter()
            .all(|param| param.direction == Direction::In);
        if !all_input {
            let message = if fixed_shape {
                format!(
                    "callback flow '{flow}' must have signature flow {flow}(<- a: i32, <- b: i32) => i32"
                )
            } else {
                format!(
                    "callback flow '{flow}' must have signature {}",
                    callback_sig.display()
                )
            };
            self.error(arg.value.span, message);
            return Ty::Error;
        }
        Ty::Callback(callback_sig)
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

    /// Check the `format` intrinsic: `format("...{}...", args...)` returns a
    /// `[]u8` allocated in the caller's `protect` region.
    fn check_format_call(&mut self, args: &[CallArg], span: Span) -> Ty {
        let Some(fmt) = args.first() else {
            self.error(span, "format expects a template string argument");
            return Ty::Slice(Box::new(Ty::U8));
        };
        if fmt.direction != Direction::In {
            self.error(fmt.value.span, "format expects a '<-' template string");
        }

        let ExprKind::String(text) = &fmt.value.kind else {
            self.error(fmt.value.span, "format template must be a string literal");
            self.check_expr(&fmt.value);
            for arg in &args[1..] {
                self.check_unmatched_arg(arg);
            }
            return Ty::Slice(Box::new(Ty::U8));
        };

        let pieces = match crate::format::parse_braced_format(text) {
            Ok(pieces) => pieces,
            Err(message) => {
                self.error(fmt.value.span, format!("invalid format string: {message}"));
                for arg in &args[1..] {
                    self.check_unmatched_arg(arg);
                }
                return Ty::Slice(Box::new(Ty::U8));
            }
        };
        let placeholders = pieces
            .iter()
            .filter(|piece| matches!(piece, crate::format::BracedPiece::Placeholder))
            .count();

        if placeholders != args.len() - 1 {
            self.error(
                span,
                format!(
                    "format template expects {} argument(s), found {}",
                    placeholders,
                    args.len() - 1
                ),
            );
        }

        for arg in &args[1..] {
            if arg.direction != Direction::In {
                self.error(arg.value.span, "format expects '<-' arguments");
            }
            let ty = self.check_expr(&arg.value);
            let is_string_lit = matches!(arg.value.kind, ExprKind::String(_));
            if !ty.is_error() && !crate::format::is_braced_printable(&ty, is_string_lit) {
                self.error(
                    arg.value.span,
                    format!("format placeholder expects an integer, float, bool, string or cstr, found {ty}"),
                );
            }
        }

        Ty::Slice(Box::new(Ty::U8))
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
        let Some(summary) = self.summary(name) else {
            return;
        };
        let consumed: Vec<crate::flow::DataPathSet> = (0..signature.params.len())
            .map(|index| summary.move_paths(index).clone())
            .collect();

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

    /// Carry loaned views written to `->` parameters back to the caller's
    /// destination places, using the callee's precomputed view summary.
    fn propagate_call_views(
        &mut self,
        name: &str,
        args: &[CallArg],
        out_targets: &[(usize, LvalueInfo)],
    ) {
        let Some(out_effects) = self.summary(name).map(FlowSummary::view_out_data_paths) else {
            return;
        };
        for (index, info) in out_targets {
            let Some(sources) = out_effects.get(index) else {
                continue;
            };
            let mut loans = Vec::new();
            for (source, _) in sources {
                if let Some(arg) = args.get(*source) {
                    self.collect_view_source(&arg.value, &mut loans);
                }
            }
            if loans.is_empty() {
                continue;
            }
            let loans = Self::normalize_views(loans);
            self.set_place_views(info, &loans);
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
        let Some(summary) = self.summary(name) else {
            return;
        };
        let consumed: Vec<crate::flow::DataPathSet> = (0..signature.params.len())
            .map(|index| summary.move_paths(index).clone())
            .collect();
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
            let base = DataPath::from_relative(&root, &path);
            for relative in relative_paths.iter() {
                let moved = base.append_relative(relative);
                self.record_move(&root, &moved.relative_string(), arg.value.span);
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
        // `Name.Variant` is a value (a mark variant or a choice constructor
        // name), not an lvalue rooted at the type name. The checker's free
        // `is_place` cannot see the mark table, so filter it here instead of
        // misreporting "unknown variable 'Name'". The C11 backend applies the
        // equivalent guard in `codegen.rs::is_place`.
        if let ExprKind::Member { base, .. } = &value.kind
            && let ExprKind::Ident(name) = &base.kind
            && (self.marks.contains_key(name) || self.choices.contains_key(name))
        {
            return self.check_expr(value);
        }
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
            .summary(callee)
            .map(|summary| summary.read_paths_typed(param_index).clone())
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
        let base = DataPath::from_relative(&root, &path);
        for read in reads.iter() {
            let effective = base.append_relative(read).relative_string();
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
                ExprKind::Ident(name) => {
                    if matches!(name.as_str(), "format" | "fmt::") {
                        Ty::Slice(Box::new(Ty::U8))
                    } else if name == "printf" {
                        Ty::Void
                    } else {
                        self.functions
                            .get(name)
                            .map(|signature| signature.return_type.clone())
                            .unwrap_or(Ty::Error)
                    }
                }
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
            ExprKind::Quote(_) => Ty::Code,
            ExprKind::Unquote(_) => Ty::Error,
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
                .any(|moved| !moved.is_whole() && path_is_ancestor(&path, moved));

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

        if self.check_view_source(&name, &path, span) {
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
        !self.check_view_source(&name, &path, span)
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
                let path = relative_data_path(path);
                info.taints.retain(|taint| !taint.path.may_overlap(&path));
                return;
            }
        }
    }

    fn record_move(&mut self, name: &str, path: &str, span: Span) {
        if self.check_view_source(name, path, span) {
            return;
        }
        if self.is_active_guard(name) {
            self.error(
                span,
                format!(
                    "cannot move guarded resource '{name}' out of its guard; it is released automatically by 'guard'"
                ),
            );
            return;
        }
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                if info.is_global {
                    return;
                }
                info.moved.retain(|moved, _| !path_is_ancestor(path, moved));
                info.moved.insert(relative_data_path(path), span);
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
                        let path = relative_data_path(&path);
                        for taint in &var.taints {
                            if taint.path.may_overlap(&path) {
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
                    && name == "view"
                    && let Some(source) = args.first()
                {
                    self.collect_value_regions(&source.value, regions);
                }
                if let ExprKind::Ident(name) = &callee.kind
                    && let Some(returns) =
                        self.summary(name).map(FlowSummary::view_returns_data_paths)
                    && matches!(self.functions.get(name), Some(sig) if matches!(sig.return_type, Ty::Slice(_)))
                {
                    for (index, _) in returns {
                        if let Some(arg) = args.get(index) {
                            self.collect_value_regions(&arg.value, regions);
                        }
                    }
                }
                if let ExprKind::Ident(name) = &callee.kind
                    && matches!(
                        name.as_str(),
                        "read_file" | "read_stdin" | "read_line" | "format" | "fmt::"
                    )
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
                    && self
                        .summary(name)
                        .is_some_and(|summary| summary.clone_allocations)
                    && let Some(region) = self.current_protect_region()
                {
                    regions.push(region);
                }
                if let ExprKind::Ident(name) = &callee.kind
                    && let Some(signature) = self.functions.get(name)
                    && self.type_contains_slice(&signature.return_type)
                    && let Some(consumed) = self.summary(name).map(|summary| {
                        (0..signature.params.len())
                            .map(|index| summary.move_paths(index).clone())
                            .collect::<Vec<_>>()
                    })
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
            ExprKind::Quote(_) | ExprKind::Unquote(_) => {}
        }
    }

    // ------------------------------------------------------------------
    // slice views and their lexical loans
    // ------------------------------------------------------------------

    /// Add a loan on the storage reached by `source` to every scope until the
    /// root variable is found. Used when a view escapes through a `return`.
    fn add_loan(&mut self, source: &str, path: &str, span: Span) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(source)
                && !info.views.iter().any(|loan| {
                    loan.source == source
                        && loan.path == relative_data_path(path)
                        && loan.slot.is_whole()
                        && loan.span == span
                })
            {
                info.views.push(ViewLoan {
                    source: source.to_string(),
                    path: relative_data_path(path),
                    slot: DataPath::default(),
                    span,
                });
                return;
            }
        }
    }

    fn normalize_views(mut loans: Vec<ViewLoan>) -> Vec<ViewLoan> {
        loans.sort_by_key(|loan| {
            (
                loan.source.clone(),
                loan.path.clone(),
                loan.slot.clone(),
                loan.span.line,
                loan.span.column,
            )
        });
        loans.dedup_by(|left, right| {
            left.source == right.source
                && left.path == right.path
                && left.slot == right.slot
                && left.span == right.span
        });
        loans
    }

    /// Loans carried by a value. A `view(...)` call borrows its first argument;
    /// a flow that returns a borrowed view borrows the matching arguments.
    fn value_views(&self, expr: &Expr) -> Vec<ViewLoan> {
        let mut loans = Vec::new();
        self.collect_value_views(expr, &mut loans);
        Self::normalize_views(loans)
    }

    fn collect_value_views(&self, expr: &Expr, loans: &mut Vec<ViewLoan>) {
        match &expr.kind {
            ExprKind::Ident(name) => {
                if let Some(var) = self.lookup(name) {
                    loans.extend(var.views.iter().cloned());
                }
            }
            ExprKind::Member { .. } | ExprKind::Index { .. } => {
                if let (Some(root), Some(path)) = (place_root(expr), place_path(expr)) {
                    if let Some(var) = self.lookup(&root) {
                        for loan in &var.views {
                            // `path` selects a slot in the holder, not in the
                            // borrowed source. Normalize the slot to the value
                            // selected by this expression.
                            if let Some(slot) = path_suffix(&loan.slot, &path) {
                                let mut loan = loan.clone();
                                loan.slot = slot;
                                loans.push(loan);
                            }
                        }
                    }
                } else if let ExprKind::Member { base, .. } = &expr.kind {
                    self.collect_value_views(base, loans);
                } else if let ExprKind::Index { base, index } = &expr.kind {
                    self.collect_value_views(base, loans);
                    self.collect_value_views(index, loans);
                }
            }
            ExprKind::Call { callee, args, .. } => {
                let ExprKind::Ident(name) = &callee.kind else {
                    return;
                };
                if name == "view" {
                    if let Some(source) = args.first() {
                        self.collect_view_source(&source.value, loans);
                    }
                    return;
                }
                if let Some(returns) = self.summary(name).map(FlowSummary::view_returns_data_paths)
                {
                    for (index, _) in returns {
                        if let Some(arg) = args.get(index) {
                            self.collect_view_source(&arg.value, loans);
                        }
                    }
                }
            }
            ExprKind::Unary { operand, .. }
            | ExprKind::Cast { expr: operand, .. }
            | ExprKind::Comptime(operand) => self.collect_value_views(operand, loans),
            ExprKind::Binary { left, right, .. } => {
                self.collect_value_views(left, loans);
                self.collect_value_views(right, loans);
            }
            ExprKind::InterfaceLiteral { fields, .. } => {
                for field in fields {
                    self.collect_value_views(&field.value, loans);
                }
            }
            ExprKind::SliceLiteral { elements, .. } => {
                for element in elements {
                    self.collect_value_views(element, loans);
                }
            }
            ExprKind::Move(value) => self.collect_value_views(value, loans),
            // A clone owns fresh storage and never aliases the source.
            ExprKind::Copy(_) => {}
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::String(_)
            | ExprKind::Alloc { .. } => {}
            ExprKind::Quote(_) | ExprKind::Unquote(_) => {}
        }
    }

    /// Collect loans already carried by `expr` and record that its own storage
    /// is now borrowed as well.
    fn collect_view_source(&self, expr: &Expr, loans: &mut Vec<ViewLoan>) {
        self.collect_value_views(expr, loans);
        if let (Some(root), Some(path)) = (place_root(expr), place_path(expr))
            && self.lookup(&root).is_some()
        {
            loans.push(ViewLoan {
                source: root,
                path: relative_data_path(&path),
                slot: DataPath::default(),
                span: expr.span,
            });
        }
    }

    fn set_variable_views(&mut self, name: &str, loans: &[ViewLoan]) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.views = loans.to_vec();
                return;
            }
        }
    }

    fn set_place_views(&mut self, place: &LvalueInfo, loans: &[ViewLoan]) {
        let Some(root) = place.root.as_deref() else {
            return;
        };
        let Some(path) = place.path.as_deref() else {
            return;
        };
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(root) {
                // Only loans stored in the overwritten slot are released.
                // `loan.path` describes the *source*, so it must not be used
                // here: writing `s.b` does not kill a view stored in `s.a`.
                let place = relative_data_path(path);
                info.views.retain(|loan| !loan.slot.may_overlap(&place));
                let relocated = loans.iter().map(|loan| {
                    let mut loan = loan.clone();
                    loan.slot = place.append_relative(&loan.slot);
                    loan
                });
                info.views.extend(relocated);
                return;
            }
        }
    }

    /// Reject a write, move or mutable borrow while a live view aliases the
    /// same storage. Loans live on view variables, so they disappear with the
    /// scope that declared the view.
    fn check_view_source(&mut self, root: &str, path: &str, span: Span) -> bool {
        let mut conflict = None;
        'outer: for scope in &self.scopes {
            for info in scope.values() {
                let place = relative_data_path(path);
                for loan in &info.views {
                    if loan.source == root && loan.path.may_overlap(&place) {
                        conflict = Some(loan.clone());
                        break 'outer;
                    }
                }
            }
        }
        let Some(loan) = conflict else {
            return false;
        };
        let place = append_relative_path(root, path);
        let mut diagnostic = Diagnostic::from_error(
            format!("cannot write to '{place}': it is borrowed by a slice view"),
            span,
        )
        .with_label("write here");
        diagnostic = diagnostic.with_secondary(loan.span, "view created here");
        self.diagnostics.push(diagnostic);
        true
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
                let path = relative_data_path(path);
                info.taints.retain(|taint| !taint.path.may_overlap(&path));
                for region in regions {
                    info.taints.push(RegionTaint {
                        path: path.clone(),
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
                let path = relative_data_path(path);
                for region in regions {
                    if !info
                        .taints
                        .iter()
                        .any(|taint| taint.path == path && taint.region == *region)
                    {
                        info.taints.push(RegionTaint {
                            path: path.clone(),
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
                        path: DataPath::default(),
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
    fn move_consumes_place(&self, ty: &Ty, path: &DataPath) -> bool {
        if path.is_whole() {
            return true;
        }
        match self.type_at_path(ty, path) {
            Some(actual) => is_composite(&actual),
            None => true,
        }
    }

    /// Resolve the type of a typed root-relative ownership path.
    fn type_at_path(&self, ty: &Ty, path: &DataPath) -> Option<Ty> {
        let mut current = ty.clone();
        for segment in &path.segments {
            current = match segment {
                PathSegment::Field(field) => match &current {
                    Ty::Slice(_) if field == "len" => Ty::U64,
                    Ty::Named(name) => {
                        let info = self.interfaces.get(name)?;
                        let index = *info.field_index.get(field)?;
                        info.fields[index].ty.clone()
                    }
                    Ty::Choice(name) => self.choices.get(name)?.variants.get(field)?.ty.clone(),
                    _ => return None,
                },
                PathSegment::Index(_) | PathSegment::UnknownTail => match current {
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
                                views: info.views.clone(),
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
                    info.views = state.views.clone();
                }
            }
        }
    }

    /// Fold the `break`/`continue` states collected since `start` into the
    /// current control state. Loop statements call this before joining the
    /// pre-loop state, so a move on an early-exit path remains visible after
    /// the loop.
    fn join_loop_exits(&mut self, start: usize) {
        let exits: Vec<_> = self.loop_exit_states.drain(start..).collect();
        let mut acc = self.snapshot();
        for exit in exits {
            self.restore(&acc);
            let current = self.snapshot();
            self.merge_states(&current, &exit);
            acc = self.snapshot();
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
                let mut views = left_state.views.clone();
                for loan in &right_state.views {
                    if !views.iter().any(|existing| {
                        existing.source == loan.source
                            && existing.path == loan.path
                            && existing.slot == loan.slot
                            && existing.span == loan.span
                    }) {
                        views.push(loan.clone());
                    }
                }
                info.views = views;
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

/// Context for the single body visitor that produces `FlowSummary`.
struct FlowScanCtx<'a> {
    /// `<-` parameter name to its declaration index.
    params: &'a HashMap<String, usize>,
    /// Names of this function's `->` parameters.
    out_params: &'a HashSet<String>,
    /// Current fixpoint approximation of every function's durable summary.
    summaries: &'a HashMap<String, FlowSummary>,
}

impl FlowScanCtx<'_> {
    /// Sources carried by a name in the current scan iteration.
    fn sources(&self, name: &str, views: &ViewScanState) -> BTreeSet<ViewSource> {
        let mut borrowed = views.locals.get(name).cloned().unwrap_or_default();
        if let Some(loans) = views.out.get(name) {
            borrowed.extend(loans.iter().map(|loan| loan.source.clone()));
        }
        if let Some(index) = self.params.get(name) {
            borrowed.insert(ViewSource::whole(*index));
        }
        borrowed
    }

    /// Record that writing `target` may publish `sources` as views. The
    /// holder-relative slot is kept so a summary can name `out.a` instead of
    /// only the holder root.
    fn record_target(
        &self,
        target: &Expr,
        sources: BTreeSet<ViewSource>,
        state: &mut ViewScanState,
    ) {
        if sources.is_empty() {
            return;
        }
        let Some(root) = place_root(target) else {
            return;
        };
        let slot = place_path(target)
            .map(|path| relative_data_path(&path))
            .unwrap_or_default();
        if self.out_params.contains(&root) {
            let published = state.out.entry(root).or_default();
            for source in sources {
                published.insert(ViewOutLoan {
                    slot: slot.clone(),
                    source,
                });
            }
        } else if !self.params.contains_key(&root) {
            state.locals.entry(root).or_default().extend(sources);
        }
    }

    /// View sources published by one call, after its arguments were scanned.
    ///
    /// `arg_sources` is computed by the same expression visitor that records
    /// ownership, allocation and read effects, so no expression is traversed
    /// twice. Each source carries the path borrowed by the callee, composed
    /// with the path borrowed at this call site.
    fn call_sources(
        &self,
        name: &str,
        args: &[CallArg],
        arg_sources: &[BTreeSet<ViewSource>],
        state: &mut ViewScanState,
    ) -> BTreeSet<ViewSource> {
        if name == "view" {
            return arg_sources.first().cloned().unwrap_or_default();
        }
        let Some(summary) = self.summaries.get(name) else {
            return BTreeSet::new();
        };
        let mut borrowed = BTreeSet::new();
        for (index, path) in summary.view_returns_data_paths() {
            if let Some(sources) = arg_sources.get(index) {
                borrowed.extend(sources.iter().map(|source| source.extended_by(&path)));
            }
        }
        for (out_index, sources_by_path) in summary.view_out_data_paths() {
            let mut mapped = BTreeSet::new();
            for (index, path) in sources_by_path {
                if let Some(sources) = arg_sources.get(index) {
                    mapped.extend(sources.iter().map(|source| source.extended_by(&path)));
                }
            }
            if let Some(arg) = args.get(out_index) {
                self.record_target(&arg.value, mapped, state);
            }
        }
        borrowed
    }
}

/// Names owned by the compiler. They are the closed intrinsic set from
/// `LOI_SPEC.md` 5.9; user code may not redeclare them.
pub fn intrinsic_names() -> &'static [&'static str] {
    &[
        "print",
        "eprint",
        "panic",
        "sqrt",
        "floor",
        "ceil",
        "fabs",
        "pow",
        "fmod",
        "len",
        "view",
        "callback",
        "read_file",
        "write_file",
        "read_stdin",
        "read_line",
        "print_int",
        "print_float",
        "printf",
        "format",
        "fmt::",
    ]
}

pub(crate) fn is_intrinsic_name(name: &str) -> bool {
    intrinsic_names().contains(&name)
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::In => "<-",
        Direction::Out => "->",
        Direction::Inout => "<->",
    }
}

/// Scan a function body for allocation, ownership, read and view effects.
///
/// This is the single body visitor behind `FlowSummary`. `depth` counts
/// enclosing lexical `protect` blocks; allocations and calls at depth zero are
/// effects that a caller must cover with a `protect`. It returns the view
/// sources that may reach a `=>` result in this block.
fn scan_block_effects(
    checker: &Checker,
    ctx: &FlowScanCtx<'_>,
    block: &Block,
    depth: usize,
    scan: &mut FunctionScan,
    locals: &mut HashMap<String, Ty>,
) -> BTreeSet<ViewSource> {
    let mut returned = BTreeSet::new();
    for statement in &block.statements {
        returned.extend(scan_statement_effects(
            checker, ctx, statement, depth, scan, locals,
        ));
    }
    returned
}

fn scan_statement_effects(
    checker: &Checker,
    ctx: &FlowScanCtx<'_>,
    statement: &Stmt,
    depth: usize,
    scan: &mut FunctionScan,
    locals: &mut HashMap<String, Ty>,
) -> BTreeSet<ViewSource> {
    match &statement.kind {
        StmtKind::Let {
            name, ty, value, ..
        } => {
            let borrowed = value
                .as_ref()
                .map(|value| {
                    scan.record_reads(value);
                    scan_expr_effects(checker, ctx, value, depth, scan, locals)
                })
                .unwrap_or_default();
            scan.views.locals.insert(name.clone(), borrowed);
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
            BTreeSet::new()
        }
        StmtKind::Assign { target, value, .. } => {
            scan.record_reads(target);
            scan.record_reads(value);
            if let (Some(root), Some(path)) = (place_root(target), place_path(target)) {
                let path = relative_data_path(&path);
                scan.written_places.insert((root.clone(), path.clone()));
                if ctx.out_params.contains(&root) {
                    scan.produces_result = true;
                    scan.produced_places.insert((root, path));
                }
            }
            scan_expr_effects(checker, ctx, target, depth, scan, locals);
            let borrowed = scan_expr_effects(checker, ctx, value, depth, scan, locals);
            ctx.record_target(target, borrowed, &mut scan.views);
            BTreeSet::new()
        }
        StmtKind::Expr(expr) => {
            scan.record_reads(expr);
            scan_expr_effects(checker, ctx, expr, depth, scan, locals);
            BTreeSet::new()
        }
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            scan.record_reads(condition);
            scan_expr_effects(checker, ctx, condition, depth, scan, locals);
            let mut returned = scan_block_effects(checker, ctx, then_block, depth, scan, locals);
            match else_branch {
                Some(ElseBranch::Block(block)) => {
                    returned.extend(scan_block_effects(checker, ctx, block, depth, scan, locals))
                }
                Some(ElseBranch::If(inner)) => returned.extend(scan_statement_effects(
                    checker, ctx, inner, depth, scan, locals,
                )),
                None => {}
            }
            returned
        }
        StmtKind::While { condition, body } => {
            scan.record_reads(condition);
            scan_expr_effects(checker, ctx, condition, depth, scan, locals);
            scan_block_effects(checker, ctx, body, depth, scan, locals)
        }
        StmtKind::ForRange {
            variable,
            start,
            end,
            body,
        } => {
            scan.record_reads(start);
            scan.record_reads(end);
            scan_expr_effects(checker, ctx, start, depth, scan, locals);
            scan_expr_effects(checker, ctx, end, depth, scan, locals);
            locals.insert(variable.clone(), Ty::U64);
            scan_block_effects(checker, ctx, body, depth, scan, locals)
        }
        StmtKind::ForEach {
            variable,
            iterable,
            body,
            ..
        } => {
            scan.record_reads(iterable);
            scan_expr_effects(checker, ctx, iterable, depth, scan, locals);
            let element = match checker.scan_expr_ty(iterable, locals) {
                Ty::Slice(element) => *element,
                _ => Ty::Error,
            };
            locals.insert(variable.clone(), element);
            scan_block_effects(checker, ctx, body, depth, scan, locals)
        }
        StmtKind::Protect(block) => {
            scan_block_effects(checker, ctx, block, depth + 1, scan, locals)
        }
        StmtKind::Guard {
            name,
            ty,
            value,
            body,
        } => {
            scan.record_reads(value);
            scan_expr_effects(checker, ctx, value, depth, scan, locals);
            let inferred = ty
                .as_ref()
                .map(|ty| checker.resolve_type_pure(ty))
                .unwrap_or_else(|| checker.scan_expr_ty(value, locals));
            locals.insert(name.clone(), inferred);
            // A guard scopes a resource, not a memory region: the body keeps
            // the current `protect` depth.
            scan_block_effects(checker, ctx, body, depth, scan, locals)
        }
        StmtKind::Return(value) => {
            if let Some(value) = value {
                scan.produces_result = true;
                scan.produced_places
                    .insert(("result".to_string(), DataPath::default()));
                if scan.returns_composite
                    && let (Some(root), Some(path)) = (place_root(value), place_path(value))
                {
                    scan.moved_places.insert((root, relative_data_path(&path)));
                }
                scan.record_reads(value);
                scan_expr_effects(checker, ctx, value, depth, scan, locals)
            } else {
                BTreeSet::new()
            }
        }
        StmtKind::Continue | StmtKind::Break => BTreeSet::new(),
        StmtKind::Splice(expr) => {
            scan_expr_effects(checker, ctx, expr, depth, scan, locals);
            BTreeSet::new()
        }
    }
}

fn scan_expr_effects(
    checker: &Checker,
    ctx: &FlowScanCtx<'_>,
    expr: &Expr,
    depth: usize,
    scan: &mut FunctionScan,
    locals: &mut HashMap<String, Ty>,
) -> BTreeSet<ViewSource> {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::String(_) => {
            BTreeSet::new()
        }
        ExprKind::Ident(name) => ctx.sources(name, &scan.views),
        ExprKind::SliceLiteral { elements, .. } => {
            let mut borrowed = BTreeSet::new();
            for element in elements {
                borrowed.extend(scan_expr_effects(
                    checker, ctx, element, depth, scan, locals,
                ));
            }
            borrowed
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            let mut borrowed = BTreeSet::new();
            for field in fields {
                borrowed.extend(scan_expr_effects(
                    checker,
                    ctx,
                    &field.value,
                    depth,
                    scan,
                    locals,
                ));
            }
            borrowed
        }
        ExprKind::Unary { operand, .. } => {
            scan_expr_effects(checker, ctx, operand, depth, scan, locals)
        }
        ExprKind::Binary { left, right, .. } => {
            let mut borrowed = scan_expr_effects(checker, ctx, left, depth, scan, locals);
            borrowed.extend(scan_expr_effects(checker, ctx, right, depth, scan, locals));
            borrowed
        }
        ExprKind::Cast { expr, .. } => scan_expr_effects(checker, ctx, expr, depth, scan, locals),
        ExprKind::Call { callee, args, .. } => {
            let callee_name = if let ExprKind::Ident(name) = &callee.kind {
                Some(name.clone())
            } else {
                scan_expr_effects(checker, ctx, callee, depth, scan, locals);
                None
            };
            if let Some(name) = &callee_name {
                scan.calls.insert(name.clone());
                if depth == 0 {
                    scan.unguarded_deps.push(name.clone());
                    if matches!(
                        name.as_str(),
                        "read_file" | "read_stdin" | "read_line" | "format"
                    ) {
                        scan.direct_alloc = true;
                    }
                }
                scan.call_args.push(CallSiteScan {
                    callee: name.clone(),
                    args: args
                        .iter()
                        .map(|arg| {
                            let place = place_root(&arg.value).zip(place_path(&arg.value));
                            (
                                arg.direction,
                                place.map(|(root, path)| (root, relative_data_path(&path))),
                            )
                        })
                        .collect(),
                });
            }
            let mut arg_sources = Vec::with_capacity(args.len());
            for arg in args {
                arg_sources.push(scan_expr_effects(
                    checker, ctx, &arg.value, depth, scan, locals,
                ));
            }
            callee_name
                .map(|name| ctx.call_sources(&name, args, &arg_sources, &mut scan.views))
                .unwrap_or_default()
        }
        ExprKind::Alloc { target, count, .. } => {
            if depth == 0 {
                scan.direct_alloc = true;
            }
            if let (Some(root), Some(path)) = (place_root(target), place_path(target)) {
                let path = relative_data_path(&path);
                scan.initialized_places.insert((root.clone(), path.clone()));
                if ctx.out_params.contains(&root) {
                    scan.produces_result = true;
                    scan.produced_places.insert((root, path));
                }
            }
            scan_expr_effects(checker, ctx, target, depth, scan, locals);
            scan_expr_effects(checker, ctx, count, depth, scan, locals);
            BTreeSet::new()
        }
        ExprKind::Move(value) => {
            if let (Some(root), Some(path)) = (place_root(value), place_path(value)) {
                scan.moved_places.insert((root, relative_data_path(&path)));
            }
            scan_expr_effects(checker, ctx, value, depth, scan, locals)
        }
        ExprKind::Copy(value) => {
            let value_ty = checker.scan_expr_ty(value, locals);
            if checker.type_contains_slice(&value_ty) {
                scan.clone_direct = true;
                if depth == 0 {
                    scan.direct_alloc = true;
                }
            }
            scan_expr_effects(checker, ctx, value, depth, scan, locals);
            // A clone owns its own storage; it cannot extend a source loan.
            BTreeSet::new()
        }
        ExprKind::Member { base, field } => {
            let relative = DataPath::new("").field(field);
            scan_expr_effects(checker, ctx, base, depth, scan, locals)
                .into_iter()
                .map(|source| source.extended_by(&relative))
                .collect()
        }
        ExprKind::Comptime(base) => scan_expr_effects(checker, ctx, base, depth, scan, locals),
        ExprKind::Index { base, index } => {
            let mut borrowed: BTreeSet<ViewSource> =
                scan_expr_effects(checker, ctx, base, depth, scan, locals)
                    .into_iter()
                    .map(|source| source.extended_by(&index_data_path(index)))
                    .collect();
            borrowed.extend(scan_expr_effects(checker, ctx, index, depth, scan, locals));
            borrowed
        }
        ExprKind::Quote(_) | ExprKind::Unquote(_) => BTreeSet::new(),
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

/// Append a relative path made of whole components. The relative part has no
/// leading separator, so a field needs a dot.
fn append_relative_path(base: &str, relative: &str) -> String {
    if relative.is_empty() {
        base.to_string()
    } else {
        append_path(base, relative)
    }
}

/// Convert a root-relative string at a legacy boundary into a typed path.
fn relative_data_path(relative: &str) -> DataPath {
    DataPath::from_relative("", relative)
}

fn path_is_ancestor(ancestor: &str, path: &DataPath) -> bool {
    relative_data_path(ancestor).is_ancestor_of(path)
}

/// Express a loan's holder-relative slot relative to the selected sub-place.
/// Selecting below the slot makes the selected value itself the view (`""`);
/// selecting above it keeps the remaining suffix. Disjoint places return
/// `None`, so a view stored in `s.a` is not carried by `s.b`.
fn path_suffix(slot: &DataPath, path: &str) -> Option<DataPath> {
    let path = relative_data_path(path);
    if path.is_whole() {
        return Some(slot.clone());
    }
    if slot.is_whole() {
        return Some(DataPath::default());
    }
    if path.is_ancestor_of(slot) {
        return Some(DataPath {
            root: String::new(),
            segments: slot.segments[path.segments.len()..].to_vec(),
        });
    }
    if slot.is_ancestor_of(&path) {
        return Some(DataPath::default());
    }
    None
}

/// Add a read path to a scan set. A whole-value read subsumes every narrower
/// path, so the set collapses to a segment-free path.
fn insert_read_path(paths: &mut HashSet<DataPath>, path: DataPath) -> bool {
    if paths.iter().any(DataPath::is_whole) {
        return false;
    }
    if path.is_whole() {
        paths.clear();
        paths.insert(path);
        true
    } else {
        paths.insert(path)
    }
}

fn paths_overlap(left: &str, right: &str) -> bool {
    relative_data_path(left).may_overlap(&relative_data_path(right))
}

fn path_is_moved(moved: &HashMap<DataPath, Span>, path: &str) -> bool {
    let path = relative_data_path(path);
    moved.keys().any(|moved| moved.may_overlap(&path))
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

/// Typed counterpart of `index_path`, keeping dynamic indexes structural.
fn index_data_path(index: &Expr) -> DataPath {
    let key = match const_int(index) {
        Some(value) => IndexKey::Const(value),
        None => IndexKey::Dynamic,
    };
    DataPath::new("").index(key)
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
        ExprKind::Quote(_) | ExprKind::Unquote(_) => {}
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
    reads: &mut [HashSet<DataPath>],
) {
    match &expr.kind {
        ExprKind::Ident(name) => {
            if let Some(&index) = read_params.get(name) {
                insert_read_path(&mut reads[index], DataPath::default());
            }
        }
        ExprKind::Member { .. } | ExprKind::Index { .. } => {
            if let (Some(root), Some(path)) = (place_root(expr), place_path(expr))
                && let Some(&index) = read_params.get(&root)
            {
                insert_read_path(&mut reads[index], relative_data_path(&path));
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
        ExprKind::Quote(_) | ExprKind::Unquote(_) => {}
    }
}

fn collect_direct_index_reads(
    expr: &Expr,
    read_params: &HashMap<String, usize>,
    reads: &mut [HashSet<DataPath>],
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

#[cfg(test)]
mod flow_integration_tests {
    use super::*;
    use crate::{parse, tokenize};

    fn checker_for(source: &str) -> Checker {
        let tokens = tokenize(source, "<test>").expect("source should lex");
        let program = parse(tokens).expect("source should parse");
        let mut checker = Checker::new();
        checker.collect_names(&program);
        checker.resolve_interfaces(&program);
        checker.resolve_marks_and_choices(&program);
        checker.resolve_functions(&program);
        checker.analyze_flow_summaries(&program);
        checker
    }

    fn check_program_for(source: &str) -> Vec<Diagnostic> {
        let tokens = tokenize(source, "<test>").expect("source should lex");
        let program = parse(tokens).expect("source should parse");
        check(&program)
    }

    #[test]
    fn legacy_effects_are_exposed_by_one_flow_summary() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: [i32]
}

flow consume
    <-> p: Pair
{
    p.left[0] = 0
    gone <- give(p.left)
}

flow read_right
    <- p: Pair
    => i32
{
    => p.right[0]
}

flow clone_it
    <- xs: [i32]
    -> out: [i32]
{
    out = clone(xs)
}

flow head2
    <- xs: [u8]
    => [u8]
{
    => view(xs, 0, 2)
}

flow fill
    <- xs: [u8]
    -> out: [u8]
{
    out = view(xs, 0, 2)
}
"#,
        );

        let consume = checker.summary("consume").expect("consume summary");
        assert_eq!(consume.consumed_paths(0), vec!["left"]);
        let read_right = checker.summary("read_right").expect("read_right summary");
        assert_eq!(read_right.read_paths(0), vec!["right[0]"]);
        let clone_it = checker.summary("clone_it").expect("clone_it summary");
        assert!(clone_it.allocations);
        assert!(clone_it.unguarded_allocations);
        assert!(clone_it.clone_allocations);
        let head2 = checker.summary("head2").expect("head2 summary");
        assert!(head2.view_returns().contains(&0));
        let fill = checker.summary("fill").expect("fill summary");
        assert!(
            fill.view_out()
                .get(&1)
                .is_some_and(|sources| sources.contains(&0))
        );
    }

    #[test]
    fn one_visitor_produces_transitive_effects_for_independent_paths() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: i32
}

flow consume_left
    <-> p: Pair
{
    gone <- give(p.left)
}

flow forward_consume
    <-> p: Pair
{
    consume_left(<-> p)
}

flow read_right
    <- p: Pair
    => i32
{
    => p.right
}

flow forward_read
    <- p: Pair
    => i32
{
    => read_right(p)
}

flow head2
    <- xs: [u8]
    => [u8]
{
    => view(xs, 0, 2)
}

flow forward_view
    <- xs: [u8]
    => [u8]
{
    => head2(xs)
}
"#,
        );

        let consume_left = checker.summary("consume_left").expect("consume_left");
        assert_eq!(consume_left.consumed_paths(0), vec!["left"]);
        let forward_consume = checker.summary("forward_consume").expect("forward_consume");
        assert_eq!(forward_consume.consumed_paths(0), vec!["left"]);

        let read_right = checker.summary("read_right").expect("read_right");
        assert_eq!(read_right.read_paths(0), vec!["right"]);
        let forward_read = checker.summary("forward_read").expect("forward_read");
        assert_eq!(forward_read.read_paths(0), vec!["right"]);

        let head2 = checker.summary("head2").expect("head2");
        assert!(head2.view_returns().contains(&0));
        let forward_view = checker.summary("forward_view").expect("forward_view");
        assert!(forward_view.view_returns().contains(&0));
    }

    /// Teste A: o contrato inferido cobre efeitos além de leitura e movimento,
    /// preservando caminhos independentes e a propagação entre flows.
    #[test]
    fn inferred_contract_records_writes_inits_production_and_escape() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: i32
}

flow write_left
    <-> p: Pair
    <- value: i32
{
    p.left[0] = value
}

flow forward_write
    <-> p: Pair
    <- value: i32
{
    write_left(<-> p, <- value)
}

flow init_out
    -> out: [i32]
{
    alloc(-> out, i32, 2)
    out[1] = 7
}

flow escape_left
    <- p: Pair
    -> out: [i32]
{
    out = view(p.left, 0, 1)
}

flow forward_escape
    <- p: Pair
    -> out: [i32]
{
    escape_left(<- p, -> out)
}
"#,
        );

        let write_left = checker.summary("write_left").expect("write_left");
        assert_eq!(
            write_left.params[0].writes.relative_strings(),
            vec!["left[0]"]
        );
        let forward_write = checker.summary("forward_write").expect("forward_write");
        assert_eq!(
            forward_write.params[0].writes.relative_strings(),
            vec!["left[0]"]
        );

        let init_out = checker.summary("init_out").expect("init_out");
        assert_eq!(init_out.params[0].inits.relative_strings(), vec![""]);

        let escape_left = checker.summary("escape_left").expect("escape_left");
        let forward_escape = checker.summary("forward_escape").expect("forward_escape");
        assert!(escape_left.result.produces);
        // The borrowed place is `p.left`, not the whole parameter.
        assert_eq!(
            escape_left.params[0].escapes.relative_strings(),
            vec!["left"]
        );
        assert!(forward_escape.result.produces);
        assert_eq!(
            forward_escape.params[0].escapes.relative_strings(),
            vec!["left"]
        );
    }

    /// FASE 2: a value delivered to the caller is a `PRODUCE` with a path,
    /// not only a `produces` boolean. Both the `alloc` destination and the
    /// assigned field must be visible in the durable contract.
    #[test]
    fn produced_paths_record_the_destination_written_by_the_flow() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: i32
}

flow fill_out
    -> out: Pair
{
    alloc(-> out.left, i32, 2)
    out.right = 7
}
"#,
        );

        let summary = checker.summary("fill_out").expect("fill_out");
        assert!(summary.result.produces);
        let produced: Vec<&DataPath> = summary.result.paths.iter().collect();
        assert!(
            produced
                .iter()
                .any(|path| path.root == "out" && path.relative_string() == "left"),
            "expected out.left in produced paths, got {produced:?}"
        );
        assert!(
            produced
                .iter()
                .any(|path| path.root == "out" && path.relative_string() == "right"),
            "expected out.right in produced paths, got {produced:?}"
        );
    }

    /// FASE 2: a produced path must follow the caller's destination, including
    /// the field selected by the callee, not only the `produces` boolean.
    #[test]
    fn produced_paths_propagate_through_calls() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: i32
}

flow make_left
    -> out: Pair
{
    alloc(-> out.left, i32, 2)
}

flow relay_left
    -> dst: Pair
{
    make_left(-> dst)
}

flow make_whole
    -> out: [i32]
{
    alloc(-> out, i32, 2)
}

flow relay_whole
    -> dst: [i32]
{
    make_whole(-> dst)
}
"#,
        );

        let relay_left = checker.summary("relay_left").expect("relay_left");
        assert!(relay_left.result.produces);
        assert!(
            relay_left
                .result
                .paths
                .iter()
                .any(|path| { path.root == "dst" && path.relative_string() == "left" }),
            "expected dst.left, got {:?}",
            relay_left.result.paths
        );

        let relay_whole = checker.summary("relay_whole").expect("relay_whole");
        assert!(
            relay_whole
                .result
                .paths
                .iter()
                .any(|path| path.root == "dst" && path.is_whole()),
            "expected whole dst, got {:?}",
            relay_whole.result.paths
        );
    }

    /// FASE 2: an escape records the borrowed *field*, not the whole source.
    #[test]
    fn escaped_source_keeps_its_relative_path() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: [i32]
}

flow escape_left
    <- p: Pair
    -> out: [i32]
{
    out = view(p.left, 0, 1)
}

flow relay_left
    <- q: Pair
    -> dst: [i32]
{
    escape_left(<- q, -> dst)
}
"#,
        );

        let direct = checker.summary("escape_left").expect("escape_left");
        assert_eq!(direct.params[0].escapes.relative_strings(), vec!["left"]);
        let relayed = checker.summary("relay_left").expect("relay_left");
        assert_eq!(relayed.params[0].escapes.relative_strings(), vec!["left"]);
    }

    /// FASE 2: field components compose across nested paths and forwarding.
    #[test]
    fn escaped_source_keeps_nested_relative_paths() {
        let checker = checker_for(
            r#"shape Inner {
    leaf: [i32]
}

shape Outer {
    inner: Inner
}

flow escape_nested
    <- p: Outer
    -> out: [i32]
{
    out = view(p.inner.leaf, 0, 1)
}

flow relay_nested
    <- q: Outer
    -> dst: [i32]
{
    escape_nested(<- q, -> dst)
}
"#,
        );

        let direct = checker.summary("escape_nested").expect("escape_nested");
        assert_eq!(
            direct.params[0].escapes.relative_strings(),
            vec!["inner.leaf"]
        );
        let relayed = checker.summary("relay_nested").expect("relay_nested");
        assert_eq!(
            relayed.params[0].escapes.relative_strings(),
            vec!["inner.leaf"]
        );
    }

    /// FASE 2: a produced field is re-rooted on the caller's nested destination.
    #[test]
    fn produced_paths_keep_the_call_site_nested_path() {
        let checker = checker_for(
            r#"shape Pair {
    left: [i32]
    right: i32
}

shape Holder {
    nested: Pair
}

flow make_left
    -> out: Pair
{
    alloc(-> out.left, i32, 2)
}

flow relay_nested
    -> dst: Holder
{
    make_left(-> dst.nested)
}
"#,
        );

        let summary = checker.summary("relay_nested").expect("relay_nested");
        assert!(
            summary
                .result
                .paths
                .iter()
                .any(|path| { path.root == "dst" && path.relative_string() == "nested.left" }),
            "expected dst.nested.left, got {:?}",
            summary.result.paths
        );
    }

    /// FASE 2: consumption summarized for a field must be applied to the
    /// caller's nested argument, not to a concatenated field name.
    #[test]
    fn nested_consumption_is_applied_at_the_call_site() {
        let source = r#"shape Inner {
    leaf: [i32]
    other: [i32]
}

shape Outer {
    inner: Inner
}

flow consume_leaf
    <- n: u64
    <-> x: Inner
{
    alloc(-> x.leaf, i32, n)
    mut gone: [i32] <- give(x.leaf)
}

flow consume_nested
    <- n: u64
    <-> o: Outer
{
    consume_leaf(<- n, <-> o.inner)
    print_int(o.inner.leaf.len)
}
"#;
        let summary = checker_for(source).summary("consume_leaf").unwrap().clone();
        assert!(
            summary.consumed_paths(1).iter().any(|path| path == "leaf"),
            "expected leaf in consumed paths, got {:?}",
            summary.consumed_paths(1)
        );
        let diagnostics = check_program_for(source);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("use of moved value")),
            "expected use of moved value, got {diagnostics:?}"
        );
    }

    /// FASE 2: a read summary must be re-rooted on the caller's nested
    /// argument before it is compared with moved paths.
    #[test]
    fn nested_read_summary_detects_a_moved_field() {
        let source = r#"shape Inner {
    leaf: [i32]
    other: [i32]
}

shape Outer {
    inner: Inner
}

flow read_leaf
    <- x: Inner
    => u64
{
    => x.leaf.len
}

flow read_nested
    => u64
{
    mut o: Outer <- Outer {
        inner: Inner {
            leaf: [i32: 1]
            other: [i32: 2]
        }
    }
    mut gone: [i32] <- give(o.inner.leaf)
    => read_leaf(<- o.inner)
}
"#;
        let summary = checker_for(source).summary("read_leaf").unwrap().clone();
        assert!(
            summary.read_paths(0).iter().any(|path| path == "leaf.len"),
            "expected leaf.len in read paths, got {:?}",
            summary.read_paths(0)
        );
        let diagnostics = check_program_for(source);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("use of moved value")),
            "expected use of moved value, got {diagnostics:?}"
        );
    }

    /// FASE 2: the durable loan also names the holder slot, not just the
    /// `->` parameter root.
    #[test]
    fn escaped_loan_keeps_the_holder_slot() {
        let checker = checker_for(
            r#"shape Holder {
    a: [i32]
    b: [i32]
}

flow slot_out
    <- p: [i32]
    -> out: Holder
{
    out.a = view(p, 0, 1)
}
"#,
        );

        let summary = checker.summary("slot_out").expect("slot_out");
        let loans = summary.escaped_loans();
        assert!(
            loans.iter().any(|loan| {
                matches!(loan.destination, LoanDestination::OutParam(1))
                    && loan
                        .source_path()
                        .is_some_and(|path| path.relative_string().is_empty())
                    && loan.holder.relative_string() == "a"
            }),
            "expected a loan of p into out.a, got {loans:?}"
        );
    }

    /// FASE 2: silence in an extern body is not evidence of no effects. The
    /// summary must flag the uncertainty and stay conservative by direction.
    #[test]
    fn extern_flow_marks_unknown_effects_and_propagates_them() {
        let checker = checker_for(
            r#"extern flow foreign
    -> out: [i32]
    <-> both: [i32]

flow relay
    -> dst: [i32]
{
    foreign(-> dst)
}
"#,
        );

        let foreign = checker.summary("foreign").expect("foreign");
        assert!(foreign.unknown_effects);
        assert!(foreign.params[0].inits.iter().any(DataPath::is_whole));
        assert!(foreign.params[1].writes.iter().any(DataPath::is_whole));

        let relay = checker.summary("relay").expect("relay");
        assert!(relay.unknown_effects);
        assert!(relay.params[0].inits.iter().any(DataPath::is_whole));
    }

    /// FASE 2: a call with no summary cannot be treated as effect-free.
    #[test]
    fn call_without_summary_marks_unknown_effects() {
        let checker = checker_for(
            r#"flow relay_unknown
    -> dst: [i32]
{
    mystery(-> dst)
}
"#,
        );

        let summary = checker.summary("relay_unknown").expect("relay_unknown");
        assert!(summary.unknown_effects);
        assert!(summary.params[0].inits.iter().any(DataPath::is_whole));
    }
}
