//! Opt-in LLVM IR backend for the supported scalar subset of Loi.
//!
//! This is deliberately separate from `ir.rs`, whose textual output is an
//! early draft and not valid LLVM IR. The emitter works from the checked,
//! monomorphized AST and produces textual IR with opaque pointers so it can be
//! compiled directly by `clang -x ir`. The C11 backend remains the default and
//! the production path; unsupported language features are rejected with a
//! diagnostic instead of being lowered incorrectly.

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
            Type::Slice(_, _) => Err("LLVM backend does not support slice types".to_string()),
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
                if let Some(value) = value {
                    self.validate_expr(value)?;
                }
            }
            StmtKind::Assign { target, value, .. } => {
                self.validate_expr(target)?;
                self.validate_expr(value)?;
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
            StmtKind::Protect(_) => {
                return Err("LLVM backend does not support 'protect' blocks".to_string());
            }
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
            ExprKind::SliceLiteral { .. } => {
                return Err("LLVM backend does not support slice literals".to_string());
            }
            ExprKind::InterfaceLiteral { .. } => {
                return Err("LLVM backend does not support shape literals".to_string());
            }
            ExprKind::Alloc { .. } => {
                return Err("LLVM backend does not support 'alloc'".to_string());
            }
            ExprKind::Member { .. } => {
                return Err("LLVM backend does not support field access".to_string());
            }
            ExprKind::Index { .. } => {
                return Err("LLVM backend does not support slice indexing".to_string());
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
        header.push_str("declare i32 @printf(ptr, ...)\n");
        header.push_str("declare i32 @fputs(ptr, ptr)\n");
        header.push_str("@stdout = external global ptr\n");

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
                let value = value.as_ref().ok_or_else(|| {
                    "LLVM backend requires an initializer for every binding".to_string()
                })?;
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
                let ExprKind::Ident(name) = &target.kind else {
                    return Err("LLVM backend only supports assignment to a binding".to_string());
                };
                let binding = self
                    .lookup(name)
                    .ok_or_else(|| format!("LLVM backend could not resolve binding '{name}'"))?;
                let (value_ty, ty) = match op {
                    AssignOp::Assign => {
                        let ty = binding.ty.clone();
                        (ty.clone(), ty)
                    }
                    _ => {
                        let ty = binding.ty.clone();
                        (ty.clone(), ty)
                    }
                };
                let inferred = self.infer(value, Some(&value_ty))?;
                let mut code = self.gen_expr(value, Some(&value_ty))?;
                code = self.coerce(&code, &inferred, &value_ty)?;
                if *op != AssignOp::Assign {
                    let previous = self.load_binding(&binding);
                    code = self.binary_code(op_to_binary(*op), &previous, &code, &ty)?;
                }
                self.current.line(format!(
                    "store {} {code}, ptr {}",
                    self.llvm_type(&ty)?,
                    binding.value
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
            StmtKind::Protect(_) => {
                return Err("LLVM backend does not support 'protect' blocks".to_string());
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
                Ok(ptr)
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
            ExprKind::SliceLiteral { .. } => {
                Err("LLVM backend does not support slice literals".to_string())
            }
            ExprKind::InterfaceLiteral { .. } => {
                Err("LLVM backend does not support shape literals".to_string())
            }
            ExprKind::Alloc { .. } => Err("LLVM backend does not support 'alloc'".to_string()),
            ExprKind::Member { .. } => {
                Err("LLVM backend does not support field access".to_string())
            }
            ExprKind::Index { .. } => {
                Err("LLVM backend does not support slice indexing".to_string())
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
            "print" => {
                let arg = args
                    .first()
                    .ok_or_else(|| "LLVM backend: 'print' requires a string".to_string())?;
                if !matches!(arg.value.kind, ExprKind::String(_)) {
                    return Err("LLVM backend requires a string literal in 'print'".to_string());
                }
                let ptr = self.gen_expr(&arg.value, None)?;
                let fmt = self.gep_string("%s");
                self.current
                    .line(format!("call i32 (ptr, ...) @printf(ptr {fmt}, ptr {ptr})"));
                Ok("void".to_string())
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
            Type::Slice(_, _) => self.validate_type(ty).map(|()| Ty::Error),
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
            other => Err(format!("LLVM backend does not support type '{other}'")),
        }
    }

    fn zero_value(&self, ty: &Ty) -> Result<String, String> {
        match ty {
            Ty::Bool => Ok("false".to_string()),
            Ty::F32 | Ty::F64 | Ty::FloatLiteral => Ok("0.0".to_string()),
            _ if ty.is_integer() => Ok("0".to_string()),
            other => Err(format!(
                "LLVM backend cannot make a zero value for '{other}'"
            )),
        }
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
            ExprKind::Call { callee, .. } => {
                let ExprKind::Ident(name) = &callee.kind else {
                    return Err("LLVM backend cannot infer an indirect call".to_string());
                };
                if SUPPORTED_INTRINSICS.contains(&name.as_str()) {
                    Ok(match name.as_str() {
                        "sqrt" | "floor" | "ceil" | "fabs" | "pow" => Ty::F64,
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
            ExprKind::SliceLiteral { .. } => {
                Err("LLVM backend does not support slice literals".to_string())
            }
            ExprKind::InterfaceLiteral { .. } => {
                Err("LLVM backend does not support shape literals".to_string())
            }
            ExprKind::Alloc { .. } => Err("LLVM backend does not support 'alloc'".to_string()),
            ExprKind::Member { .. } => {
                Err("LLVM backend does not support field access".to_string())
            }
            ExprKind::Index { .. } => {
                Err("LLVM backend does not support slice indexing".to_string())
            }
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

    fn load_binding(&mut self, binding: &Binding) -> String {
        let pointer = binding.value.clone();
        self.load_raw(&pointer, &binding.ty)
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
    "print_int",
    "print_float",
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
