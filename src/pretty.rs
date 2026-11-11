//! Stable pretty-printer for the Loi AST.
//!
//! Formatting is deliberately simple: four-space indentation, one declaration
//! or statement per line, and expressions printed with the minimum parentheses
//! needed to preserve the AST. [`format_program_with_comments`] reattaches the
//! `//` comments recovered by the lexer, anchored to the source lines of the
//! declarations and statements they surround.

use crate::ast::*;
use crate::lexer::Comment;

const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_COMPARE: u8 = 3;
const PREC_BIT_OR: u8 = 4;
const PREC_BIT_XOR: u8 = 5;
const PREC_BIT_AND: u8 = 6;
const PREC_SHIFT: u8 = 7;
const PREC_ADD: u8 = 8;
const PREC_MUL: u8 = 9;
const PREC_CAST: u8 = 10;
const PREC_UNARY: u8 = 11;
const PREC_POSTFIX: u8 = 12;
const PREC_ATOM: u8 = 13;

/// Format a complete program as canonical Loi source.
pub fn format_program(program: &Program) -> String {
    PrettyPrinter::new().format(program)
}

/// Format a complete program while preserving `//` comments.
///
/// `comments` must be the list returned by [`crate::tokenize_with_comments`]
/// for the same source that produced `program`.
pub fn format_program_with_comments(program: &Program, comments: &[Comment]) -> String {
    PrettyPrinter::with_comments(comments.to_vec()).format(program)
}

/// Format a single expression.
pub fn format_expr(expr: &Expr) -> String {
    expr_str(expr, 0)
}

/// Format a type using Flow Syntax (`[T]`).
pub fn format_type(ty: &Type) -> String {
    match ty {
        Type::Named(name) => name.clone(),
        Type::Slice(element) => format!("[{}]", format_type(element)),
    }
}

struct PrettyPrinter {
    output: String,
    indent: usize,
    comments: Vec<Comment>,
    next_comment: usize,
}

impl PrettyPrinter {
    fn new() -> Self {
        Self::with_comments(Vec::new())
    }

    fn with_comments(comments: Vec<Comment>) -> Self {
        Self {
            output: String::new(),
            indent: 0,
            comments,
            next_comment: 0,
        }
    }

    fn format(mut self, program: &Program) -> String {
        for (index, item) in program.items.iter().enumerate() {
            if index > 0 {
                self.output.push('\n');
            }
            self.item(item);
        }
        self.flush_remaining_comments();
        self.output
    }

    fn line(&mut self, text: impl AsRef<str>) {
        for _ in 0..self.indent {
            self.output.push_str("    ");
        }
        self.output.push_str(text.as_ref());
        self.output.push('\n');
    }

    /// Emit a line anchored to `source_line`.
    ///
    /// Standalone comments before that source line are printed first, and a
    /// comment trailing code on the same source line is appended after it.
    fn line_at(&mut self, source_line: usize, text: impl AsRef<str>) {
        self.flush_comments_before(source_line);
        self.line(text);
        self.flush_trailing_comment(source_line);
    }

    fn flush_comments_before(&mut self, source_line: usize) {
        while let Some(comment) = self.comments.get(self.next_comment) {
            if comment.line >= source_line {
                break;
            }
            let text = comment.text.clone();
            self.next_comment += 1;
            self.line(text);
        }
    }

    fn flush_trailing_comment(&mut self, source_line: usize) {
        while let Some(comment) = self.comments.get(self.next_comment) {
            if comment.line != source_line || !comment.trailing {
                break;
            }
            let text = comment.text.clone();
            self.next_comment += 1;
            if self.output.ends_with('\n') {
                self.output.pop();
            }
            self.output.push(' ');
            self.output.push_str(&text);
            self.output.push('\n');
        }
    }

    fn flush_remaining_comments(&mut self) {
        while let Some(comment) = self.comments.get(self.next_comment) {
            let text = comment.text.clone();
            self.next_comment += 1;
            self.line(text);
        }
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Import(import) => self.line_at(
                import.span.line,
                format!("import \"{}\"", escape_string(&import.path)),
            ),
            Item::Interface(interface) => self.interface(interface),
            Item::Opaque(opaque) => self.opaque(opaque),
            Item::Global(global) => self.global(global),
            Item::Function(function) => self.function(function),
        }
    }

    fn global(&mut self, global: &GlobalDecl) {
        let mutable = if global.mutable { "mut " } else { "" };
        let ty = global
            .ty
            .as_ref()
            .map(|ty| format!(": {}", format_type(ty)))
            .unwrap_or_default();
        self.line_at(
            global.span.line,
            format!(
                "{}{}{}{} <- {}",
                visibility_prefix(global.visibility),
                mutable,
                global.name,
                ty,
                format_expr(&global.value)
            ),
        );
    }

    fn opaque(&mut self, opaque: &OpaqueDecl) {
        self.line_at(
            opaque.span.line,
            format!(
                "{}opaque {}",
                visibility_prefix(opaque.visibility),
                opaque.name
            ),
        );
    }

    fn interface(&mut self, interface: &InterfaceDecl) {
        if interface.fields.is_empty() {
            self.line_at(
                interface.span.line,
                format!(
                    "{}shape {} []",
                    visibility_prefix(interface.visibility),
                    interface.name
                ),
            );
            self.flush_comments_before(interface.close_line);
            return;
        }

        self.line_at(
            interface.span.line,
            format!(
                "{}shape {} [",
                visibility_prefix(interface.visibility),
                interface.name
            ),
        );
        self.indent += 1;
        for field in &interface.fields {
            let mut text = format!("{}: {}", field.name, format_type(&field.ty));
            if let Some(default) = &field.default {
                text.push_str(" = ");
                text.push_str(&format_expr(default));
            }
            self.line_at(field.span.line, text);
        }
        self.flush_comments_before(interface.close_line);
        self.indent -= 1;
        self.line_at(interface.close_line, "]");
    }

    fn function(&mut self, function: &FunctionDecl) {
        let prefix = if function.external { "extern " } else { "" };
        let link_name = function
            .link_name
            .as_ref()
            .map(|symbol| format!(" as {symbol}"))
            .unwrap_or_default();
        let header = format!(
            "{}{}flow {}{link_name}",
            visibility_prefix(function.visibility),
            prefix,
            function.name
        );

        if function.external {
            self.extern_function(&header, function);
            return;
        }

        if function.params.is_empty() && function.return_type.is_none() {
            self.line_at(function.span.line, format!("{header} {{"));
            self.indent += 1;
            self.statements(&function.body);
            self.indent -= 1;
            self.line_at(function.body.close_line, "}");
            return;
        }

        self.line_at(function.span.line, header);
        self.indent += 1;
        for param in &function.params {
            self.line_at(
                param.span.line,
                format!(
                    "{} {}: {}",
                    direction_glyph(param.direction),
                    param.name,
                    format_type(&param.ty)
                ),
            );
        }
        if let Some(return_type) = &function.return_type {
            self.line(format!("=> {}", format_type(return_type)));
        }
        self.indent -= 1;
        self.line_at(function.body.open_line, "{");
        self.indent += 1;
        self.statements(&function.body);
        self.indent -= 1;
        self.line_at(function.body.close_line, "}");
    }

    fn extern_function(&mut self, header: &str, function: &FunctionDecl) {
        if function.params.is_empty() && function.return_type.is_none() {
            self.line_at(function.span.line, header);
            return;
        }

        self.line_at(function.span.line, header);
        self.indent += 1;
        for param in &function.params {
            self.line_at(
                param.span.line,
                format!(
                    "{} {}: {}",
                    direction_glyph(param.direction),
                    param.name,
                    format_type(&param.ty)
                ),
            );
        }
        if let Some(return_type) = &function.return_type {
            self.line(format!("=> {}", format_type(return_type)));
        }
        self.indent -= 1;
    }

    fn statements(&mut self, block: &Block) {
        for statement in &block.statements {
            self.statement(statement);
        }
        self.flush_comments_before(block.close_line);
    }

    fn statement(&mut self, statement: &Stmt) {
        let source_line = statement.span.line;
        match &statement.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let mut text = if *mutable {
                    format!("mut {name}")
                } else {
                    name.clone()
                };
                if let Some(ty) = ty {
                    text.push_str(": ");
                    text.push_str(&format_type(ty));
                }
                if let Some(value) = value {
                    text.push_str(" <- ");
                    text.push_str(&format_expr(value));
                }
                self.line_at(source_line, text);
            }
            StmtKind::Assign { target, op, value } => {
                self.line_at(
                    source_line,
                    format!(
                        "{} {} {}",
                        format_expr(target),
                        assign_op_name(*op),
                        format_expr(value)
                    ),
                );
            }
            StmtKind::Expr(expr) => self.line_at(source_line, format_expr(expr)),
            StmtKind::If {
                condition,
                then_block,
                else_branch,
            } => self.if_statement(condition, then_block, else_branch, "", source_line),
            StmtKind::While { condition, body } => {
                self.line_at(source_line, format!("while {} {{", expr_str(condition, 0)));
                self.indent += 1;
                self.statements(body);
                self.indent -= 1;
                self.line_at(body.close_line, "}");
            }
            StmtKind::ForRange {
                variable,
                start,
                end,
                body,
            } => {
                self.line_at(
                    source_line,
                    format!(
                        "for {} in {}..{} {{",
                        variable,
                        expr_str(start, 0),
                        expr_str(end, 0)
                    ),
                );
                self.indent += 1;
                self.statements(body);
                self.indent -= 1;
                self.line_at(body.close_line, "}");
            }
            StmtKind::ForEach {
                direction,
                variable,
                iterable,
                body,
            } => {
                self.line_at(
                    source_line,
                    format!(
                        "for {} {} in {} {{",
                        direction_glyph(*direction),
                        variable,
                        expr_str(iterable, 0)
                    ),
                );
                self.indent += 1;
                self.statements(body);
                self.indent -= 1;
                self.line_at(body.close_line, "}");
            }
            StmtKind::Protect(body) => {
                self.line_at(source_line, "protect {");
                self.indent += 1;
                self.statements(body);
                self.indent -= 1;
                self.line_at(body.close_line, "}");
            }
            StmtKind::Return(value) => {
                let text = match value {
                    Some(value) => format!("=> {}", format_expr(value)),
                    None => "=>".to_string(),
                };
                self.line_at(source_line, text);
            }
            StmtKind::Continue => self.line_at(source_line, "continue"),
            StmtKind::Break => self.line_at(source_line, "break"),
        }
    }

    fn if_statement(
        &mut self,
        condition: &Expr,
        then_block: &Block,
        else_branch: &Option<ElseBranch>,
        prefix: &str,
        source_line: usize,
    ) {
        self.line_at(
            source_line,
            format!("{}if {} {{", prefix, expr_str(condition, 0)),
        );
        self.indent += 1;
        self.statements(then_block);
        self.indent -= 1;

        match else_branch {
            None => self.line_at(then_block.close_line, "}"),
            Some(ElseBranch::Block(block)) => {
                self.line_at(then_block.close_line, "} else {");
                self.indent += 1;
                self.statements(block);
                self.indent -= 1;
                self.line_at(block.close_line, "}");
            }
            Some(ElseBranch::If(inner)) => {
                if let StmtKind::If {
                    condition,
                    then_block,
                    else_branch,
                } = &inner.as_ref().kind
                {
                    self.if_statement(
                        condition,
                        then_block,
                        else_branch,
                        "} else ",
                        then_block.close_line,
                    );
                }
            }
        }
    }
}

fn escape_string(text: &str) -> String {
    let mut escaped = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            other if (other as u32) < 0x20 || other == '\u{7f}' => {
                escaped.push_str(&format!("\\x{:02x}", other as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

fn visibility_prefix(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Visible => "open ",
        Visibility::Hidden => "",
    }
}

fn direction_glyph(direction: Direction) -> &'static str {
    match direction {
        Direction::In => "<-",
        Direction::Out => "->",
        Direction::Inout => "<->",
    }
}

fn assign_op_name(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign => "=",
        AssignOp::Add => "+=",
        AssignOp::Sub => "-=",
        AssignOp::Mul => "*=",
        AssignOp::Div => "/=",
        AssignOp::Mod => "%=",
        AssignOp::BitAnd => "&=",
        AssignOp::BitOr => "|=",
        AssignOp::BitXor => "^=",
        AssignOp::Shl => "<<=",
        AssignOp::Shr => ">>=",
    }
}

fn binary_op_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Or => "or",
        BinaryOp::And => "and",
        BinaryOp::Eq => "==",
        BinaryOp::Neq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Lte => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Gte => ">=",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::BitAnd => "&",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
    }
}

fn binary_op_precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Or => PREC_OR,
        BinaryOp::And => PREC_AND,
        BinaryOp::Eq
        | BinaryOp::Neq
        | BinaryOp::Lt
        | BinaryOp::Lte
        | BinaryOp::Gt
        | BinaryOp::Gte => PREC_COMPARE,
        BinaryOp::BitOr => PREC_BIT_OR,
        BinaryOp::BitXor => PREC_BIT_XOR,
        BinaryOp::BitAnd => PREC_BIT_AND,
        BinaryOp::Shl | BinaryOp::Shr => PREC_SHIFT,
        BinaryOp::Add | BinaryOp::Sub => PREC_ADD,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => PREC_MUL,
    }
}

fn parenthesize(text: String, precedence: u8, parent_precedence: u8) -> String {
    if precedence < parent_precedence {
        format!("({text})")
    } else {
        text
    }
}

fn expr_str(expr: &Expr, parent_precedence: u8) -> String {
    match &expr.kind {
        ExprKind::Int(text) | ExprKind::Float(text) | ExprKind::Ident(text) => text.clone(),
        ExprKind::String(text) => format!("\"{}\"", escape_string(text)),
        ExprKind::Bool(value) => value.to_string(),
        ExprKind::SliceLiteral {
            element_type,
            elements,
        } => {
            let elements = elements
                .iter()
                .map(|element| expr_str(element, 0))
                .collect::<Vec<_>>()
                .join(", ");
            match element_type {
                Some(ty) => {
                    if elements.is_empty() {
                        format!("[{}:]", format_type(ty))
                    } else {
                        format!("[{}: {elements}]", format_type(ty))
                    }
                }
                None => format!("[{elements}]"),
            }
        }
        ExprKind::InterfaceLiteral { name, fields } => {
            if fields.is_empty() {
                format!("{name}[]")
            } else {
                let fields = fields
                    .iter()
                    .map(|field| format!("{}: {}", field.name, expr_str(&field.value, 0)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}[ {fields} ]")
            }
        }
        ExprKind::Unary { op, operand } => {
            let text = match op {
                UnaryOp::Neg => format!("-{}", expr_str(operand, PREC_UNARY)),
                UnaryOp::Not => format!("not {}", expr_str(operand, PREC_UNARY)),
                UnaryOp::BitNot => format!("~{}", expr_str(operand, PREC_UNARY)),
            };
            parenthesize(text, PREC_UNARY, parent_precedence)
        }
        ExprKind::Binary { op, left, right } => {
            let precedence = binary_op_precedence(*op);
            let text = format!(
                "{} {} {}",
                expr_str(left, precedence),
                binary_op_name(*op),
                expr_str(right, precedence + 1)
            );
            parenthesize(text, precedence, parent_precedence)
        }
        ExprKind::Cast { expr, ty } => {
            let text = format!("{} as {}", expr_str(expr, PREC_CAST + 1), format_type(ty));
            parenthesize(text, PREC_CAST, parent_precedence)
        }
        ExprKind::Call { callee, args } => {
            let args = args
                .iter()
                .map(|arg| {
                    let value = expr_str(&arg.value, 0);
                    match arg.direction {
                        Direction::In => value,
                        Direction::Out => format!("-> {value}"),
                        Direction::Inout => format!("<-> {value}"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let text = format!("{}({})", expr_str(callee, PREC_POSTFIX), args);
            parenthesize(text, PREC_POSTFIX, parent_precedence)
        }
        ExprKind::Alloc {
            target,
            element_type,
            count,
        } => {
            let text = format!(
                "alloc(-> {}, {}, {})",
                expr_str(target, 0),
                format_type(element_type),
                expr_str(count, 0)
            );
            parenthesize(text, PREC_ATOM, parent_precedence)
        }
        ExprKind::Move(value) => {
            let text = format!("give({})", expr_str(value, 0));
            parenthesize(text, PREC_ATOM, parent_precedence)
        }
        ExprKind::Copy(value) => {
            let text = format!("clone({})", expr_str(value, 0));
            parenthesize(text, PREC_ATOM, parent_precedence)
        }
        ExprKind::Member { base, field } => {
            let text = format!("{}.{}", expr_str(base, PREC_POSTFIX), field);
            parenthesize(text, PREC_POSTFIX, parent_precedence)
        }
        ExprKind::Index { base, index } => {
            let text = format!("{}[{}]", expr_str(base, PREC_POSTFIX), expr_str(index, 0));
            parenthesize(text, PREC_POSTFIX, parent_precedence)
        }
    }
}
