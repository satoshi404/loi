//! Opt-in LLVM IR backend for the scalar and slice subset of Loi.
//!
//! This is deliberately separate from `ir.rs`, whose textual output is an
//! early draft and not valid LLVM IR. The emitter works from the checked,
//! monomorphized AST and produces textual IR with opaque pointers so it can be
//! compiled directly by `clang -x ir`. The C11 backend remains the default and
//! the production path; unsupported language features are rejected with a
//! diagnostic instead of being lowered incorrectly.
//!
//! Slices use the C ABI layout `{ ptr, i64 }` (`void* data`, `size_t len`) and
//! the same runtime contract as the C11 backend: a bump arena whose offset is
//! saved and restored by `protect`, checked indexing and byte-wise printing.

use std::collections::{HashMap, HashSet};

use crate::ast::{self, *};
use crate::types::{self, Ty};

/// Generate textual LLVM IR for a checked program.
///
/// Returns a human-readable error when the program uses a feature outside the
/// scalar subset. The caller reports it with the source path and a normal
/// frontend-style diagnostic prefix.
pub fn generate_llvm_ir(program: &Program) -> Result<String, String> {
    let (owned, errors) = crate::monomorphize::monomorphize(program);
    if let Some(error) = errors.first() {
        return Err(format!(
            "{}:{}: {}",
            error.span.line, error.span.column, error.message
        ));
    }
    let mut emitter = Emitter::new(&owned)?;
    emitter.emit_module()
}

/// A basic block under construction.
struct BasicBlock {
    label: String,
    lines: Vec<String>,
    terminator: Option<String>,
}

impl BasicBlock {
    fn new(label: String) -> Self {
        Self {
            label,
            lines: Vec::new(),
            terminator: None,
        }
    }

    fn line(&mut self, text: impl Into<String>) {
        if self.terminator.is_none() {
            self.lines.push(text.into());
        }
    }

    fn terminate(&mut self, text: impl Into<String>) {
        if self.terminator.is_none() {
            self.terminator = Some(text.into());
        }
    }

    fn finish(self, out: &mut String) {
        out.push_str(&format!("{}:\n", self.label));
        for line in self.lines {
            out.push_str(&format!("  {line}\n"));
        }
        if let Some(terminator) = self.terminator {
            out.push_str(&format!("  {terminator}\n"));
        }
        out.push('\n');
    }
}

#[derive(Clone)]
struct Binding {
    value: String,
    ty: Ty,
}

struct Emitter<'a> {
    program: &'a Program,
    functions: HashMap<String, &'a FunctionDecl>,
    marks: HashSet<String>,
    choices: HashSet<String>,
    opaques: HashSet<String>,
    strings: Vec<(String, String)>,
    string_index: HashMap<String, String>,
    counter: usize,
    current: BasicBlock,
    finished: Vec<BasicBlock>,
    scopes: Vec<HashMap<String, Binding>>,
    loops: Vec<(String, String)>,
    current_return: Ty,
    uses_float_helper: bool,
    uses_math: HashSet<String>,
}

impl<'a> Emitter<'a> {
    fn new(program: &'a Program) -> Result<Self, String> {
        let mut emitter = Self {
            program,
            functions: HashMap::new(),
            marks: HashSet::new(),
            choices: HashSet::new(),
            opaques: HashSet::new(),
            strings: Vec::new(),
            string_index: HashMap::new(),
            counter: 0,
            current: BasicBlock::new("entry".to_string()),
            finished: Vec::new(),
            scopes: Vec::new(),
            loops: Vec::new(),
            current_return: Ty::Void,
            uses_float_helper: false,
            uses_math: HashSet::new(),
        };
        emitter.collect_names();
        emitter.validate()?;
        Ok(emitter)
    }

    // --------------------------------------------------------------
    // Collection and validation
    // --------------------------------------------------------------

    fn collect_names(&mut self) {
        for item in &self.program.items {
            match item {
                Item::Function(decl) => {
                    self.functions.insert(decl.name.clone(), decl);
                }
                Item::Mark(decl) => {
                    self.marks.insert(decl.name.clone());
                }
                Item::Choice(decl) => {
                    self.choices.insert(decl.name.clone());
                }
                Item::Opaque(decl) => {
                    self.opaques.insert(decl.name.clone());
                }
                _ => {}
            }
        }
    }

    fn validate(&self) -> Result<(), String> {
        for item in &self.program.items {
            match item {
                Item::Import(_) => {}
                Item::Interface(_) => {
                    return Err("LLVM backend does not support shape types".to_string());
                }
                Item::Mark(_) => {
                    return Err("LLVM backend does not support mark types".to_string());
                }
                Item::Choice(_) => {
                    return Err("LLVM backend does not support choice types".to_string());
                }
                Item::Opaque(_) => {
                    return Err("LLVM backend does not support opaque types".to_string());
                }
                Item::Global(_) => {
                    return Err("LLVM backend does not support global declarations".to_string());
                }
                Item::Function(decl) => {
                    if !decl.type_params.is_empty() {
                        return Err(
                            "LLVM backend does not support unresolved generic declarations"
                                .to_string(),
                        );
                    }
                    for param in &decl.params {
                        self.validate_type(&param.ty)?;
                    }
                    if let Some(ty) = &decl.return_type {
                        self.validate_type(ty)?;
                    }
                    if !decl.external {
                        self.validate_block(&decl.body)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_type(&self, ty: &Type) -> Result<(), String> {
        match ty {
            Type::Named(name, _) => {
                if name == "void" || Ty::from_name(name).is_some() {
                    Ok(())
                } else if name == "callback" {
                    Err("LLVM backend does not support callback values".to_string())
                } else if name == "cstr" {
                    Err("LLVM backend does not support cstr handles".to_string())
                } else if self.marks.contains(name) {
                    Err("LLVM backend does not support mark types".to_string())
                } else if self.choices.contains(name) {
                    Err("LLVM backend does not support choice types".to_string())
                } else if self.opaques.contains(name) {
                    Err("LLVM backend does not support opaque types".to_string())
                } else {
                    Err("LLVM backend does not support shape types".to_string())
                }
            }
            Type::Slice(element, _) => {
                self.validate_slice_element(element)?;
                Ok(())
            }
            Type::Generic(_, _, _) => {
                Err("LLVM backend does not support unresolved generic declarations".to_string())
            }
            Type::Callback { .. } => {
                Err("LLVM backend does not support callback values".to_string())
            }
        }
    }

    /// Slice elements are restricted to scalar values whose size is known at
    /// compile time. Nested slices and user-defined composites still need an
    /// agreed LLVM ABI, so they stay on the C11 path.
    fn validate_slice_element(&self, element: &Type) -> Result<(), String> {
        match element {
            Type::Named(name, _) if name == "void" => {
                Err("LLVM backend does not support void slice elements".to_string())
            }
            Type::Named(name, _) if Ty::from_name(name).is_some() => Ok(()),
            Type::Named(_, _) => self.validate_type(element),
            Type::Slice(_, _) => {
                Err("LLVM backend does not support nested slice elements".to_string())
            }
            Type::Generic(_, _, _) => {
                Err("LLVM backend does not support unresolved generic declarations".to_string())
            }
            Type::Callback { .. } => {
                Err("LLVM backend does not support callback values".to_string())
            }
        }
    }

    fn validate_block(&self, block: &ast::Block) -> Result<(), String> {
        for statement in &block.statements {
            self.validate_stmt(statement)?;
        }
        Ok(())
    }

    fn validate_stmt(&self, statement: &Stmt) -> Result<(), String> {
        match &statement.kind {
            StmtKind::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    self.validate_type(ty)?;
                }
                match value {
                    Some(value) => self.validate_expr(value)?,
                    None => {
                        if !matches!(ty, Some(Type::Slice(_, _))) {
                            return Err("LLVM backend requires an initializer for every binding"
                                .to_string());
                        }
                    }
                }
            }
            StmtKind::Assign { target, value, .. } => {
                self.validate_expr(target)?;
                self.validate_expr(value)?;
                if !matches!(target.kind, ExprKind::Ident(_) | ExprKind::Index { .. }) {
                    return Err(
                        "LLVM backend only supports assignment to a binding or slice element"
                            .to_string(),
                    );
                }
            }
            StmtKind::Expr(expr) | StmtKind::Return(Some(expr)) => self.validate_expr(expr)?,
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                self.validate_expr(condition)?;
                self.validate_block(then_block)?;
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.validate_block(block)?,
                    Some(ElseBranch::If(statement)) => self.validate_stmt(statement)?,
                    None => {}
                }
            }
            StmtKind::While { condition, body } => {
                self.validate_expr(condition)?;
                self.validate_block(body)?;
            }
            StmtKind::ForRange {
                start, end, body, ..
            } => {
                self.validate_expr(start)?;
                self.validate_expr(end)?;
                self.validate_block(body)?;
            }
            StmtKind::ForEach { .. } => {
                return Err("LLVM backend does not support foreach loops".to_string());
            }
            StmtKind::Protect(block) => self.validate_block(block)?,
            StmtKind::Guard { .. } => {
                return Err("LLVM backend does not support 'guard' bindings".to_string());
            }
            StmtKind::Return(None) | StmtKind::Continue | StmtKind::Break => {}
        }
        Ok(())
    }

    fn validate_expr(&self, expr: &Expr) -> Result<(), String> {
        match &expr.kind {
            ExprKind::String(_) | ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) => {}
            ExprKind::Ident(_) => {}
            ExprKind::Unary { operand, .. } => self.validate_expr(operand)?,
            ExprKind::Binary { left, right, .. } => {
                self.validate_expr(left)?;
                self.validate_expr(right)?;
            }
            ExprKind::Cast { expr, ty } => {
                self.validate_expr(expr)?;
                self.validate_type(ty)?;
            }
            ExprKind::Call {
                callee,
                type_args,
                args,
                ..
            } => {
                if !type_args.is_empty() {
                    return Err(
                        "LLVM backend does not support unresolved generic declarations".to_string(),
                    );
                }
                let ExprKind::Ident(name) = &callee.kind else {
                    return Err(
                        "LLVM backend does not support indirect calls or function values"
                            .to_string(),
                    );
                };
                if !SUPPORTED_INTRINSICS.contains(&name.as_str())
                    && !self.functions.contains_key(name)
                {
                    return Err(format!("LLVM backend does not support intrinsic '{name}'"));
                }
                for arg in args {
                    self.validate_expr(&arg.value)?;
                }
            }
            ExprKind::Move(inner) | ExprKind::Copy(inner) => self.validate_expr(inner)?,
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                if let Some(element_type) = element_type {
                    self.validate_slice_element(element_type)?;
                }
                for element in elements {
                    self.validate_expr(element)?;
                }
            }
            ExprKind::InterfaceLiteral { .. } => {
                return Err("LLVM backend does not support shape literals".to_string());
            }
            ExprKind::Alloc {
                target,
                element_type,
                count,
            } => {
                if !matches!(target.kind, ExprKind::Ident(_)) {
                    return Err("LLVM backend: 'alloc' requires a binding".to_string());
                }
                self.validate_slice_element(element_type)?;
                self.validate_expr(count)?;
            }
            ExprKind::Member { base, field } => {
                if field != "len" {
                    return Err("LLVM backend does not support field access".to_string());
                }
                self.validate_expr(base)?;
            }
            ExprKind::Index { base, index } => {
                self.validate_expr(base)?;
                self.validate_expr(index)?;
            }
            ExprKind::Comptime(_) => {
                return Err("LLVM backend does not support 'comptime' expressions".to_string());
            }
        }
        Ok(())
    }

    // --------------------------------------------------------------
    // Module emission
    // --------------------------------------------------------------

    fn emit_module(&mut self) -> Result<String, String> {
        let mut header = String::from("; Generated by the Loi LLVM backend\n\n");
        header.push_str("%LoiSlice = type { ptr, i64 }\n\n");
        header.push_str("declare i32 @printf(ptr, ...)\n");
        header.push_str("declare i32 @fputs(ptr, ptr)\n");
        header.push_str("declare i32 @fwrite(ptr, i64, i64, ptr)\n");
        header.push_str("@stdout = external global ptr\n");
        header.push_str("@stderr = external global ptr\n");

        let mut declarations = String::new();
        for decl in self.program.items.iter().filter_map(|item| match item {
            Item::Function(decl) => Some(decl),
            _ => None,
        }) {
            if !decl.external {
                continue;
            }
            let name = self.function_name(decl);
            let return_ty = self.signature_return(decl);
            let params = decl
                .params
                .iter()
                .map(|param| self.ast_type(&param.ty))
                .collect::<Result<Vec<_>, _>>()?;
            let params = params
                .iter()
                .map(|ty| self.llvm_type(ty))
                .collect::<Result<Vec<_>, _>>()?;
            let signature = params.join(", ");
            declarations.push_str(&format!("declare {return_ty} @{name}({signature})\n"));
        }
        declarations.push('\n');

        let mut body = String::new();
        for decl in self.program.items.iter().filter_map(|item| match item {
            Item::Function(decl) => Some(decl),
            _ => None,
        }) {
            if decl.external {
                continue;
            }
            self.emit_function(decl, &mut body)?;
        }

        let mut strings = String::new();
        for (name, literal) in &self.strings {
            strings.push_str(&format!("{name} = internal constant {literal}\n"));
        }
        if !strings.is_empty() {
            strings.push('\n');
        }

        let mut helpers = String::new();
        if self.uses_float_helper {
            helpers.push_str(&declare_float_helper_decls());
            helpers.push_str(LOI_PRINT_FLOAT_HELPER);
        }
        for name in &self.uses_math {
            let params = if *name == "pow" {
                "double, double"
            } else {
                "double"
            };
            helpers.push_str(&format!("declare double @{name}({params})\n"));
        }
        if !helpers.is_empty() {
            helpers.push('\n');
        }

        let mut result = String::new();
        result.push_str(&header);
        result.push_str(&strings);
        result.push_str(&helpers);
        result.push_str(&declarations);
        result.push_str(&body);
        result.push_str(LOI_SLICE_RUNTIME);
        Ok(result)
    }

    fn emit_function(&mut self, decl: &FunctionDecl, out: &mut String) -> Result<(), String> {
        self.current = BasicBlock::new("entry".to_string());
        self.finished.clear();
        self.scopes.clear();
        self.loops.clear();
        self.current_return = self.declared_return(decl);
        let signature_return = self.signature_return(decl);

        let mut parameters = Vec::new();
        let mut scope = HashMap::new();
        for param in &decl.params {
            let semantic = self.ast_type(&param.ty)?;
            let ty = self.llvm_type(&semantic)?;
            let local = self.fresh_local(&param.name);
            let variable = format!("{local}.addr");
            self.current.line(format!("{variable} = alloca {ty}"));
            self.current
                .line(format!("store {ty} {local}, ptr {variable}"));
            scope.insert(
                param.name.clone(),
                Binding {
                    value: variable,
                    ty: self.ast_type(&param.ty)?,
                },
            );
            parameters.push(format!("{ty} {local}"));
        }
        self.scopes.push(scope);

        self.emit_block(&decl.body)?;
        if self.current.terminator.is_none() {
            let terminator = if signature_return == Ty::Void {
                "ret void".to_string()
            } else if decl.name == "main" {
                "ret i32 0".to_string()
            } else {
                format!(
                    "ret {} {}",
                    self.llvm_type(&signature_return)?,
                    self.zero_value(&signature_return)?
                )
            };
            self.current.terminate(terminator);
        }

        let name = self.function_name(decl);
        out.push_str(&format!(
            "define {} @{name}({}) {{\n",
            self.llvm_type(&signature_return)?,
            parameters.join(", ")
        ));
        for block in std::mem::take(&mut self.finished) {
            block.finish(out);
        }
        let last = std::mem::replace(&mut self.current, BasicBlock::new("entry".to_string()));
        last.finish(out);
        out.push_str("}\n\n");
        Ok(())
    }

    // --------------------------------------------------------------
    // Statements
    // --------------------------------------------------------------

    fn emit_block(&mut self, block: &ast::Block) -> Result<(), String> {
        self.scopes.push(HashMap::new());
        for statement in &block.statements {
            if self.current.terminator.is_some() {
                break;
            }
            self.emit_stmt(statement)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn emit_stmt(&mut self, statement: &Stmt) -> Result<(), String> {
        match &statement.kind {
            StmtKind::Let {
                name, ty, value, ..
            } => {
                let expected = match ty {
                    Some(ty) => Some(self.ast_type(ty)?),
                    None => None,
                };
                let Some(value) = value else {
                    let ty = expected.ok_or_else(|| {
                        "LLVM backend requires an initializer for every binding".to_string()
                    })?;
                    if !matches!(ty, Ty::Slice(_)) {
                        return Err(
                            "LLVM backend requires an initializer for every binding".to_string()
                        );
                    }
                    let llvm = self.llvm_type(&ty)?;
                    let variable = self.fresh_local(name);
                    self.current.line(format!("{variable} = alloca {llvm}"));
                    self.current.line(format!(
                        "store {llvm} {}, ptr {variable}",
                        self.zero_value(&ty)?
                    ));
                    self.bind(name, variable, ty);
                    return Ok(());
                };
                let inferred = self.infer(value, expected.as_ref())?;
                let final_ty = expected.unwrap_or_else(|| inferred.clone());
                let code = self.gen_expr(value, Some(&final_ty))?;
                let code = self.coerce(&code, &inferred, &final_ty)?;
                let variable = self.fresh_local(name);
                let llvm = self.llvm_type(&final_ty)?;
                self.current.line(format!("{variable} = alloca {llvm}"));
                self.current
                    .line(format!("store {llvm} {code}, ptr {variable}"));
                self.bind(name, variable, final_ty);
            }
            StmtKind::Assign { target, op, value } => {
                let (pointer, ty) = self.gen_place(target)?;
                let inferred = self.infer(value, Some(&ty))?;
                let mut code = self.gen_expr(value, Some(&ty))?;
                code = self.coerce(&code, &inferred, &ty)?;
                if *op != AssignOp::Assign {
                    let previous = self.load_raw(&pointer, &ty);
                    code = self.binary_code(op_to_binary(*op), &previous, &code, &ty)?;
                }
                self.current.line(format!(
                    "store {} {code}, ptr {}",
                    self.llvm_type(&ty)?,
                    pointer
                ));
            }
            StmtKind::Expr(expr) => {
                self.gen_expr(expr, None)?;
            }
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let condition = self.gen_expr(condition, Some(&Ty::Bool))?;
                let then_label = self.fresh_label("then");
                let else_label = self.fresh_label("else");
                let end_label = self.fresh_label("end");
                self.current.terminate(format!(
                    "br i1 {condition}, label %{then_label}, label %{else_label}"
                ));
                self.push_block(then_label);
                self.emit_block(then_block)?;
                if self.current.terminator.is_none() {
                    self.current.terminate(format!("br label %{end_label}"));
                }

                self.push_block(else_label.clone());
                match else_branch {
                    Some(ElseBranch::Block(block)) => self.emit_block(block)?,
                    Some(ElseBranch::If(statement)) => self.emit_stmt(statement)?,
                    None => {}
                }
                if self.current.terminator.is_none() {
                    self.current.terminate(format!("br label %{end_label}"));
                }
                self.push_block(end_label);
            }
            StmtKind::While { condition, body } => {
                let cond_label = self.fresh_label("while.cond");
                let body_label = self.fresh_label("while.body");
                let end_label = self.fresh_label("while.end");
                self.current.terminate(format!("br label %{cond_label}"));
                self.push_block(cond_label.clone());
                let condition = self.gen_expr(condition, Some(&Ty::Bool))?;
                self.current.terminate(format!(
                    "br i1 {condition}, label %{body_label}, label %{end_label}"
                ));
                self.loops.push((cond_label.clone(), end_label.clone()));
                self.push_block(body_label);
                self.emit_block(body)?;
                self.loops.pop();
                if self.current.terminator.is_none() {
                    self.current.terminate(format!("br label %{cond_label}"));
                }
                self.push_block(end_label);
            }
            StmtKind::ForRange {
                variable,
                start,
                end,
                body,
            } => {
                let start_ty = self.infer(start, Some(&Ty::I32))?;
                let start_code = self.gen_expr(start, Some(&start_ty))?;
                let end_ty = self.infer(end, Some(&start_ty))?;
                let end_code = self.gen_expr(end, Some(&end_ty))?;
                let end_code = self.coerce(&end_code, &end_ty, &start_ty)?;
                let iterator = self.fresh_local(variable);
                let limit = self.fresh_local("limit");
                let llvm = self.llvm_type(&start_ty)?;
                self.current.line(format!("{iterator} = alloca {llvm}"));
                self.current.line(format!("{limit} = alloca {llvm}"));
                self.current
                    .line(format!("store {llvm} {start_code}, ptr {iterator}"));
                self.current
                    .line(format!("store {llvm} {end_code}, ptr {limit}"));
                let cond_label = self.fresh_label("for.cond");
                let body_label = self.fresh_label("for.body");
                let end_label = self.fresh_label("for.end");
                self.current.terminate(format!("br label %{cond_label}"));
                self.push_block(cond_label.clone());
                let index = self.load_raw(&iterator, &start_ty);
                let bound = self.load_raw(&limit, &start_ty);
                let comparison = self.fresh("%cmp");
                self.current
                    .line(format!("{comparison} = icmp slt {llvm} {index}, {bound}"));
                self.current.terminate(format!(
                    "br i1 {comparison}, label %{body_label}, label %{end_label}"
                ));
                self.loops.push((cond_label.clone(), end_label.clone()));
                self.push_block(body_label);
                self.scopes.push(HashMap::new());
                self.bind(variable, iterator.clone(), start_ty.clone());
                self.emit_block_contents(body)?;
                self.scopes.pop();
                self.loops.pop();
                if self.current.terminator.is_none() {
                    let next = self.fresh("%next");
                    self.current.line(format!("{next} = add {llvm} {index}, 1"));
                    self.current
                        .line(format!("store {llvm} {next}, ptr {iterator}"));
                    self.current.terminate(format!("br label %{cond_label}"));
                }
                self.push_block(end_label);
            }
            StmtKind::Return(value) => match value {
                Some(expr) => {
                    let return_ty = self.current_return.clone();
                    let inferred = self.infer(expr, Some(&return_ty))?;
                    let code = self.gen_expr(expr, Some(&return_ty))?;
                    let code = self.coerce(&code, &inferred, &return_ty)?;
                    self.current
                        .terminate(format!("ret {} {code}", self.llvm_type(&return_ty)?));
                }
                None => {
                    if self.current_return != Ty::Void {
                        return Err("LLVM backend requires a value in this return".to_string());
                    }
                    self.current.terminate("ret void".to_string());
                }
            },
            StmtKind::Continue => {
                let (cond, _) = self
                    .loops
                    .last()
                    .ok_or_else(|| "LLVM backend found 'continue' outside a loop".to_string())?;
                self.current.terminate(format!("br label %{cond}"));
            }
            StmtKind::Break => {
                let (_, end) = self
                    .loops
                    .last()
                    .ok_or_else(|| "LLVM backend found 'break' outside a loop".to_string())?;
                self.current.terminate(format!("br label %{end}"));
            }
            StmtKind::Protect(block) => {
                let save = self.fresh("%arena.save");
                self.current
                    .line(format!("{save} = load ptr, ptr @loi_arena_ptr"));
                self.emit_block(block)?;
                self.current
                    .line(format!("store ptr {save}, ptr @loi_arena_ptr"));
            }
            StmtKind::Guard { .. } => {
                return Err("LLVM backend does not support 'guard' bindings".to_string());
            }
            StmtKind::ForEach { .. } => {
                return Err("LLVM backend does not support foreach loops".to_string());
            }
        }
        Ok(())
    }

    /// Emit the statements of an already-scoped block.
    fn emit_block_contents(&mut self, block: &ast::Block) -> Result<(), String> {
        for statement in &block.statements {
            if self.current.terminator.is_some() {
                break;
            }
            self.emit_stmt(statement)?;
        }
        Ok(())
    }

    // --------------------------------------------------------------
    // Expressions
    // --------------------------------------------------------------

    fn gen_expr(&mut self, expr: &Expr, expected: Option<&Ty>) -> Result<String, String> {
        match &expr.kind {
            ExprKind::Int(text) => {
                let ty = expected
                    .filter(|ty| ty.is_numeric())
                    .cloned()
                    .unwrap_or(Ty::I32)
                    .concretize();
                if ty.is_float() {
                    Ok(float_literal(text, &ty))
                } else {
                    Ok(int_literal(text)?)
                }
            }
            ExprKind::Float(text) => {
                let ty = expected
                    .filter(|ty| ty.is_float())
                    .cloned()
                    .unwrap_or(Ty::F64);
                Ok(float_literal(text, &ty))
            }
            ExprKind::Bool(value) => Ok(if *value { "true" } else { "false" }.to_string()),
            ExprKind::String(text) => {
                let name = self.string_constant(text);
                let literal_len = text.len() + 1;
                let ptr = self.fresh("%str");
                self.current.line(format!(
                    "{ptr} = getelementptr inbounds [{literal_len} x i8], ptr {name}, i64 0, i64 0"
                ));
                let slice = self.fresh("%str.slice");
                self.current.line(format!(
                    "{slice} = insertvalue %LoiSlice undef, ptr {ptr}, 0"
                ));
                let result = self.fresh("%str.slice.len");
                self.current.line(format!(
                    "{result} = insertvalue %LoiSlice {slice}, i64 {}, 1",
                    text.len()
                ));
                Ok(result)
            }
            ExprKind::Ident(name) => {
                if let Some(binding) = self.lookup(name) {
                    return Ok(self.load_binding(&binding));
                }
                Err(format!("LLVM backend could not resolve binding '{name}'"))
            }
            ExprKind::Unary { op, operand } => {
                let ty = self.infer(operand, expected)?.concretize();
                let value = self.gen_expr(operand, Some(&ty))?;
                let llvm = self.llvm_type(&ty)?;
                let result = self.fresh("%un");
                let code = match op {
                    UnaryOp::Neg if ty.is_float() => format!("{result} = fneg {llvm} {value}"),
                    UnaryOp::Neg => format!("{result} = sub {llvm} 0, {value}"),
                    UnaryOp::Not => format!("{result} = icmp eq {llvm} {value}, false"),
                    UnaryOp::BitNot => format!("{result} = xor {llvm} {value}, -1"),
                };
                self.current.line(code);
                Ok(result)
            }
            ExprKind::Binary { op, left, right } => {
                if *op == BinaryOp::Is {
                    return Err("LLVM backend does not support 'is' tests".to_string());
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    return self.gen_logical(*op, left, right);
                }
                let left_ty = self.infer(left, expected)?.concretize();
                let right_ty = self.infer(right, Some(&left_ty))?.concretize();
                let left = self.gen_expr(left, Some(&left_ty))?;
                let right = self.gen_expr(right, Some(&right_ty))?;
                self.binary_code(*op, &left, &right, &left_ty)
            }
            ExprKind::Cast { expr, ty } => {
                let target = self.ast_type(ty)?;
                let source = self.infer(expr, None)?;
                let value = self.gen_expr(expr, Some(&source))?;
                self.coerce(&value, &source, &target)
            }
            ExprKind::Call {
                callee,
                type_args,
                args,
                ..
            } => {
                if !type_args.is_empty() {
                    return Err(
                        "LLVM backend does not support unresolved generic declarations".to_string(),
                    );
                }
                let ExprKind::Ident(name) = &callee.kind else {
                    return Err(
                        "LLVM backend does not support indirect calls or function values"
                            .to_string(),
                    );
                };
                if SUPPORTED_INTRINSICS.contains(&name.as_str()) {
                    self.gen_intrinsic(name, args)
                } else {
                    let decl = *self.functions.get(name).ok_or_else(|| {
                        format!("LLVM backend could not resolve function '{name}'")
                    })?;
                    let mut values = Vec::new();
                    for (arg, param) in args.iter().zip(decl.params.iter()) {
                        let expected = self.ast_type(&param.ty)?;
                        let inferred = self.infer(&arg.value, Some(&expected))?;
                        let code = self.gen_expr(&arg.value, Some(&expected))?;
                        values.push((self.coerce(&code, &inferred, &expected)?, expected));
                    }
                    let rendered = values
                        .iter()
                        .map(|(value, ty)| {
                            format!("{} {value}", self.llvm_type(ty).unwrap_or_default())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let result = self.fresh("%call");
                    let ret = self.declared_return(decl);
                    if ret == Ty::Void {
                        self.current.line(format!(
                            "call void @{}({rendered})",
                            self.function_name(decl)
                        ));
                        Ok("void".to_string())
                    } else {
                        self.current.line(format!(
                            "{result} = call {} @{}({rendered})",
                            self.llvm_type(&ret)?,
                            self.function_name(decl)
                        ));
                        Ok(result)
                    }
                }
            }
            ExprKind::Move(inner) | ExprKind::Copy(inner) => self.gen_expr(inner, expected),
            ExprKind::SliceLiteral { elements, .. } => {
                let ty = self.infer(expr, expected)?;
                let Ty::Slice(element) = ty else {
                    return Err("LLVM backend expected a slice literal type".to_string());
                };
                let llvm_element = self.llvm_type(&element)?;
                let element_size = self.element_size(&element)?;
                let count = elements.len() as i64;
                if count == 0 {
                    return Ok("{ ptr null, i64 0 }".to_string());
                }
                let slice = self.fresh("%slice.literal");
                self.current.line(format!(
                    "{slice} = call %LoiSlice @loi_alloc_slice(i64 {count}, i64 {element_size}, i64 {}, i64 {})",
                    expr.span.line, expr.span.column
                ));
                let data = self.fresh("%slice.data");
                self.current
                    .line(format!("{data} = extractvalue %LoiSlice {slice}, 0"));
                for (index, value) in elements.iter().enumerate() {
                    let inferred = self.infer(value, Some(&element))?;
                    let code = self.gen_expr(value, Some(&element))?;
                    let code = self.coerce(&code, &inferred, &element)?;
                    let pointer = self.fresh("%slice.elem");
                    self.current.line(format!(
                        "{pointer} = getelementptr {llvm_element}, ptr {data}, i64 {index}"
                    ));
                    self.current
                        .line(format!("store {llvm_element} {code}, ptr {pointer}"));
                }
                Ok(slice)
            }
            ExprKind::InterfaceLiteral { .. } => {
                Err("LLVM backend does not support shape literals".to_string())
            }
            ExprKind::Alloc {
                target,
                element_type,
                count,
            } => {
                let ExprKind::Ident(name) = &target.kind else {
                    return Err("LLVM backend: 'alloc' requires a binding".to_string());
                };
                let binding = self
                    .lookup(name)
                    .ok_or_else(|| format!("LLVM backend could not resolve binding '{name}'"))?;
                let element = self.ast_type(element_type)?;
                let element_size = self.element_size(&element)?;
                let inferred = self.infer(count, Some(&Ty::U64))?;
                let count_code = self.gen_expr(count, Some(&Ty::U64))?;
                let count_code = self.coerce(&count_code, &inferred, &Ty::U64)?;
                let slice = self.fresh("%alloc.slice");
                self.current.line(format!(
                    "{slice} = call %LoiSlice @loi_alloc_slice(i64 {count_code}, i64 {element_size}, i64 {}, i64 {})",
                    expr.span.line, expr.span.column
                ));
                self.current
                    .line(format!("store %LoiSlice {slice}, ptr {}", binding.value));
                Ok("void".to_string())
            }
            ExprKind::Member { base, field } => {
                if field != "len" {
                    return Err("LLVM backend does not support field access".to_string());
                }
                let base_ty = self.infer(base, None)?;
                if !matches!(base_ty, Ty::Slice(_)) {
                    return Err("LLVM backend: 'len' requires a slice".to_string());
                }
                let base_code = self.gen_expr(base, Some(&base_ty))?;
                let length = self.fresh("%len");
                self.current
                    .line(format!("{length} = extractvalue %LoiSlice {base_code}, 1"));
                Ok(length)
            }
            ExprKind::Index { base, index } => {
                let base_ty = self.infer(base, None)?;
                let Ty::Slice(ref element) = base_ty else {
                    return Err("LLVM backend: indexing requires a slice".to_string());
                };
                let base_code = self.gen_expr(base, Some(&base_ty))?;
                let inferred = self.infer(index, Some(&Ty::U64))?;
                let index_code = self.gen_expr(index, Some(&Ty::U64))?;
                let index_code = self.coerce(&index_code, &inferred, &Ty::U64)?;
                let element_size = self.element_size(element)?;
                let pointer = self.fresh("%index.ptr");
                self.current.line(format!(
                    "{pointer} = call ptr @loi_index(%LoiSlice {base_code}, i64 {index_code}, i64 {element_size}, i64 {}, i64 {})",
                    expr.span.line, expr.span.column
                ));
                let llvm_element = self.llvm_type(element)?;
                let result = self.fresh("%index.load");
                self.current
                    .line(format!("{result} = load {llvm_element}, ptr {pointer}"));
                Ok(result)
            }
            ExprKind::Comptime(_) => {
                Err("LLVM backend does not support 'comptime' expressions".to_string())
            }
        }
    }

    fn gen_logical(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Result<String, String> {
        let left = self.gen_expr(left, Some(&Ty::Bool))?;
        let rhs_label = self.fresh_label("logic.rhs");
        let end_label = self.fresh_label("logic.end");
        let result = self.fresh("%logic");
        self.current.line(format!("{result} = alloca i1"));
        self.current.line(format!("store i1 {left}, ptr {result}"));
        if op == BinaryOp::And {
            self.current.terminate(format!(
                "br i1 {left}, label %{rhs_label}, label %{end_label}"
            ));
        } else {
            self.current.terminate(format!(
                "br i1 {left}, label %{end_label}, label %{rhs_label}"
            ));
        }
        self.push_block(rhs_label);
        let right = self.gen_expr(right, Some(&Ty::Bool))?;
        self.current.line(format!("store i1 {right}, ptr {result}"));
        self.current.terminate(format!("br label %{end_label}"));
        self.push_block(end_label);
        let loaded = self.fresh("%logic.load");
        self.current
            .line(format!("{loaded} = load i1, ptr {result}"));
        Ok(loaded)
    }

    fn gen_intrinsic(&mut self, name: &str, args: &[CallArg]) -> Result<String, String> {
        match name {
            "print" | "eprint" => {
                let arg = args
                    .first()
                    .ok_or_else(|| format!("LLVM backend: '{name}' requires a slice"))?;
                let slice = self.gen_expr(&arg.value, None)?;
                let data = self.fresh("%write.data");
                let length = self.fresh("%write.len");
                self.current
                    .line(format!("{data} = extractvalue %LoiSlice {slice}, 0"));
                self.current
                    .line(format!("{length} = extractvalue %LoiSlice {slice}, 1"));
                let stream = self.load_stream(if name == "print" {
                    "@stdout"
                } else {
                    "@stderr"
                });
                self.current.line(format!(
                    "call i32 @fwrite(ptr {data}, i64 1, i64 {length}, ptr {stream})"
                ));
                Ok("void".to_string())
            }
            "panic" => {
                let arg = args
                    .first()
                    .ok_or_else(|| "LLVM backend: 'panic' requires a slice".to_string())?;
                let slice = self.gen_expr(&arg.value, None)?;
                let data = self.fresh("%panic.data");
                let length = self.fresh("%panic.len");
                self.current
                    .line(format!("{data} = extractvalue %LoiSlice {slice}, 0"));
                self.current
                    .line(format!("{length} = extractvalue %LoiSlice {slice}, 1"));
                let stream = self.load_stream("@stderr");
                self.current.line(format!(
                    "call i32 @fwrite(ptr {data}, i64 1, i64 {length}, ptr {stream})"
                ));
                self.current.line("call void @exit(i32 1)".to_string());
                Ok("void".to_string())
            }
            "len" => {
                let arg = args
                    .first()
                    .ok_or_else(|| "LLVM backend: 'len' requires a slice".to_string())?;
                let slice = self.gen_expr(&arg.value, None)?;
                let length = self.fresh("%len");
                self.current
                    .line(format!("{length} = extractvalue %LoiSlice {slice}, 1"));
                Ok(length)
            }
            "view" => {
                let source = args
                    .first()
                    .ok_or_else(|| "LLVM backend: 'view' requires three arguments".to_string())?;
                args.get(2)
                    .ok_or_else(|| "LLVM backend: 'view' requires three arguments".to_string())?;
                let source_ty = self.infer(&source.value, None)?;
                let Ty::Slice(ref element) = source_ty else {
                    return Err("LLVM backend: 'view' requires a slice".to_string());
                };
                let element_size = self.element_size(element)?;
                let source_code = self.gen_expr(&source.value, Some(&source_ty))?;
                let mut values = Vec::new();
                for arg in args.iter().skip(1).take(2) {
                    let inferred = self.infer(&arg.value, Some(&Ty::U64))?;
                    let code = self.gen_expr(&arg.value, Some(&Ty::U64))?;
                    values.push(self.coerce(&code, &inferred, &Ty::U64)?);
                }
                let result = self.fresh("%view.slice");
                self.current.line(format!(
                    "{result} = call %LoiSlice @loi_view(%LoiSlice {source_code}, i64 {}, i64 {}, i64 {element_size}, i64 {}, i64 {})",
                    values[0], values[1], source.value.span.line, source.value.span.column
                ));
                Ok(result)
            }
            "print_int" => {
                let arg = args
                    .first()
                    .ok_or_else(|| "LLVM backend: 'print_int' requires a value".to_string())?;
                let inferred = self.infer(&arg.value, Some(&Ty::I64))?;
                let value = self.gen_expr(&arg.value, Some(&Ty::I64))?;
                let value = self.coerce(&value, &inferred, &Ty::I64)?;
                let fmt = self.gep_string("%lld");
                self.current.line(format!(
                    "call i32 (ptr, ...) @printf(ptr {fmt}, i64 {value})"
                ));
                Ok("void".to_string())
            }
            "print_float" => {
                let arg = args
                    .first()
                    .ok_or_else(|| "LLVM backend: 'print_float' requires a value".to_string())?;
                let inferred = self.infer(&arg.value, Some(&Ty::F64))?;
                let value = self.gen_expr(&arg.value, Some(&Ty::F64))?;
                let value = self.coerce(&value, &inferred, &Ty::F64)?;
                self.uses_float_helper = true;
                self.current
                    .line(format!("call void @loi_print_float(double {value})"));
                Ok("void".to_string())
            }
            "sqrt" | "floor" | "ceil" | "fabs" | "pow" => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.gen_expr(&arg.value, Some(&Ty::F64))?);
                }
                self.uses_math.insert(name.to_string());
                let result = self.fresh("%math");
                let operands = values
                    .iter()
                    .map(|value| format!("double {value}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.current
                    .line(format!("{result} = call double @{name}({operands})"));
                Ok(result)
            }
            _ => Err(format!("LLVM backend does not support intrinsic '{name}'")),
        }
    }

    // --------------------------------------------------------------
    // Type and value helpers
    // --------------------------------------------------------------

    fn ast_type(&self, ty: &Type) -> Result<Ty, String> {
        match ty {
            Type::Named(name, _) => {
                if name == "void" {
                    Ok(Ty::Void)
                } else if let Some(ty) = Ty::from_name(name) {
                    Ok(ty)
                } else {
                    self.validate_type(ty)?;
                    Ok(Ty::Error)
                }
            }
            Type::Slice(element, _) => {
                self.validate_slice_element(element)?;
                Ok(Ty::Slice(Box::new(self.ast_type(element)?)))
            }
            Type::Generic(_, _, _) => self.validate_type(ty).map(|()| Ty::Error),
            Type::Callback { .. } => self.validate_type(ty).map(|()| Ty::Error),
        }
    }

    fn llvm_type(&self, ty: &Ty) -> Result<String, String> {
        match ty {
            Ty::I8 | Ty::U8 => Ok("i8".to_string()),
            Ty::I16 | Ty::U16 => Ok("i16".to_string()),
            Ty::I32 | Ty::U32 | Ty::IntLiteral => Ok("i32".to_string()),
            Ty::I64 | Ty::U64 => Ok("i64".to_string()),
            Ty::Bool => Ok("i1".to_string()),
            Ty::F32 => Ok("float".to_string()),
            Ty::F64 | Ty::FloatLiteral => Ok("double".to_string()),
            Ty::Void => Ok("void".to_string()),
            Ty::Slice(_) => Ok("%LoiSlice".to_string()),
            other => Err(format!("LLVM backend does not support type '{other}'")),
        }
    }

    fn zero_value(&self, ty: &Ty) -> Result<String, String> {
        match ty {
            Ty::Bool => Ok("false".to_string()),
            Ty::F32 | Ty::F64 | Ty::FloatLiteral => Ok("0.0".to_string()),
            Ty::Slice(_) => Ok("{ ptr null, i64 0 }".to_string()),
            _ if ty.is_integer() => Ok("0".to_string()),
            other => Err(format!(
                "LLVM backend cannot make a zero value for '{other}'"
            )),
        }
    }

    /// Byte size of a slice element. Only scalar elements are supported by the
    /// LLVM slice ABI, so the sizes are static and match the C backend.
    fn element_size(&self, ty: &Ty) -> Result<i64, String> {
        let size = match ty {
            Ty::Bool | Ty::I8 | Ty::U8 => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::I32 | Ty::U32 | Ty::IntLiteral | Ty::F32 => 4,
            Ty::I64 | Ty::U64 | Ty::F64 | Ty::FloatLiteral => 8,
            other => {
                return Err(format!(
                    "LLVM backend does not support slice element type '{other}'"
                ));
            }
        };
        Ok(size)
    }

    fn declared_return(&self, decl: &FunctionDecl) -> Ty {
        match &decl.return_type {
            Some(ty) => self.ast_type(ty).unwrap_or(Ty::Void),
            None => Ty::Void,
        }
    }

    /// `main` always has a C-compatible `i32` result, even when the Loi flow
    /// omits its return type; the driver then returns zero.
    fn signature_return(&self, decl: &FunctionDecl) -> Ty {
        let declared = self.declared_return(decl);
        if decl.name == "main" && declared == Ty::Void {
            Ty::I32
        } else {
            declared
        }
    }

    fn function_name(&self, decl: &FunctionDecl) -> String {
        if decl.name == "main" {
            return "main".to_string();
        }
        if decl.external {
            return decl
                .link_name
                .clone()
                .unwrap_or_else(|| sanitize(&decl.name));
        }
        format!("loi_fn_{}", sanitize(&decl.name))
    }

    fn binary_code(
        &mut self,
        op: BinaryOp,
        left: &str,
        right: &str,
        ty: &Ty,
    ) -> Result<String, String> {
        if matches!(ty, Ty::Slice(_)) {
            if !matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                return Err(format!(
                    "LLVM backend cannot lower operator '{op:?}' on slices"
                ));
            }
            let equal = self.fresh("%slice.eq");
            self.current.line(format!(
                "{equal} = call i1 @loi_slice_eq(%LoiSlice {left}, %LoiSlice {right})"
            ));
            if op == BinaryOp::Eq {
                return Ok(equal);
            }
            let result = self.fresh("%slice.neq");
            self.current
                .line(format!("{result} = xor i1 {equal}, true"));
            return Ok(result);
        }
        let llvm = self.llvm_type(ty)?;
        let result = self.fresh("%bin");
        let signed = is_signed(ty);
        let code = match op {
            BinaryOp::Add if ty.is_float() => format!("{result} = fadd {llvm} {left}, {right}"),
            BinaryOp::Add => format!("{result} = add {llvm} {left}, {right}"),
            BinaryOp::Sub if ty.is_float() => format!("{result} = fsub {llvm} {left}, {right}"),
            BinaryOp::Sub => format!("{result} = sub {llvm} {left}, {right}"),
            BinaryOp::Mul if ty.is_float() => format!("{result} = fmul {llvm} {left}, {right}"),
            BinaryOp::Mul => format!("{result} = mul {llvm} {left}, {right}"),
            BinaryOp::Div if ty.is_float() => format!("{result} = fdiv {llvm} {left}, {right}"),
            BinaryOp::Div if signed => format!("{result} = sdiv {llvm} {left}, {right}"),
            BinaryOp::Div => format!("{result} = udiv {llvm} {left}, {right}"),
            BinaryOp::Mod if ty.is_float() => format!("{result} = frem {llvm} {left}, {right}"),
            BinaryOp::Mod if signed => format!("{result} = srem {llvm} {left}, {right}"),
            BinaryOp::Mod => format!("{result} = urem {llvm} {left}, {right}"),
            BinaryOp::BitOr => format!("{result} = or {llvm} {left}, {right}"),
            BinaryOp::BitAnd => format!("{result} = and {llvm} {left}, {right}"),
            BinaryOp::BitXor => format!("{result} = xor {llvm} {left}, {right}"),
            BinaryOp::Shl => format!("{result} = shl {llvm} {left}, {right}"),
            BinaryOp::Shr if signed => format!("{result} = ashr {llvm} {left}, {right}"),
            BinaryOp::Shr => format!("{result} = lshr {llvm} {left}, {right}"),
            BinaryOp::Eq if ty.is_float() => format!("{result} = fcmp oeq {llvm} {left}, {right}"),
            BinaryOp::Eq => format!("{result} = icmp eq {llvm} {left}, {right}"),
            BinaryOp::Neq if ty.is_float() => format!("{result} = fcmp one {llvm} {left}, {right}"),
            BinaryOp::Neq => format!("{result} = icmp ne {llvm} {left}, {right}"),
            BinaryOp::Lt if ty.is_float() => format!("{result} = fcmp olt {llvm} {left}, {right}"),
            BinaryOp::Lt if signed => format!("{result} = icmp slt {llvm} {left}, {right}"),
            BinaryOp::Lt => format!("{result} = icmp ult {llvm} {left}, {right}"),
            BinaryOp::Lte if ty.is_float() => format!("{result} = fcmp ole {llvm} {left}, {right}"),
            BinaryOp::Lte if signed => format!("{result} = icmp sle {llvm} {left}, {right}"),
            BinaryOp::Lte => format!("{result} = icmp ule {llvm} {left}, {right}"),
            BinaryOp::Gt if ty.is_float() => format!("{result} = fcmp ogt {llvm} {left}, {right}"),
            BinaryOp::Gt if signed => format!("{result} = icmp sgt {llvm} {left}, {right}"),
            BinaryOp::Gt => format!("{result} = icmp ugt {llvm} {left}, {right}"),
            BinaryOp::Gte if ty.is_float() => format!("{result} = fcmp oge {llvm} {left}, {right}"),
            BinaryOp::Gte if signed => format!("{result} = icmp sge {llvm} {left}, {right}"),
            BinaryOp::Gte => format!("{result} = icmp uge {llvm} {left}, {right}"),
            BinaryOp::And | BinaryOp::Or | BinaryOp::Is => {
                return Err(format!("LLVM backend cannot lower operator '{op:?}' here"));
            }
        };
        self.current.line(code);
        Ok(result)
    }

    fn coerce(&mut self, value: &str, from: &Ty, to: &Ty) -> Result<String, String> {
        let from = from.concretize();
        let to = to.concretize();
        if from == to || to == Ty::Error || from == Ty::Error {
            return Ok(value.to_string());
        }
        let result = self.fresh("%cast");
        let from_llvm = self.llvm_type(&from)?;
        let to_llvm = self.llvm_type(&to)?;
        // Signedness is a source-level distinction only: two integer types
        // with the same LLVM representation need no instruction at all.
        if from_llvm == to_llvm {
            return Ok(value.to_string());
        }
        let instruction = match (&from, &to) {
            (Ty::Bool, _) if to.is_integer() => {
                format!("{result} = zext {from_llvm} {value} to {to_llvm}")
            }
            (Ty::Bool, _) if to.is_float() => {
                format!("{result} = sitofp {from_llvm} {value} to {to_llvm}")
            }
            (_, Ty::Bool) if from.is_integer() => {
                format!("{result} = icmp ne {from_llvm} {value}, 0")
            }
            _ if from.is_integer() && to.is_integer() => {
                let width = |ty: &Ty| -> u32 {
                    match ty {
                        Ty::I8 | Ty::U8 => 8,
                        Ty::I16 | Ty::U16 => 16,
                        Ty::I32 | Ty::U32 => 32,
                        Ty::I64 | Ty::U64 => 64,
                        _ => 32,
                    }
                };
                if width(&from) > width(&to) {
                    format!("{result} = trunc {from_llvm} {value} to {to_llvm}")
                } else if is_signed(&from) {
                    format!("{result} = sext {from_llvm} {value} to {to_llvm}")
                } else {
                    format!("{result} = zext {from_llvm} {value} to {to_llvm}")
                }
            }
            _ if from.is_integer() && to.is_float() => {
                if is_signed(&from) {
                    format!("{result} = sitofp {from_llvm} {value} to {to_llvm}")
                } else {
                    format!("{result} = uitofp {from_llvm} {value} to {to_llvm}")
                }
            }
            _ if from.is_float() && to.is_integer() => {
                if is_signed(&to) {
                    format!("{result} = fptosi {from_llvm} {value} to {to_llvm}")
                } else {
                    format!("{result} = fptoui {from_llvm} {value} to {to_llvm}")
                }
            }
            _ if from.is_float() && to.is_float() => {
                format!("{result} = fpext {from_llvm} {value} to {to_llvm}")
            }
            _ => {
                return Err(format!("LLVM backend cannot convert '{from}' to '{to}'"));
            }
        };
        self.current.line(instruction);
        Ok(result)
    }

    fn infer(&self, expr: &Expr, expected: Option<&Ty>) -> Result<Ty, String> {
        match &expr.kind {
            ExprKind::Int(_) => Ok(expected
                .filter(|ty| ty.is_numeric())
                .cloned()
                .unwrap_or(Ty::I32)
                .concretize()),
            ExprKind::Float(_) => Ok(expected
                .filter(|ty| ty.is_float())
                .cloned()
                .unwrap_or(Ty::F64)),
            ExprKind::Bool(_) => Ok(Ty::Bool),
            ExprKind::String(_) => Ok(Ty::Slice(Box::new(Ty::U8))),
            ExprKind::Ident(name) => self
                .lookup(name)
                .map(|binding| binding.ty.clone())
                .ok_or_else(|| format!("LLVM backend could not resolve binding '{name}'")),
            ExprKind::Unary { operand, .. } => self.infer(operand, expected),
            ExprKind::Binary { op, left, right } => {
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Neq
                        | BinaryOp::Lt
                        | BinaryOp::Lte
                        | BinaryOp::Gt
                        | BinaryOp::Gte
                        | BinaryOp::And
                        | BinaryOp::Or
                        | BinaryOp::Is
                ) {
                    return Ok(Ty::Bool);
                }
                let left = self.infer(left, expected)?.concretize();
                let right = self.infer(right, Some(&left))?.concretize();
                Ok(types::common_numeric(&left, &right).unwrap_or(left))
            }
            ExprKind::Cast { ty, .. } => self.ast_type(ty),
            ExprKind::Call { callee, args, .. } => {
                let ExprKind::Ident(name) = &callee.kind else {
                    return Err("LLVM backend cannot infer an indirect call".to_string());
                };
                if SUPPORTED_INTRINSICS.contains(&name.as_str()) {
                    Ok(match name.as_str() {
                        "sqrt" | "floor" | "ceil" | "fabs" | "pow" => Ty::F64,
                        "len" => Ty::U64,
                        "view" => {
                            let source = args.first().ok_or_else(|| {
                                "LLVM backend: 'view' requires three arguments".to_string()
                            })?;
                            match self.infer(&source.value, None)? {
                                Ty::Slice(element) => Ty::Slice(element),
                                _ => {
                                    return Err("LLVM backend: 'view' requires a slice".to_string());
                                }
                            }
                        }
                        _ => Ty::Void,
                    })
                } else {
                    self.functions
                        .get(name)
                        .map(|decl| self.declared_return(decl))
                        .ok_or_else(|| format!("LLVM backend could not resolve function '{name}'"))
                }
            }
            ExprKind::Move(inner) | ExprKind::Copy(inner) => self.infer(inner, expected),
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                let element = match element_type {
                    Some(ty) => self.ast_type(ty)?,
                    None => match elements.first() {
                        Some(first) => self.infer(first, None)?.concretize(),
                        None => Ty::I32,
                    },
                };
                Ok(Ty::Slice(Box::new(element)))
            }
            ExprKind::InterfaceLiteral { .. } => {
                Err("LLVM backend does not support shape literals".to_string())
            }
            ExprKind::Alloc { .. } => Ok(Ty::Void),
            ExprKind::Member { base, field } => {
                if field != "len" {
                    return Err("LLVM backend does not support field access".to_string());
                }
                if matches!(self.infer(base, None)?, Ty::Slice(_)) {
                    Ok(Ty::U64)
                } else {
                    Err("LLVM backend: 'len' requires a slice".to_string())
                }
            }
            ExprKind::Index { base, .. } => match self.infer(base, None)? {
                Ty::Slice(element) => Ok(*element),
                _ => Err("LLVM backend: indexing requires a slice".to_string()),
            },
            ExprKind::Comptime(_) => {
                Err("LLVM backend does not support 'comptime' expressions".to_string())
            }
        }
    }

    fn string_constant(&mut self, text: &str) -> String {
        if let Some(name) = self.string_index.get(text) {
            return name.clone();
        }
        let name = format!("@.str{}", self.strings.len());
        let literal = format!("[{} x i8] c\"{}\"", text.len() + 1, escape(text));
        self.strings.push((name.clone(), literal));
        self.string_index.insert(text.to_string(), name.clone());
        name
    }

    fn gep_string(&mut self, text: &str) -> String {
        self.string_constant(text);
        let length = text.len() + 1;
        let name = self
            .string_index
            .get(text)
            .expect("string was just interned")
            .clone();
        let ptr = self.fresh("%fmt");
        self.current.line(format!(
            "{ptr} = getelementptr inbounds [{length} x i8], ptr {name}, i64 0, i64 0"
        ));
        ptr
    }

    fn bind(&mut self, name: &str, value: String, ty: Ty) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), Binding { value, ty });
        }
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }

    /// Address of an assignable place. Slices are passed as `{ ptr, i64 }`
    /// values, so an index assignment needs a checked element pointer first.
    fn gen_place(&mut self, expr: &Expr) -> Result<(String, Ty), String> {
        match &expr.kind {
            ExprKind::Ident(name) => {
                let binding = self
                    .lookup(name)
                    .ok_or_else(|| format!("LLVM backend could not resolve binding '{name}'"))?;
                Ok((binding.value, binding.ty))
            }
            ExprKind::Index { base, index } => {
                let base_ty = self.infer(base, None)?;
                let Ty::Slice(ref element) = base_ty else {
                    return Err("LLVM backend: indexing requires a slice".to_string());
                };
                let base_code = self.gen_expr(base, Some(&base_ty))?;
                let inferred = self.infer(index, Some(&Ty::U64))?;
                let index_code = self.gen_expr(index, Some(&Ty::U64))?;
                let index_code = self.coerce(&index_code, &inferred, &Ty::U64)?;
                let element_size = self.element_size(element)?;
                let pointer = self.fresh("%place.ptr");
                self.current.line(format!(
                    "{pointer} = call ptr @loi_index(%LoiSlice {base_code}, i64 {index_code}, i64 {element_size}, i64 {}, i64 {})",
                    expr.span.line, expr.span.column
                ));
                Ok((pointer, element.as_ref().clone()))
            }
            _ => Err(
                "LLVM backend only supports assignment to a binding or slice element".to_string(),
            ),
        }
    }

    fn load_binding(&mut self, binding: &Binding) -> String {
        let pointer = binding.value.clone();
        self.load_raw(&pointer, &binding.ty)
    }

    /// `stdout`/`stderr` are globals holding a `FILE*`, so the pointer must be
    /// loaded before it is passed to `fwrite`/`fprintf`.
    fn load_stream(&mut self, global: &str) -> String {
        let stream = self.fresh("%stream");
        self.current
            .line(format!("{stream} = load ptr, ptr {global}"));
        stream
    }

    fn load_raw(&mut self, pointer: &str, ty: &Ty) -> String {
        let result = self.fresh("%load");
        let value = self.llvm_type(ty).unwrap_or_else(|_| "i32".to_string());
        self.current
            .line(format!("{result} = load {value}, ptr {pointer}"));
        result
    }

    fn push_block(&mut self, label: String) {
        let finished = std::mem::replace(&mut self.current, BasicBlock::new(label));
        self.finished.push(finished);
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{}{}", prefix, self.counter)
    }

    fn fresh_local(&mut self, name: &str) -> String {
        self.counter += 1;
        format!("%{}.{}", sanitize(name), self.counter)
    }

    fn fresh_label(&mut self, name: &str) -> String {
        self.counter += 1;
        format!("{}.{}", name, self.counter)
    }
}

const SUPPORTED_INTRINSICS: &[&str] = &[
    "print",
    "eprint",
    "panic",
    "print_int",
    "print_float",
    "len",
    "view",
    "sqrt",
    "floor",
    "ceil",
    "fabs",
    "pow",
];

fn is_signed(ty: &Ty) -> bool {
    matches!(ty, Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 | Ty::IntLiteral)
}

fn op_to_binary(op: AssignOp) -> BinaryOp {
    match op {
        AssignOp::Add => BinaryOp::Add,
        AssignOp::Sub => BinaryOp::Sub,
        AssignOp::Mul => BinaryOp::Mul,
        AssignOp::Div => BinaryOp::Div,
        AssignOp::Mod => BinaryOp::Mod,
        AssignOp::BitAnd => BinaryOp::BitAnd,
        AssignOp::BitOr => BinaryOp::BitOr,
        AssignOp::BitXor => BinaryOp::BitXor,
        AssignOp::Shl => BinaryOp::Shl,
        AssignOp::Shr => BinaryOp::Shr,
        AssignOp::Assign => BinaryOp::Add,
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn int_literal(text: &str) -> Result<String, String> {
    let text = text.replace('_', "");
    let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u128::from_str_radix(hex, 16)
    } else if let Some(binary) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        u128::from_str_radix(binary, 2)
    } else {
        text.parse::<u128>()
    }
    .map_err(|_| format!("LLVM backend could not parse integer literal '{text}'"))?;
    Ok(value.to_string())
}

fn float_literal(text: &str, ty: &Ty) -> String {
    let text = text.replace('_', "");
    let parsed = if *ty == Ty::F32 {
        text.parse::<f32>()
            .map(|value| format!("{value:?}"))
            .unwrap_or(text.clone())
    } else {
        text.parse::<f64>()
            .map(|value| format!("{value:?}"))
            .unwrap_or(text.clone())
    };
    if parsed.contains('.') || parsed.contains('e') || parsed.contains('E') {
        parsed
    } else {
        format!("{parsed}.0")
    }
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        match byte {
            b'"' => out.push_str("\\22"),
            b'\\' => out.push_str("\\5C"),
            b'\n' => out.push_str("\\0A"),
            b'\r' => out.push_str("\\0D"),
            b'\t' => out.push_str("\\09"),
            0x20..=0x7E => out.push(*byte as char),
            other => out.push_str(&format!("\\{other:02X}")),
        }
    }
    out.push_str("\\00");
    out
}

fn declare_float_helper_decls() -> String {
    "@.loi_float_fmt = internal constant [5 x i8] c\"%.*g\\00\"\ndeclare i32 @snprintf(ptr, i64, ptr, ...)\n".to_string()
}

const LOI_PRINT_FLOAT_HELPER: &str = r#"define void @loi_print_float(double %value) {
entry:
  %buf = alloca [32 x i8]
  %precision = alloca i32
  store i32 1, ptr %precision
  br label %loop

loop:
  %prec = load i32, ptr %precision
  %within = icmp ule i32 %prec, 17
  br i1 %within, label %body, label %fallback

body:
  %fmt = getelementptr inbounds [5 x i8], ptr @.loi_float_fmt, i64 0, i64 0
  %bufptr = getelementptr inbounds [32 x i8], ptr %buf, i64 0, i64 0
  %written = call i32 (ptr, i64, ptr, ...) @snprintf(ptr %bufptr, i64 32, ptr %fmt, i32 %prec, double %value)
  %parsed = call double @strtod(ptr %bufptr, ptr null)
  %same = fcmp oeq double %parsed, %value
  br i1 %same, label %done, label %next

next:
  %incremented = add i32 %prec, 1
  store i32 %incremented, ptr %precision
  br label %loop

fallback:
  %final = getelementptr inbounds [32 x i8], ptr %buf, i64 0, i64 0
  %stdout = load ptr, ptr @stdout
  call i32 @fputs(ptr %final, ptr %stdout)
  ret void

done:
  %donebuf = getelementptr inbounds [32 x i8], ptr %buf, i64 0, i64 0
  %donestream = load ptr, ptr @stdout
  call i32 @fputs(ptr %donebuf, ptr %donestream)
  ret void
}

declare double @strtod(ptr, ptr)
"#;

/// Runtime helpers for the slice ABI. They mirror the C11 contracts: a bump
/// arena with a restorable top, checked indexing and byte-wise comparison.
/// `protect` saves and restores `@loi_arena_ptr` from generated code.
const LOI_SLICE_RUNTIME: &str = r#"
declare i32 @fprintf(ptr, ptr, ...)
declare void @exit(i32)
declare ptr @malloc(i64)
declare i32 @memcmp(ptr, ptr, i64)

@loi_arena_ptr = internal global ptr null
@loi_arena_end = internal global ptr null
@.loi_fail_fmt = internal constant [20 x i8] c"loi: %s at %zu:%zu\0A\00"
@.loi_arena_fail = internal constant [24 x i8] c"arena allocation failed\00"
@.loi_slice_alloc_fail = internal constant [26 x i8] c"slice allocation overflow\00"
@.loi_index_fail = internal constant [26 x i8] c"slice index out of bounds\00"
@.loi_view_fail = internal constant [25 x i8] c"view range out of bounds\00"

define void @loi_runtime_fail(ptr %message, i64 %line, i64 %column) {
entry:
  %fmt = getelementptr inbounds [20 x i8], ptr @.loi_fail_fmt, i64 0, i64 0
  %stream = load ptr, ptr @stderr
  %result = call i32 (ptr, ptr, ...) @fprintf(ptr %stream, ptr %fmt, ptr %message, i64 %line, i64 %column)
  call void @exit(i32 1)
  unreachable
}

define ptr @loi_alloc(i64 %size, i64 %line, i64 %column) {
entry:
  %empty = icmp eq i64 %size, 0
  %minimum = select i1 %empty, i64 8, i64 %size
  %rounded = add i64 %minimum, 7
  %aligned = and i64 %rounded, -8
  %current = load ptr, ptr @loi_arena_ptr
  %end = load ptr, ptr @loi_arena_end
  %current_int = ptrtoint ptr %current to i64
  %end_int = ptrtoint ptr %end to i64
  %remaining = sub i64 %end_int, %current_int
  %fits = icmp uge i64 %remaining, %aligned
  br i1 %fits, label %reuse, label %grow

reuse:
  %next = getelementptr i8, ptr %current, i64 %aligned
  store ptr %next, ptr @loi_arena_ptr
  ret ptr %current

grow:
  %small = icmp ult i64 %aligned, 65536
  %chunk_size = select i1 %small, i64 65536, i64 %aligned
  %memory = call ptr @malloc(i64 %chunk_size)
  %failed = icmp eq ptr %memory, null
  br i1 %failed, label %fail, label %install

fail:
  %message = getelementptr inbounds [24 x i8], ptr @.loi_arena_fail, i64 0, i64 0
  call void @loi_runtime_fail(ptr %message, i64 %line, i64 %column)
  unreachable

install:
  %finish = getelementptr i8, ptr %memory, i64 %chunk_size
  %after = getelementptr i8, ptr %memory, i64 %aligned
  store ptr %finish, ptr @loi_arena_end
  store ptr %after, ptr @loi_arena_ptr
  ret ptr %memory
}

define %LoiSlice @loi_alloc_slice(i64 %count, i64 %element_size, i64 %line, i64 %column) {
entry:
  %has_element = icmp ne i64 %element_size, 0
  br i1 %has_element, label %check, label %allocate

check:
  %limit = udiv i64 -1, %element_size
  %overflow = icmp ugt i64 %count, %limit
  br i1 %overflow, label %fail, label %allocate

fail:
  %message = getelementptr inbounds [26 x i8], ptr @.loi_slice_alloc_fail, i64 0, i64 0
  call void @loi_runtime_fail(ptr %message, i64 %line, i64 %column)
  unreachable

allocate:
  %bytes = mul i64 %count, %element_size
  %data = call ptr @loi_alloc(i64 %bytes, i64 %line, i64 %column)
  %with_data = insertvalue %LoiSlice undef, ptr %data, 0
  %result = insertvalue %LoiSlice %with_data, i64 %count, 1
  ret %LoiSlice %result
}

define ptr @loi_index(%LoiSlice %slice, i64 %index, i64 %element_size, i64 %line, i64 %column) {
entry:
  %length = extractvalue %LoiSlice %slice, 1
  %outside = icmp uge i64 %index, %length
  br i1 %outside, label %fail, label %ok

fail:
  %message = getelementptr inbounds [26 x i8], ptr @.loi_index_fail, i64 0, i64 0
  call void @loi_runtime_fail(ptr %message, i64 %line, i64 %column)
  unreachable

ok:
  %data = extractvalue %LoiSlice %slice, 0
  %offset = mul i64 %index, %element_size
  %pointer = getelementptr i8, ptr %data, i64 %offset
  ret ptr %pointer
}

define i1 @loi_slice_eq(%LoiSlice %left, %LoiSlice %right) {
entry:
  %left_len = extractvalue %LoiSlice %left, 1
  %right_len = extractvalue %LoiSlice %right, 1
  %same = icmp eq i64 %left_len, %right_len
  br i1 %same, label %bytes, label %false

bytes:
  %empty = icmp eq i64 %left_len, 0
  br i1 %empty, label %true, label %compare

compare:
  %left_data = extractvalue %LoiSlice %left, 0
  %right_data = extractvalue %LoiSlice %right, 0
  %cmp = call i32 @memcmp(ptr %left_data, ptr %right_data, i64 %left_len)
  %equal = icmp eq i32 %cmp, 0
  br label %done

true:
  br label %done

false:
  br label %done

done:
  %result = phi i1 [ false, %false ], [ true, %true ], [ %equal, %compare ]
  ret i1 %result
}

define %LoiSlice @loi_view(%LoiSlice %source, i64 %start, i64 %end, i64 %element_size, i64 %line, i64 %column) {
entry:
  %length = extractvalue %LoiSlice %source, 1
  %ordered = icmp ule i64 %start, %end
  %bounded = icmp ule i64 %end, %length
  %valid = and i1 %ordered, %bounded
  br i1 %valid, label %ok, label %fail

fail:
  %message = getelementptr inbounds [25 x i8], ptr @.loi_view_fail, i64 0, i64 0
  call void @loi_runtime_fail(ptr %message, i64 %line, i64 %column)
  unreachable

ok:
  %data = extractvalue %LoiSlice %source, 0
  %skip = mul i64 %start, %element_size
  %pointer = getelementptr i8, ptr %data, i64 %skip
  %view_len = sub i64 %end, %start
  %with_data = insertvalue %LoiSlice undef, ptr %pointer, 0
  %result = insertvalue %LoiSlice %with_data, i64 %view_len, 1
  ret %LoiSlice %result
}
"#;
