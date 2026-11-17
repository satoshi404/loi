//! Compile-time evaluation for `comptime` and global initializers.
//!
//! The evaluator interprets the pure subset of Loi: literals, shapes, slices,
//! arithmetic, and calls to `comptime flow` declarations plus a small set of
//! math intrinsics. It is deliberately independent from the C backend so the
//! checker can prove that an initializer is constant before code generation.
//!
//! Evaluation is bounded by a step budget and a call-depth limit, so a
//! recursive `comptime flow` fails with a diagnostic instead of hanging the
//! compiler.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::types::{self, Ty};

/// A value produced by compile-time evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Void,
    Int(i128),
    UInt(u128),
    Float(f64),
    Bool(bool),
    String(String),
    CStr(String),
    Slice {
        element: Ty,
        values: Vec<ConstValue>,
    },
    Shape {
        name: String,
        fields: Vec<(String, ConstValue)>,
    },
}

impl ConstValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConstValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i128> {
        match self {
            ConstValue::Int(value) => Some(*value),
            ConstValue::UInt(value) => i128::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_uint(&self) -> Option<u128> {
        match self {
            ConstValue::UInt(value) => Some(*value),
            ConstValue::Int(value) => u128::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            ConstValue::Float(value) => Some(*value),
            ConstValue::Int(value) => Some(*value as f64),
            ConstValue::UInt(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn ty(&self) -> Ty {
        match self {
            ConstValue::Void => Ty::Void,
            ConstValue::Int(_) => Ty::I32,
            ConstValue::UInt(_) => Ty::U32,
            ConstValue::Float(_) => Ty::F64,
            ConstValue::Bool(_) => Ty::Bool,
            ConstValue::String(_) => Ty::Slice(Box::new(Ty::U8)),
            ConstValue::CStr(_) => Ty::CStr,
            ConstValue::Slice { element, .. } => Ty::Slice(Box::new(element.clone())),
            ConstValue::Shape { name, .. } => Ty::Named(name.clone()),
        }
    }
}

/// A failure during constant evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstError {
    pub span: Span,
    pub message: String,
}

impl ConstError {
    fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct Binding {
    value: ConstValue,
    /// Declared type of the binding, when known. Compile-time arithmetic uses
    /// it to reproduce the width and overflow behaviour of runtime values.
    ty: Option<Ty>,
}

type Env = HashMap<String, Binding>;

/// Evaluates constant expressions and pure `comptime flow` bodies.
pub struct ConstEvaluator<'a> {
    interfaces: HashMap<&'a str, &'a InterfaceDecl>,
    functions: HashMap<&'a str, &'a FunctionDecl>,
    globals: HashMap<String, Binding>,
    steps: usize,
    max_steps: usize,
    call_depth: usize,
    max_call_depth: usize,
    /// Expected type of the `return` currently being evaluated, used to catch
    /// overflow with the same width the runtime would use.
    current_return: Option<Ty>,
}

impl<'a> ConstEvaluator<'a> {
    pub fn new(program: &'a Program) -> Self {
        let mut interfaces = HashMap::new();
        let mut functions = HashMap::new();
        for item in &program.items {
            match item {
                Item::Interface(decl) => {
                    interfaces.entry(decl.name.as_str()).or_insert(decl);
                }
                Item::Function(decl) => {
                    functions.entry(decl.name.as_str()).or_insert(decl);
                }
                _ => {}
            }
        }
        Self {
            interfaces,
            functions,
            globals: HashMap::new(),
            steps: 0,
            max_steps: 1_000_000,
            call_depth: 0,
            max_call_depth: 64,
            current_return: None,
        }
    }

    /// Evaluate a global initializer and record its value for later lookups.
    pub fn eval_global(&mut self, decl: &GlobalDecl) -> Result<ConstValue, ConstError> {
        let expected = decl.ty.as_ref().and_then(|ty| self.resolve_type(ty));
        let value = self.eval_expr(&decl.value, expected.as_ref())?;
        let value = match &expected {
            Some(expected) => coerce_to(value, expected, decl.value.span)?,
            None => value,
        };
        self.globals.insert(
            decl.name.clone(),
            Binding {
                value: value.clone(),
                ty: expected,
            },
        );
        Ok(value)
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: ConstValue) {
        self.globals
            .insert(name.into(), Binding { value, ty: None });
    }

    /// Evaluate an expression with no local bindings.
    pub fn eval_expr(
        &mut self,
        expr: &Expr,
        expected: Option<&Ty>,
    ) -> Result<ConstValue, ConstError> {
        let mut env = Env::new();
        self.eval_expr_env(expr, expected, &mut env)
    }

    fn tick(&mut self, span: Span) -> Result<(), ConstError> {
        self.steps += 1;
        if self.steps > self.max_steps {
            return Err(ConstError::new(
                span,
                "comptime evaluation exceeded the step limit",
            ));
        }
        Ok(())
    }

    pub fn resolve_type(&self, ty: &Type) -> Option<Ty> {
        match ty {
            Type::Named(name) => {
                if let Some(primitive) = Ty::from_name(name) {
                    Some(primitive)
                } else if name == "cstr" {
                    Some(Ty::CStr)
                } else if self.interfaces.contains_key(name.as_str()) {
                    Some(Ty::Named(name.clone()))
                } else {
                    None
                }
            }
            Type::Slice(element) => Some(Ty::Slice(Box::new(self.resolve_type(element)?))),
            Type::Generic(_, _) => None,
        }
    }

    fn eval_expr_env(
        &mut self,
        expr: &Expr,
        expected: Option<&Ty>,
        env: &mut Env,
    ) -> Result<ConstValue, ConstError> {
        self.tick(expr.span)?;
        match &expr.kind {
            ExprKind::Int(text) => Ok(ConstValue::Int(parse_int(text, expr.span)?)),
            ExprKind::Float(text) => {
                let value = parse_float(text, expr.span)?;
                // A literal used as `f32` must be rounded to `f32` precision
                // before it reaches arithmetic, exactly like the C backend.
                let value = if matches!(expected, Some(Ty::F32)) {
                    value as f32 as f64
                } else {
                    value
                };
                Ok(ConstValue::Float(value))
            }
            ExprKind::Bool(value) => Ok(ConstValue::Bool(*value)),
            ExprKind::String(text) => {
                if matches!(expected, Some(Ty::CStr)) {
                    Ok(ConstValue::CStr(text.clone()))
                } else {
                    Ok(ConstValue::String(text.clone()))
                }
            }
            ExprKind::Ident(name) => {
                if let Some(binding) = env.get(name) {
                    Ok(binding.value.clone())
                } else if let Some(binding) = self.globals.get(name) {
                    Ok(binding.value.clone())
                } else {
                    Err(ConstError::new(
                        expr.span,
                        format!("'{name}' is not a compile-time constant"),
                    ))
                }
            }
            ExprKind::SliceLiteral {
                element_type,
                elements,
            } => {
                let explicit = element_type.as_ref().and_then(|ty| self.resolve_type(ty));
                let element = explicit.clone().or_else(|| match expected {
                    Some(Ty::Slice(element)) => Some((**element).clone()),
                    _ => None,
                });
                let mut values = Vec::with_capacity(elements.len());
                for item in elements {
                    let value = self.eval_expr_env(item, element.as_ref(), env)?;
                    let value = match &element {
                        Some(element) => coerce_to(value, element, item.span)?,
                        None => value,
                    };
                    values.push(value);
                }
                let element = match element {
                    Some(element) => element,
                    None => values
                        .first()
                        .map(ConstValue::ty)
                        .ok_or_else(|| {
                            ConstError::new(
                                expr.span,
                                "cannot infer the element type of an empty slice literal",
                            )
                        })?
                        .concretize(),
                };
                Ok(ConstValue::Slice { element, values })
            }
            ExprKind::InterfaceLiteral { name, fields, .. } => {
                let Some(decl) = self.interfaces.get(name.as_str()).copied() else {
                    return Err(ConstError::new(
                        expr.span,
                        format!("unknown shape '{name}'"),
                    ));
                };
                let decl_fields = decl.fields.clone();
                let mut values = Vec::new();
                for field in &decl_fields {
                    let expected = self.resolve_type(&field.ty);
                    let init = fields.iter().find(|init| init.name == field.name);
                    let value = match (init, &field.default) {
                        (Some(init), _) => {
                            self.eval_expr_env(&init.value, expected.as_ref(), env)?
                        }
                        (None, Some(default)) => {
                            self.eval_expr_env(default, expected.as_ref(), env)?
                        }
                        (None, None) => {
                            return Err(ConstError::new(
                                expr.span,
                                format!("missing field '{}' in literal '{name}'", field.name),
                            ));
                        }
                    };
                    let value = match expected {
                        Some(expected) => coerce_to(value, &expected, expr.span)?,
                        None => value,
                    };
                    values.push((field.name.clone(), value));
                }
                for init in fields {
                    if !decl_fields.iter().any(|field| field.name == init.name) {
                        return Err(ConstError::new(
                            init.value.span,
                            format!("unknown field '{}' in literal '{name}'", init.name),
                        ));
                    }
                }
                Ok(ConstValue::Shape {
                    name: name.clone(),
                    fields: values,
                })
            }
            ExprKind::Unary { op, operand } => {
                let operand_expected = match op {
                    UnaryOp::Neg | UnaryOp::BitNot => expected.filter(|ty| ty.is_numeric()),
                    UnaryOp::Not => None,
                };
                let value = self.eval_expr_env(operand, operand_expected, env)?;
                let span = expr.span;
                match op {
                    UnaryOp::Neg => {
                        let value = match value {
                            ConstValue::Int(value) => {
                                ConstValue::Int(value.checked_neg().ok_or_else(|| {
                                    ConstError::new(span, "integer overflow in comptime expression")
                                })?)
                            }
                            ConstValue::Float(value) => ConstValue::Float(-value),
                            ConstValue::UInt(value) => {
                                let value = i128::try_from(value).map_err(|_| {
                                    ConstError::new(span, "integer overflow in comptime expression")
                                })?;
                                ConstValue::Int(value.checked_neg().ok_or_else(|| {
                                    ConstError::new(span, "integer overflow in comptime expression")
                                })?)
                            }
                            _ => {
                                return Err(ConstError::new(
                                    span,
                                    "'-' requires a numeric compile-time value",
                                ));
                            }
                        };
                        fit_result(value, expected, span)
                    }
                    UnaryOp::Not => match value {
                        ConstValue::Bool(value) => Ok(ConstValue::Bool(!value)),
                        _ => Err(ConstError::new(
                            span,
                            "'not' requires a bool compile-time value",
                        )),
                    },
                    UnaryOp::BitNot => {
                        let value = match value {
                            ConstValue::Int(value) => ConstValue::Int(!value),
                            ConstValue::UInt(value) => ConstValue::UInt(!value),
                            _ => {
                                return Err(ConstError::new(
                                    span,
                                    "'~' requires an integer compile-time value",
                                ));
                            }
                        };
                        fit_bitwise_result(value, expected, span)
                    }
                }
            }
            ExprKind::Binary { op, left, right } => {
                if *op == BinaryOp::And {
                    let left_value = self.eval_expr_env(left, Some(&Ty::Bool), env)?;
                    let left_value = as_bool_value(left_value, left.span)?;
                    if !left_value {
                        return Ok(ConstValue::Bool(false));
                    }
                    let right_value = self.eval_expr_env(right, Some(&Ty::Bool), env)?;
                    return Ok(ConstValue::Bool(as_bool_value(right_value, right.span)?));
                }
                if *op == BinaryOp::Or {
                    let left_value = self.eval_expr_env(left, Some(&Ty::Bool), env)?;
                    let left_value = as_bool_value(left_value, left.span)?;
                    if left_value {
                        return Ok(ConstValue::Bool(true));
                    }
                    let right_value = self.eval_expr_env(right, Some(&Ty::Bool), env)?;
                    return Ok(ConstValue::Bool(as_bool_value(right_value, right.span)?));
                }
                let numeric_expected = expected.filter(|ty| ty.is_numeric());
                let left_value = self.eval_expr_env(left, numeric_expected, env)?;
                let right_value = self.eval_expr_env(right, numeric_expected, env)?;
                eval_binary(*op, left_value, right_value, expr.span, expected)
            }
            ExprKind::Cast { expr: inner, ty } => {
                let target = self
                    .resolve_type(ty)
                    .ok_or_else(|| ConstError::new(expr.span, "unknown cast target type"))?;
                let value = self.eval_expr_env(inner, None, env)?;
                convert_to(value, &target, expr.span)
            }
            ExprKind::Call { callee, args, .. } => {
                let ExprKind::Ident(name) = &callee.kind else {
                    return Err(ConstError::new(
                        expr.span,
                        "only named flows can be called in a comptime expression",
                    ));
                };
                self.eval_call(name, args, env, expr.span)
            }
            ExprKind::Alloc { .. } => Err(ConstError::new(
                expr.span,
                "'alloc' is not allowed in a comptime expression",
            )),
            ExprKind::Move(value) | ExprKind::Copy(value) | ExprKind::Comptime(value) => {
                self.eval_expr_env(value, expected, env)
            }
            ExprKind::Member { base, field } => {
                let base_value = self.eval_expr_env(base, None, env)?;
                match base_value {
                    ConstValue::Slice { values, .. } if field == "len" => {
                        Ok(ConstValue::UInt(values.len() as u128))
                    }
                    ConstValue::String(text) if field == "len" => {
                        Ok(ConstValue::UInt(text.len() as u128))
                    }
                    ConstValue::Shape { fields, .. } => fields
                        .into_iter()
                        .find(|(name, _)| name == field)
                        .map(|(_, value)| value)
                        .ok_or_else(|| {
                            ConstError::new(
                                expr.span,
                                format!("compile-time shape has no field '{field}'"),
                            )
                        }),
                    _ => Err(ConstError::new(
                        expr.span,
                        format!("cannot read field '{field}' at compile time"),
                    )),
                }
            }
            ExprKind::Index { base, index } => {
                let base_value = self.eval_expr_env(base, None, env)?;
                let index_value = self.eval_expr_env(index, None, env)?;
                let index = index_value
                    .as_uint()
                    .ok_or_else(|| ConstError::new(index.span, "slice index must be an integer"))?
                    as usize;
                match base_value {
                    ConstValue::Slice { values, .. } => values
                        .get(index)
                        .cloned()
                        .ok_or_else(|| ConstError::new(expr.span, "slice index out of bounds")),
                    ConstValue::String(text) => {
                        let byte = text.as_bytes().get(index).copied().ok_or_else(|| {
                            ConstError::new(expr.span, "string index out of bounds")
                        })?;
                        Ok(ConstValue::UInt(byte as u128))
                    }
                    _ => Err(ConstError::new(
                        expr.span,
                        "only slices and strings support indexing at compile time",
                    )),
                }
            }
        }
    }

    fn eval_call(
        &mut self,
        name: &str,
        args: &[CallArg],
        env: &mut Env,
        span: Span,
    ) -> Result<ConstValue, ConstError> {
        match name {
            "len" => {
                if args.len() != 1 {
                    return Err(ConstError::new(span, "len expects 1 argument"));
                }
                let value = self.eval_expr_env(&args[0].value, None, env)?;
                match value {
                    ConstValue::Slice { values, .. } => Ok(ConstValue::UInt(values.len() as u128)),
                    ConstValue::String(text) => Ok(ConstValue::UInt(text.len() as u128)),
                    _ => Err(ConstError::new(span, "len expects a slice at compile time")),
                }
            }
            "read_file" | "read_stdin" | "read_line" | "write_file" | "print_int"
            | "print_float" => Err(ConstError::new(
                span,
                "I/O is not allowed in a comptime expression",
            )),
            "sqrt" | "floor" | "ceil" | "fabs" | "pow" | "fmod" => {
                let mut values = Vec::new();
                for arg in args {
                    let value = self.eval_expr_env(&arg.value, None, env)?;
                    let value = value.as_float().ok_or_else(|| {
                        ConstError::new(arg.value.span, format!("{name} expects numeric arguments"))
                    })?;
                    values.push(value);
                }
                let result = match name {
                    "sqrt" => values.first().map(|x| x.sqrt()),
                    "floor" => values.first().map(|x| x.floor()),
                    "ceil" => values.first().map(|x| x.ceil()),
                    "fabs" => values.first().map(|x| x.abs()),
                    "pow" => match values.as_slice() {
                        [base, exponent] => Some(base.powf(*exponent)),
                        _ => None,
                    },
                    "fmod" => match values.as_slice() {
                        [left, right] => Some(left % right),
                        _ => None,
                    },
                    _ => None,
                }
                .ok_or_else(|| {
                    ConstError::new(
                        span,
                        format!("{name} received the wrong number of arguments"),
                    )
                })?;
                Ok(ConstValue::Float(result))
            }
            _ => self.call_comptime_flow(name, args, env, span),
        }
    }

    fn call_comptime_flow(
        &mut self,
        name: &str,
        args: &[CallArg],
        caller_env: &mut Env,
        span: Span,
    ) -> Result<ConstValue, ConstError> {
        let Some(function) = self.functions.get(name).copied() else {
            return Err(ConstError::new(
                span,
                format!("unknown flow '{name}' in comptime expression"),
            ));
        };
        if !function.comptime {
            return Err(ConstError::new(
                span,
                format!("flow '{name}' is not declared 'comptime' and cannot run at compile time"),
            ));
        }
        if function.external {
            return Err(ConstError::new(
                span,
                format!("extern flow '{name}' cannot run at compile time"),
            ));
        }
        if function.params.len() != args.len() {
            return Err(ConstError::new(
                span,
                format!(
                    "flow '{name}' expects {} argument(s), found {}",
                    function.params.len(),
                    args.len()
                ),
            ));
        }

        let params = function.params.clone();
        let mut env = Env::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            if arg.direction != Direction::In || param.direction != Direction::In {
                return Err(ConstError::new(
                    arg.value.span,
                    "comptime flows only accept '<-' arguments",
                ));
            }
            let expected = self.resolve_type(&param.ty);
            let value = self.eval_expr_env(&arg.value, expected.as_ref(), caller_env)?;
            let value = match &expected {
                Some(expected) => coerce_to(value, expected, arg.value.span)?,
                None => value,
            };
            env.insert(
                param.name.clone(),
                Binding {
                    value,
                    ty: expected,
                },
            );
        }

        self.call_depth += 1;
        if self.call_depth > self.max_call_depth {
            self.call_depth -= 1;
            return Err(ConstError::new(
                span,
                "comptime call depth exceeded; is the flow recursive?",
            ));
        }
        let return_expected = match &function.return_type {
            Some(ty) => Some(
                self.resolve_type(ty)
                    .ok_or_else(|| ConstError::new(span, "unknown return type in comptime flow"))?,
            ),
            None => None,
        };
        let saved_return = std::mem::replace(&mut self.current_return, return_expected.clone());
        let result = self.exec_block(&function.body, &mut env);
        self.current_return = saved_return;
        self.call_depth -= 1;
        match result? {
            Flow::Return(value) => {
                let value = match &return_expected {
                    Some(expected) => coerce_to(value, expected, span)?,
                    None => value,
                };
                Ok(value)
            }
            _ => Ok(ConstValue::Void),
        }
    }

    fn exec_block(&mut self, block: &Block, env: &mut Env) -> Result<Flow, ConstError> {
        for statement in &block.statements {
            self.tick(statement.span)?;
            match self.exec_statement(statement, env)? {
                Flow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec_statement(&mut self, statement: &Stmt, env: &mut Env) -> Result<Flow, ConstError> {
        match &statement.kind {
            StmtKind::Let {
                name, ty, value, ..
            } => {
                let expected = ty.as_ref().and_then(|ty| self.resolve_type(ty));
                let value = value.as_ref().ok_or_else(|| {
                    ConstError::new(
                        statement.span,
                        "comptime flow bindings require an initializer",
                    )
                })?;
                let mut value = self.eval_expr_env(value, expected.as_ref(), env)?;
                if let Some(expected) = &expected {
                    value = coerce_to(value, expected, statement.span)?;
                }
                env.insert(
                    name.clone(),
                    Binding {
                        value,
                        ty: expected,
                    },
                );
                Ok(Flow::Normal)
            }
            StmtKind::Assign { target, op, value } => {
                let ExprKind::Ident(name) = &target.kind else {
                    return Err(ConstError::new(
                        target.span,
                        "comptime flows can only assign to local bindings",
                    ));
                };
                if !env.contains_key(name) {
                    return Err(ConstError::new(
                        target.span,
                        format!(
                            "cannot mutate '{name}' at compile time; only local bindings are allowed"
                        ),
                    ));
                }
                let existing = env.get(name).cloned().expect("local binding");
                let right = self.eval_expr_env(value, existing.ty.as_ref(), env)?;
                let result = if *op == AssignOp::Assign {
                    right
                } else {
                    apply_assign(
                        *op,
                        existing.value.clone(),
                        right,
                        statement.span,
                        existing.ty.as_ref(),
                    )?
                };
                let result = if let Some(expected) = &existing.ty {
                    coerce_to(result, expected, statement.span)?
                } else {
                    result
                };
                env.insert(
                    name.clone(),
                    Binding {
                        value: result,
                        ty: existing.ty,
                    },
                );
                Ok(Flow::Normal)
            }
            StmtKind::Expr(expr) => {
                self.eval_expr_env(expr, None, env)?;
                Ok(Flow::Normal)
            }
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                let value = self.eval_expr_env(condition, Some(&Ty::Bool), env)?;
                if as_bool_value(value, condition.span)? {
                    self.exec_block(then_block, env)
                } else {
                    match else_branch {
                        Some(ElseBranch::Block(block)) => self.exec_block(block, env),
                        Some(ElseBranch::If(inner)) => self.exec_statement(inner, env),
                        None => Ok(Flow::Normal),
                    }
                }
            }
            StmtKind::While { condition, body } => {
                loop {
                    self.tick(condition.span)?;
                    let value = self.eval_expr_env(condition, Some(&Ty::Bool), env)?;
                    if !as_bool_value(value, condition.span)? {
                        break;
                    }
                    match self.exec_block(body, env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForRange {
                variable,
                start,
                end,
                body,
            } => {
                let start_value = self.eval_expr_env(start, None, env)?;
                let end_value = self.eval_expr_env(end, None, env)?;
                let index_ty = start_value.ty();
                let start_int = start_value
                    .as_int()
                    .ok_or_else(|| ConstError::new(start.span, "range start must be an integer"))?;
                let end_int = end_value
                    .as_int()
                    .ok_or_else(|| ConstError::new(end.span, "range end must be an integer"))?;
                let mut index = start_int;
                while index < end_int {
                    self.tick(statement.span)?;
                    env.insert(
                        variable.clone(),
                        Binding {
                            value: ConstValue::Int(index),
                            ty: Some(index_ty.clone()),
                        },
                    );
                    match self.exec_block(body, env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                    index += 1;
                }
                Ok(Flow::Normal)
            }
            StmtKind::ForEach {
                direction,
                variable,
                iterable,
                body,
            } => {
                if *direction != Direction::In {
                    return Err(ConstError::new(
                        statement.span,
                        "comptime loops only support '<-' iteration",
                    ));
                }
                let value = self.eval_expr_env(iterable, None, env)?;
                let (values, element_ty) = match value {
                    ConstValue::Slice { element, values } => (values, Some(element)),
                    ConstValue::String(text) => (
                        text.into_bytes()
                            .into_iter()
                            .map(|byte| ConstValue::UInt(byte as u128))
                            .collect(),
                        Some(Ty::U8),
                    ),
                    _ => {
                        return Err(ConstError::new(
                            iterable.span,
                            "comptime iteration expects a slice",
                        ));
                    }
                };
                for value in values {
                    self.tick(statement.span)?;
                    env.insert(
                        variable.clone(),
                        Binding {
                            value,
                            ty: element_ty.clone(),
                        },
                    );
                    match self.exec_block(body, env)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Protect(_) => Err(ConstError::new(
                statement.span,
                "'protect' is not allowed in a comptime flow",
            )),
            StmtKind::Return(value) => {
                let expected = self.current_return.clone();
                let value = match value {
                    Some(value) => {
                        let span = value.span;
                        let mut value = self.eval_expr_env(value, expected.as_ref(), env)?;
                        if let Some(expected) = &expected {
                            value = coerce_to(value, expected, span)?;
                        }
                        value
                    }
                    None => ConstValue::Void,
                };
                Ok(Flow::Return(value))
            }
            StmtKind::Continue => Ok(Flow::Continue),
            StmtKind::Break => Ok(Flow::Break),
        }
    }
}

enum Flow {
    Normal,
    Return(ConstValue),
    Continue,
    Break,
}

fn as_bool_value(value: ConstValue, span: Span) -> Result<bool, ConstError> {
    match value {
        ConstValue::Bool(value) => Ok(value),
        _ => Err(ConstError::new(span, "expected a bool compile-time value")),
    }
}

fn parse_int(text: &str, span: Span) -> Result<i128, ConstError> {
    let cleaned = text.replace('_', "");
    let (digits, radix) = if let Some(rest) = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
    {
        (rest, 16)
    } else if let Some(rest) = cleaned
        .strip_prefix("0b")
        .or_else(|| cleaned.strip_prefix("0B"))
    {
        (rest, 2)
    } else {
        (cleaned.as_str(), 10)
    };
    i128::from_str_radix(digits, radix)
        .map_err(|_| ConstError::new(span, "integer literal is out of range"))
}

fn parse_float(text: &str, span: Span) -> Result<f64, ConstError> {
    text.replace('_', "")
        .parse::<f64>()
        .map_err(|_| ConstError::new(span, "invalid float literal"))
}

fn eval_binary(
    op: BinaryOp,
    left: ConstValue,
    right: ConstValue,
    span: Span,
    expected: Option<&Ty>,
) -> Result<ConstValue, ConstError> {
    match op {
        BinaryOp::Eq => Ok(ConstValue::Bool(values_equal(&left, &right))),
        BinaryOp::Neq => Ok(ConstValue::Bool(!values_equal(&left, &right))),
        BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
            let ordering = compare_values(&left, &right, span)?;
            let value = match op {
                BinaryOp::Lt => ordering.is_lt(),
                BinaryOp::Lte => ordering.is_le(),
                BinaryOp::Gt => ordering.is_gt(),
                BinaryOp::Gte => ordering.is_ge(),
                _ => unreachable!(),
            };
            Ok(ConstValue::Bool(value))
        }
        _ => numeric_binary(op, left, right, span, expected),
    }
}

fn numeric_binary(
    op: BinaryOp,
    left: ConstValue,
    right: ConstValue,
    span: Span,
    expected: Option<&Ty>,
) -> Result<ConstValue, ConstError> {
    let float_expected = expected.filter(|ty| ty.is_float());
    if float_expected.is_some()
        || matches!(left, ConstValue::Float(_))
        || matches!(right, ConstValue::Float(_))
    {
        let mut a = left
            .as_float()
            .ok_or_else(|| ConstError::new(span, "operator requires numeric operands"))?;
        let mut b = right
            .as_float()
            .ok_or_else(|| ConstError::new(span, "operator requires numeric operands"))?;
        let round = matches!(float_expected, Some(Ty::F32));
        if round {
            a = a as f32 as f64;
            b = b as f32 as f64;
        }
        let mut value = match op {
            BinaryOp::Add => a + b,
            BinaryOp::Sub => a - b,
            BinaryOp::Mul => a * b,
            BinaryOp::Div => {
                if b == 0.0 {
                    return Err(ConstError::new(span, "division by zero"));
                }
                a / b
            }
            BinaryOp::Mod => {
                return Err(ConstError::new(
                    span,
                    "'%' requires integer operands at compile time",
                ));
            }
            _ => {
                return Err(ConstError::new(
                    span,
                    "bitwise operators require integer operands",
                ));
            }
        };
        if round {
            value = value as f32 as f64;
        }
        return Ok(ConstValue::Float(value));
    }

    let bits = expected.and_then(integer_bits).unwrap_or(128);
    let signed_expected =
        expected.is_some_and(|ty| matches!(ty, Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64));
    let unsigned_expected =
        expected.is_some_and(|ty| matches!(ty, Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64));
    let unsigned = unsigned_expected
        || (!signed_expected
            && (matches!(left, ConstValue::UInt(_)) || matches!(right, ConstValue::UInt(_))));
    if unsigned {
        let a = left
            .as_uint()
            .ok_or_else(|| ConstError::new(span, "unsigned operand is out of range"))?;
        let b = right
            .as_uint()
            .ok_or_else(|| ConstError::new(span, "unsigned operand is out of range"))?;
        let value = match op {
            // Unsigned `+`, `-` and `*` wrap, matching the C backend.
            BinaryOp::Add => a.wrapping_add(b),
            BinaryOp::Sub => a.wrapping_sub(b),
            BinaryOp::Mul => a.wrapping_mul(b),
            BinaryOp::Div => {
                if b == 0 {
                    return Err(ConstError::new(span, "division by zero"));
                }
                mask_int(a / b, bits)
            }
            BinaryOp::Mod => {
                if b == 0 {
                    return Err(ConstError::new(span, "remainder by zero"));
                }
                mask_int(a % b, bits)
            }
            BinaryOp::BitAnd => mask_int(a & b, bits),
            BinaryOp::BitOr => mask_int(a | b, bits),
            BinaryOp::BitXor => mask_int(a ^ b, bits),
            BinaryOp::Shl => {
                if b >= bits as u128 {
                    return Err(ConstError::new(span, "shift count is out of range"));
                }
                mask_int(a.wrapping_shl(b as u32), bits)
            }
            BinaryOp::Shr => {
                if b >= bits as u128 {
                    return Err(ConstError::new(span, "shift count is out of range"));
                }
                mask_int(a >> b, bits)
            }
            _ => {
                return Err(ConstError::new(span, "operator requires numeric operands"));
            }
        };
        return Ok(ConstValue::UInt(mask_int(value, bits)));
    }

    let a = left
        .as_int()
        .ok_or_else(|| ConstError::new(span, "operator requires numeric operands"))?;
    let b = right
        .as_int()
        .ok_or_else(|| ConstError::new(span, "operator requires numeric operands"))?;
    let value = match op {
        BinaryOp::Add => a.checked_add(b),
        BinaryOp::Sub => a.checked_sub(b),
        BinaryOp::Mul => a.checked_mul(b),
        BinaryOp::Div => {
            if b == 0 {
                return Err(ConstError::new(span, "division by zero"));
            }
            a.checked_div(b)
        }
        BinaryOp::Mod => {
            if b == 0 {
                return Err(ConstError::new(span, "remainder by zero"));
            }
            a.checked_rem(b)
        }
        BinaryOp::BitAnd => Some(a & b),
        BinaryOp::BitOr => Some(a | b),
        BinaryOp::BitXor => Some(a ^ b),
        BinaryOp::Shl => {
            if !(0..bits as i128).contains(&b) {
                return Err(ConstError::new(span, "shift count is out of range"));
            }
            Some(a.wrapping_shl(b as u32))
        }
        BinaryOp::Shr => {
            if !(0..bits as i128).contains(&b) {
                return Err(ConstError::new(span, "shift count is out of range"));
            }
            Some(a >> b)
        }
        _ => {
            return Err(ConstError::new(span, "operator requires numeric operands"));
        }
    }
    .ok_or_else(|| ConstError::new(span, "integer overflow in comptime expression"))?;
    let value = if matches!(
        op,
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
    ) {
        check_signed_range(value, bits, span)?
    } else {
        wrap_int(value, bits)
    };
    Ok(ConstValue::Int(value))
}

/// Fit a unary arithmetic result to the expected type, rejecting the overflow
/// that the runtime would trap on.
fn fit_result(
    value: ConstValue,
    expected: Option<&Ty>,
    span: Span,
) -> Result<ConstValue, ConstError> {
    fit_result_impl(value, expected, span, false)
}

/// Fit a bitwise result, which wraps to the expected width instead of trapping.
fn fit_bitwise_result(
    value: ConstValue,
    expected: Option<&Ty>,
    span: Span,
) -> Result<ConstValue, ConstError> {
    fit_result_impl(value, expected, span, true)
}

fn fit_result_impl(
    value: ConstValue,
    expected: Option<&Ty>,
    span: Span,
    wrap: bool,
) -> Result<ConstValue, ConstError> {
    let Some(expected) = expected else {
        return Ok(value);
    };
    match expected {
        Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 => {
            let bits = integer_bits(expected).expect("signed integer width");
            match value {
                ConstValue::Int(value) => {
                    let value = if wrap {
                        wrap_int(value, bits)
                    } else {
                        check_signed_range(value, bits, span)?
                    };
                    Ok(ConstValue::Int(value))
                }
                other => Ok(other),
            }
        }
        Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 => {
            let bits = integer_bits(expected).expect("unsigned integer width");
            match value {
                ConstValue::UInt(value) => Ok(ConstValue::UInt(mask_int(value, bits))),
                ConstValue::Int(value) => Ok(ConstValue::UInt(mask_int(value as u128, bits))),
                other => Ok(other),
            }
        }
        Ty::F32 => match value {
            ConstValue::Float(value) => Ok(ConstValue::Float(value as f32 as f64)),
            other => Ok(other),
        },
        _ => Ok(value),
    }
}

fn check_signed_range(value: i128, bits: u32, span: Span) -> Result<i128, ConstError> {
    if bits >= 128 {
        return Ok(value);
    }
    let min = -(1i128 << (bits - 1));
    let max = (1i128 << (bits - 1)) - 1;
    if value < min || value > max {
        return Err(ConstError::new(
            span,
            "integer overflow in comptime expression",
        ));
    }
    Ok(value)
}

fn apply_assign(
    op: AssignOp,
    left: ConstValue,
    right: ConstValue,
    span: Span,
    expected: Option<&Ty>,
) -> Result<ConstValue, ConstError> {
    let binary = match op {
        AssignOp::Assign => return Ok(right),
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
    };
    numeric_binary(binary, left, right, span, expected)
}

fn values_equal(left: &ConstValue, right: &ConstValue) -> bool {
    match (left, right) {
        (ConstValue::Bool(a), ConstValue::Bool(b)) => a == b,
        (ConstValue::String(a), ConstValue::String(b))
        | (ConstValue::CStr(a), ConstValue::CStr(b))
        | (ConstValue::String(a), ConstValue::CStr(b))
        | (ConstValue::CStr(a), ConstValue::String(b)) => a == b,
        (ConstValue::Slice { values: a, .. }, ConstValue::Slice { values: b, .. }) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(left, right)| values_equal(left, right))
        }
        (ConstValue::Shape { fields: a, .. }, ConstValue::Shape { fields: b, .. }) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|((name_a, value_a), (name_b, value_b))| {
                        name_a == name_b && values_equal(value_a, value_b)
                    })
        }
        _ => {
            if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
                a == b
            } else {
                false
            }
        }
    }
}

fn compare_values(
    left: &ConstValue,
    right: &ConstValue,
    span: Span,
) -> Result<std::cmp::Ordering, ConstError> {
    if let (Some(a), Some(b)) = (left.as_float(), right.as_float()) {
        return a
            .partial_cmp(&b)
            .ok_or_else(|| ConstError::new(span, "cannot compare NaN at compile time"));
    }
    Err(ConstError::new(
        span,
        "comparison requires numeric compile-time values",
    ))
}

/// Explicit cast (`as`): truncate integers to the target width.
fn convert_to(value: ConstValue, target: &Ty, span: Span) -> Result<ConstValue, ConstError> {
    convert_impl(value, target, span, false)
}

/// Implicit assignment coercion: require the value to fit, like runtime
/// arithmetic that traps instead of wrapping.
fn coerce_to(value: ConstValue, target: &Ty, span: Span) -> Result<ConstValue, ConstError> {
    convert_impl(value, target, span, true)
}

fn convert_impl(
    value: ConstValue,
    target: &Ty,
    span: Span,
    checked: bool,
) -> Result<ConstValue, ConstError> {
    match target {
        Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 => {
            let bits = integer_bits(target).unwrap_or(64);
            let raw = match &value {
                ConstValue::Int(value) => *value,
                ConstValue::UInt(value) => i128::try_from(*value).map_err(|_| {
                    ConstError::new(span, format!("integer value is out of range for {target}"))
                })?,
                ConstValue::Float(value) => *value as i128,
                _ => {
                    return Err(ConstError::new(span, "expected an integer value"));
                }
            };
            if checked {
                let min = -(1i128 << (bits - 1));
                let max = (1i128 << (bits - 1)) - 1;
                if raw < min || raw > max {
                    return Err(ConstError::new(
                        span,
                        format!(
                            "integer overflow in comptime expression: value does not fit in {target}"
                        ),
                    ));
                }
                Ok(ConstValue::Int(raw))
            } else {
                Ok(ConstValue::Int(wrap_int(raw, bits)))
            }
        }
        Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 => {
            let bits = integer_bits(target).unwrap_or(64);
            let raw = match &value {
                ConstValue::UInt(value) => *value,
                ConstValue::Int(value) => {
                    if *value < 0 {
                        return Err(ConstError::new(
                            span,
                            format!(
                                "integer overflow in comptime expression: value does not fit in {target}"
                            ),
                        ));
                    }
                    *value as u128
                }
                ConstValue::Float(value) => {
                    if *value < 0.0 {
                        return Err(ConstError::new(
                            span,
                            format!(
                                "integer overflow in comptime expression: value does not fit in {target}"
                            ),
                        ));
                    }
                    *value as u128
                }
                _ => {
                    return Err(ConstError::new(span, "expected an integer value"));
                }
            };
            if checked {
                let max = if bits >= 128 {
                    u128::MAX
                } else {
                    (1u128 << bits) - 1
                };
                if raw > max {
                    return Err(ConstError::new(
                        span,
                        format!(
                            "integer overflow in comptime expression: value does not fit in {target}"
                        ),
                    ));
                }
            }
            Ok(ConstValue::UInt(mask_int(raw, bits)))
        }
        Ty::F32 => value
            .as_float()
            .map(|value| ConstValue::Float(value as f32 as f64))
            .ok_or_else(|| ConstError::new(span, "expected a numeric value")),
        Ty::F64 => value
            .as_float()
            .map(ConstValue::Float)
            .ok_or_else(|| ConstError::new(span, "expected a numeric value")),
        Ty::Bool => match value {
            ConstValue::Bool(value) => Ok(ConstValue::Bool(value)),
            ConstValue::Int(value) => Ok(ConstValue::Bool(value != 0)),
            ConstValue::UInt(value) => Ok(ConstValue::Bool(value != 0)),
            _ => Err(ConstError::new(span, "expected a bool value")),
        },
        Ty::CStr => match value {
            ConstValue::String(text) | ConstValue::CStr(text) => Ok(ConstValue::CStr(text)),
            _ => Err(ConstError::new(span, "expected a string value")),
        },
        Ty::Slice(_) => Ok(value),
        Ty::Named(_) => Ok(value),
        _ => Err(ConstError::new(span, "unsupported compile-time conversion")),
    }
}

fn integer_bits(ty: &Ty) -> Option<u32> {
    match ty {
        Ty::I8 | Ty::U8 => Some(8),
        Ty::I16 | Ty::U16 => Some(16),
        Ty::I32 | Ty::U32 => Some(32),
        Ty::I64 | Ty::U64 => Some(64),
        _ => None,
    }
}

fn mask_int(value: u128, bits: u32) -> u128 {
    if bits >= 128 {
        value
    } else {
        value & ((1u128 << bits) - 1)
    }
}

fn wrap_int(value: i128, bits: u32) -> i128 {
    if bits >= 128 {
        return value;
    }
    let masked = mask_int(value as u128, bits);
    let sign = 1u128 << (bits - 1);
    if masked & sign != 0 {
        (masked as i128) - (1i128 << bits)
    } else {
        masked as i128
    }
}

// ------------------------------------------------------------------
// Purity and compile-time validation
// ------------------------------------------------------------------

const PURE_INTRINSICS: &[&str] = &["len", "sqrt", "floor", "ceil", "fabs", "pow", "fmod"];

/// Check that every `comptime flow` body is pure: no `alloc`, no `protect`, no
/// I/O, no mutation of globals, and only calls to other pure comptime flows or
/// the math intrinsics.
pub fn validate_comptime_functions(program: &Program) -> Vec<ConstError> {
    let mut errors = Vec::new();
    let globals = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Global(decl) => Some(decl.name.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let comptime_functions = function_names(program, true);
    let all_functions = function_names(program, false);

    for item in &program.items {
        let Item::Function(decl) = item else {
            continue;
        };
        if !decl.comptime {
            continue;
        }
        validate_block(
            &decl.body,
            &globals,
            &comptime_functions,
            &all_functions,
            &mut errors,
        );
    }
    errors
}

/// Evaluate every `comptime expr` in function bodies and interface defaults.
/// Global initializers are covered by [`ConstEvaluator::eval_global`].
pub fn check_comptime_expressions(program: &Program) -> Vec<ConstError> {
    let mut evaluator = ConstEvaluator::new(program);
    for item in &program.items {
        if let Item::Global(decl) = item {
            let _ = evaluator.eval_global(decl);
        }
    }

    let mut errors = Vec::new();
    for item in &program.items {
        match item {
            Item::Interface(decl) => {
                for field in &decl.fields {
                    if let Some(default) = &field.default {
                        check_expr_comptime(default, &mut evaluator, &mut errors);
                    }
                }
            }
            Item::Function(decl) => {
                check_block_comptime(&decl.body, &mut evaluator, &mut errors);
            }
            _ => {}
        }
    }
    errors
}

fn function_names(program: &Program, only_comptime: bool) -> HashSet<&str> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(decl) if !only_comptime || decl.comptime => Some(decl.name.as_str()),
            _ => None,
        })
        .collect()
}

fn validate_block(
    block: &Block,
    globals: &HashSet<&str>,
    comptime_functions: &HashSet<&str>,
    all_functions: &HashSet<&str>,
    errors: &mut Vec<ConstError>,
) {
    for statement in &block.statements {
        validate_statement(
            statement,
            globals,
            comptime_functions,
            all_functions,
            errors,
        );
    }
}

fn validate_statement(
    statement: &Stmt,
    globals: &HashSet<&str>,
    comptime_functions: &HashSet<&str>,
    all_functions: &HashSet<&str>,
    errors: &mut Vec<ConstError>,
) {
    match &statement.kind {
        StmtKind::Let { value, .. } => {
            if let Some(value) = value {
                validate_expr(value, globals, comptime_functions, all_functions, errors);
            }
        }
        StmtKind::Assign { target, value, .. } => {
            if let Some(root) = place_root(target)
                && globals.contains(root.as_str())
            {
                errors.push(ConstError::new(
                    target.span,
                    "a comptime flow cannot mutate a global",
                ));
            }
            validate_expr(target, globals, comptime_functions, all_functions, errors);
            validate_expr(value, globals, comptime_functions, all_functions, errors);
        }
        StmtKind::Expr(expr) => {
            validate_expr(expr, globals, comptime_functions, all_functions, errors);
        }
        StmtKind::If {
            condition,
            then_block,
            else_branch,
        } => {
            validate_expr(
                condition,
                globals,
                comptime_functions,
                all_functions,
                errors,
            );
            validate_block(
                then_block,
                globals,
                comptime_functions,
                all_functions,
                errors,
            );
            match else_branch {
                Some(ElseBranch::Block(block)) => {
                    validate_block(block, globals, comptime_functions, all_functions, errors)
                }
                Some(ElseBranch::If(inner)) => {
                    validate_statement(inner, globals, comptime_functions, all_functions, errors)
                }
                None => {}
            }
        }
        StmtKind::While { condition, body } => {
            validate_expr(
                condition,
                globals,
                comptime_functions,
                all_functions,
                errors,
            );
            validate_block(body, globals, comptime_functions, all_functions, errors);
        }
        StmtKind::ForRange {
            start, end, body, ..
        } => {
            validate_expr(start, globals, comptime_functions, all_functions, errors);
            validate_expr(end, globals, comptime_functions, all_functions, errors);
            validate_block(body, globals, comptime_functions, all_functions, errors);
        }
        StmtKind::ForEach {
            direction,
            iterable,
            body,
            ..
        } => {
            if *direction != Direction::In {
                errors.push(ConstError::new(
                    statement.span,
                    "a comptime flow can only use '<-' iteration",
                ));
            }
            validate_expr(iterable, globals, comptime_functions, all_functions, errors);
            validate_block(body, globals, comptime_functions, all_functions, errors);
        }
        StmtKind::Protect(_) => errors.push(ConstError::new(
            statement.span,
            "'protect' is not allowed in a comptime flow",
        )),
        StmtKind::Return(value) => {
            if let Some(value) = value {
                validate_expr(value, globals, comptime_functions, all_functions, errors);
            }
        }
        StmtKind::Continue | StmtKind::Break => {}
    }
}

#[allow(clippy::only_used_in_recursion)]
fn validate_expr(
    expr: &Expr,
    globals: &HashSet<&str>,
    comptime_functions: &HashSet<&str>,
    all_functions: &HashSet<&str>,
    errors: &mut Vec<ConstError>,
) {
    match &expr.kind {
        ExprKind::Alloc { .. } => errors.push(ConstError::new(
            expr.span,
            "'alloc' is not allowed in a comptime flow",
        )),
        ExprKind::Call { callee, args, .. } => {
            if let ExprKind::Ident(name) = &callee.kind {
                if !PURE_INTRINSICS.contains(&name.as_str())
                    && !comptime_functions.contains(name.as_str())
                {
                    let detail = if all_functions.contains(name.as_str()) {
                        format!("flow '{name}' is not declared 'comptime'")
                    } else {
                        format!("call to '{name}' is not allowed in a comptime flow")
                    };
                    errors.push(ConstError::new(expr.span, detail));
                }
            } else {
                errors.push(ConstError::new(
                    expr.span,
                    "only named flows can be called in a comptime flow",
                ));
            }
            for arg in args {
                validate_expr(
                    &arg.value,
                    globals,
                    comptime_functions,
                    all_functions,
                    errors,
                );
            }
        }
        ExprKind::Unary { operand, .. } => {
            validate_expr(operand, globals, comptime_functions, all_functions, errors)
        }
        ExprKind::Binary { left, right, .. } => {
            validate_expr(left, globals, comptime_functions, all_functions, errors);
            validate_expr(right, globals, comptime_functions, all_functions, errors);
        }
        ExprKind::Cast { expr, .. } | ExprKind::Move(expr) | ExprKind::Copy(expr) => {
            validate_expr(expr, globals, comptime_functions, all_functions, errors)
        }
        ExprKind::Comptime(inner) => {
            validate_expr(inner, globals, comptime_functions, all_functions, errors)
        }
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                validate_expr(element, globals, comptime_functions, all_functions, errors);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                validate_expr(
                    &field.value,
                    globals,
                    comptime_functions,
                    all_functions,
                    errors,
                );
            }
        }
        ExprKind::Member { base, .. } => {
            validate_expr(base, globals, comptime_functions, all_functions, errors)
        }
        ExprKind::Index { base, index } => {
            validate_expr(base, globals, comptime_functions, all_functions, errors);
            validate_expr(index, globals, comptime_functions, all_functions, errors);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => {}
    }
}

fn check_block_comptime(
    block: &Block,
    evaluator: &mut ConstEvaluator<'_>,
    errors: &mut Vec<ConstError>,
) {
    for statement in &block.statements {
        match &statement.kind {
            StmtKind::Let { value, .. } => {
                if let Some(value) = value {
                    check_expr_comptime(value, evaluator, errors);
                }
            }
            StmtKind::Assign { target, value, .. } => {
                check_expr_comptime(target, evaluator, errors);
                check_expr_comptime(value, evaluator, errors);
            }
            StmtKind::Expr(expr) => check_expr_comptime(expr, evaluator, errors),
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => {
                check_expr_comptime(condition, evaluator, errors);
                check_block_comptime(then_block, evaluator, errors);
                match else_branch {
                    Some(ElseBranch::Block(block)) => {
                        check_block_comptime(block, evaluator, errors)
                    }
                    Some(ElseBranch::If(inner)) => {
                        check_statement_comptime(inner, evaluator, errors)
                    }
                    None => {}
                }
            }
            StmtKind::While { condition, body } => {
                check_expr_comptime(condition, evaluator, errors);
                check_block_comptime(body, evaluator, errors);
            }
            StmtKind::ForRange {
                start, end, body, ..
            } => {
                check_expr_comptime(start, evaluator, errors);
                check_expr_comptime(end, evaluator, errors);
                check_block_comptime(body, evaluator, errors);
            }
            StmtKind::ForEach { iterable, body, .. } => {
                check_expr_comptime(iterable, evaluator, errors);
                check_block_comptime(body, evaluator, errors);
            }
            StmtKind::Protect(block) => check_block_comptime(block, evaluator, errors),
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    check_expr_comptime(value, evaluator, errors);
                }
            }
            StmtKind::Continue | StmtKind::Break => {}
        }
    }
}

fn check_statement_comptime(
    statement: &Stmt,
    evaluator: &mut ConstEvaluator<'_>,
    errors: &mut Vec<ConstError>,
) {
    let block = Block {
        statements: vec![statement.clone()],
        open_line: statement.span.line,
        close_line: statement.span.line,
    };
    check_block_comptime(&block, evaluator, errors);
}

fn check_expr_comptime(
    expr: &Expr,
    evaluator: &mut ConstEvaluator<'_>,
    errors: &mut Vec<ConstError>,
) {
    if let ExprKind::Comptime(inner) = &expr.kind {
        if let Err(error) = evaluator.eval_expr(inner, None) {
            errors.push(error);
        }
        return;
    }
    match &expr.kind {
        ExprKind::SliceLiteral { elements, .. } => {
            for element in elements {
                check_expr_comptime(element, evaluator, errors);
            }
        }
        ExprKind::InterfaceLiteral { fields, .. } => {
            for field in fields {
                check_expr_comptime(&field.value, evaluator, errors);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Cast { expr: operand, .. }
        | ExprKind::Move(operand)
        | ExprKind::Copy(operand) => check_expr_comptime(operand, evaluator, errors),
        ExprKind::Binary { left, right, .. } => {
            check_expr_comptime(left, evaluator, errors);
            check_expr_comptime(right, evaluator, errors);
        }
        ExprKind::Call { callee, args, .. } => {
            check_expr_comptime(callee, evaluator, errors);
            for arg in args {
                check_expr_comptime(&arg.value, evaluator, errors);
            }
        }
        ExprKind::Alloc { target, count, .. } => {
            check_expr_comptime(target, evaluator, errors);
            check_expr_comptime(count, evaluator, errors);
        }
        ExprKind::Member { base, .. } => check_expr_comptime(base, evaluator, errors),
        ExprKind::Index { base, index } => {
            check_expr_comptime(base, evaluator, errors);
            check_expr_comptime(index, evaluator, errors);
        }
        ExprKind::Comptime(inner) => check_expr_comptime(inner, evaluator, errors),
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::String(_)
        | ExprKind::Ident(_) => {}
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

/// Whether `actual` can initialize a global declared as `expected`.
pub fn compatible_constant(expected: &Ty, actual: &Ty) -> bool {
    types::assignable(expected, actual)
}
