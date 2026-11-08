use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reg(pub u32);

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IrType {
    Int(u32),
    Uint(u32),
    Float(u32),
    Bool,
    Ptr(Box<IrType>),
    Slice(Box<IrType>),
    Void,
    Named(String),
    Array(Box<IrType>, u64),
}

impl IrType {
    pub fn size_bytes(&self) -> u64 {
        match self {
            IrType::Int(8) | IrType::Uint(8) | IrType::Bool => 1,
            IrType::Int(16) | IrType::Uint(16) => 2,
            IrType::Int(32) | IrType::Uint(32) | IrType::Float(32) => 4,
            IrType::Int(64) | IrType::Uint(64) | IrType::Float(64) => 8,
            IrType::Ptr(_) => 8,
            IrType::Slice(_) => 16,
            IrType::Void => 0,
            IrType::Named(_) => 0,
            IrType::Array(inner, len) => inner.size_bytes() * len,
            _ => 0,
        }
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, IrType::Int(_) | IrType::Uint(_))
    }

    pub fn is_float(&self) -> bool {
        matches!(self, IrType::Float(_))
    }

    pub fn is_pointer(&self) -> bool {
        matches!(self, IrType::Ptr(_))
    }
}

impl fmt::Display for IrType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrType::Int(8) => write!(f, "i8"),
            IrType::Int(16) => write!(f, "i16"),
            IrType::Int(32) => write!(f, "i32"),
            IrType::Int(64) => write!(f, "i64"),
            IrType::Uint(8) => write!(f, "u8"),
            IrType::Uint(16) => write!(f, "u16"),
            IrType::Uint(32) => write!(f, "u32"),
            IrType::Uint(64) => write!(f, "u64"),
            IrType::Float(32) => write!(f, "f32"),
            IrType::Float(64) => write!(f, "f64"),
            IrType::Bool => write!(f, "i1"),
            IrType::Ptr(inner) => write!(f, "ptr<{inner}>"),
            IrType::Slice(inner) => write!(f, "slice<{inner}>"),
            IrType::Void => write!(f, "void"),
            IrType::Named(name) => write!(f, "%{name}"),
            IrType::Array(inner, len) => write!(f, "[{inner} x {len}]"),
            IrType::Int(bits) | IrType::Uint(bits) | IrType::Float(bits) => {
                write!(f, "type{bits}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum IrInst {
    Alloca(Reg, IrType),
    Load(Reg, IrType, Reg),
    Store(IrType, Reg, Reg),
    GetElemPtr(Reg, IrType, Reg, Reg),
    BinOp(BinOp, IrType, Reg, Reg, Reg),
    UnOp(UnOp, IrType, Reg, Reg),
    Cmp(CmpOp, IrType, Reg, Reg, Reg),
    Call(Reg, String, Vec<Reg>, IrType),
    Return(Option<Reg>),
    Jump(String),
    CondJump(Reg, String, String),
    Label(String),
    Phi(Reg, IrType, Vec<(Reg, String)>),
    BitCast(Reg, IrType, IrType, Reg),
    IntToPtr(Reg, IrType, IrType, Reg),
    PtrToInt(Reg, IrType, IrType, Reg),
    AllocSlice(Reg, IrType, Reg, Reg),
    IndexSlice(Reg, IrType, Reg, Reg),
    Imm(Reg, IrType, i128),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    SDiv,
    UDiv,
    SRem,
    URem,
    And,
    Or,
    Xor,
    Shl,
    LShr,
    AShr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    FNeg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
    Flt,
    Fle,
    Fgt,
    Fge,
}

impl fmt::Display for IrInst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrInst::Alloca(reg, ty) => write!(f, "{reg} = alloca {ty}"),
            IrInst::Load(reg, ty, ptr) => write!(f, "{reg} = load {ty}, ptr {ptr}"),
            IrInst::Store(ty, ptr, val) => write!(f, "store {ty} {val}, ptr {ptr}"),
            IrInst::GetElemPtr(reg, ty, ptr, idx) => {
                write!(f, "{reg} = getelementptr {ty}, ptr {ptr}, i64 {idx}")
            }
            IrInst::BinOp(op, ty, dst, lhs, rhs) => {
                let op_str = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "mul",
                    BinOp::Div => "div",
                    BinOp::SDiv => "sdiv",
                    BinOp::UDiv => "udiv",
                    BinOp::SRem => "srem",
                    BinOp::URem => "urem",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    BinOp::Xor => "xor",
                    BinOp::Shl => "shl",
                    BinOp::LShr => "lshr",
                    BinOp::AShr => "ashr",
                };
                write!(f, "{dst} = {op_str} {ty} {lhs}, {rhs}")
            }
            IrInst::UnOp(op, ty, dst, operand) => {
                let op_str = match op {
                    UnOp::Neg => "neg",
                    UnOp::FNeg => "fneg",
                    UnOp::Not => "not",
                    UnOp::BitNot => "bitnot",
                };
                write!(f, "{dst} = {op_str} {ty} {operand}")
            }
            IrInst::Cmp(op, ty, dst, lhs, rhs) => {
                let op_str = match op {
                    CmpOp::Eq => "eq",
                    CmpOp::Ne => "ne",
                    CmpOp::Slt => "slt",
                    CmpOp::Sle => "sle",
                    CmpOp::Sgt => "sgt",
                    CmpOp::Sge => "sge",
                    CmpOp::Ult => "ult",
                    CmpOp::Ule => "ule",
                    CmpOp::Ugt => "ugt",
                    CmpOp::Uge => "uge",
                    CmpOp::Flt => "flt",
                    CmpOp::Fle => "fle",
                    CmpOp::Fgt => "fgt",
                    CmpOp::Fge => "fge",
                };
                write!(f, "{dst} = {op_str} {ty} {lhs}, {rhs}")
            }
            IrInst::Call(reg, name, args, _ret_ty) => {
                let args_str = args
                    .iter()
                    .map(|a| format!("{a}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                if reg.0 == 0 {
                    write!(f, "call {name}({args_str})")
                } else {
                    write!(f, "{reg} = call {name}({args_str})")
                }
            }
            IrInst::Return(Some(reg)) => write!(f, "ret {reg}"),
            IrInst::Return(None) => write!(f, "ret void"),
            IrInst::Jump(label) => write!(f, "jump {label}"),
            IrInst::CondJump(cond, true_label, false_label) => {
                write!(f, "condjump {cond}, {true_label}, {false_label}")
            }
            IrInst::Label(label) => write!(f, "{label}:"),
            IrInst::Phi(reg, ty, incoming) => {
                let inc_str = incoming
                    .iter()
                    .map(|(v, l)| format!("[{v}, {l}]"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{reg} = phi {ty} {inc_str}")
            }
            IrInst::BitCast(reg, src_ty, dst_ty, operand) => {
                write!(f, "{reg} = bitcast {src_ty} {operand} to {dst_ty}")
            }
            IrInst::IntToPtr(reg, src_ty, dst_ty, operand) => {
                write!(f, "{reg} = inttoptr {src_ty} {operand} to {dst_ty}")
            }
            IrInst::PtrToInt(reg, src_ty, dst_ty, operand) => {
                write!(f, "{reg} = ptrtoint {src_ty} {operand} to {dst_ty}")
            }
            IrInst::AllocSlice(reg, elem_ty, count, len) => {
                write!(f, "{reg} = allocslice {elem_ty}, i64 {count}, i64 {len}")
            }
            IrInst::Imm(reg, ty, value) => {
                write!(f, "{reg} = imm {ty}, {value}")
            }
            IrInst::IndexSlice(reg, elem_ty, slice, index) => {
                write!(f, "{reg} = indexslice {elem_ty}, {slice}, i64 {index}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct IrBlock {
    pub label: String,
    pub insts: Vec<IrInst>,
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub visibility: crate::ast::Visibility,
    pub params: Vec<(String, IrType)>,
    pub return_ty: IrType,
    pub blocks: Vec<IrBlock>,
}

#[derive(Debug, Clone)]
pub struct IrModule {
    pub functions: Vec<IrFunction>,
    pub interfaces: Vec<(String, Vec<(String, IrType)>)>,
}

impl Default for IrModule {
    fn default() -> Self {
        Self::new()
    }
}

impl IrModule {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            interfaces: Vec::new(),
        }
    }

    pub fn emit(&self) -> String {
        let mut out = String::new();
        out.push_str("; Module generated by the Loi compiler\n");
        out.push_str("; Target: x86-64\n\n");

        for (name, fields) in &self.interfaces {
            out.push_str(&format!("%{name} = type {{"));
            for (i, (_fname, fty)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{fty}"));
            }
            out.push_str("}\n");
        }
        out.push('\n');

        for func in &self.functions {
            out.push_str(&format!("define {} @{}(", func.return_ty, func.name));
            for (i, (pname, pty)) in func.params.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("{pty} %{pname}"));
            }
            out.push_str(") {\n");

            for block in &func.blocks {
                out.push_str(&format!("{}:\n", block.label));
                for inst in &block.insts {
                    out.push_str(&format!("  {inst}\n"));
                }
            }

            out.push_str("}\n\n");
        }

        out
    }
}

impl fmt::Display for IrModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.emit())
    }
}

pub struct IrBuilder {
    next_reg: u32,
    next_block: u32,
    module: IrModule,
    current_function: Option<IrFunction>,
    current_block: Option<IrBlock>,
    /// Maps variable names to their allocated registers
    var_regs: HashMap<String, Reg>,
}

impl Default for IrBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IrBuilder {
    pub fn new() -> Self {
        Self {
            next_reg: 1,
            next_block: 1,
            module: IrModule::new(),
            current_function: None,
            current_block: None,
            var_regs: HashMap::new(),
        }
    }

    pub fn fresh_reg(&mut self) -> Reg {
        let reg = Reg(self.next_reg);
        self.next_reg += 1;
        reg
    }

    pub fn fresh_block(&mut self) -> String {
        let label = format!("bb{}", self.next_block);
        self.next_block += 1;
        label
    }

    pub fn module(&self) -> &IrModule {
        &self.module
    }

    pub fn module_mut(&mut self) -> &mut IrModule {
        &mut self.module
    }

    pub fn start_function(
        &mut self,
        name: &str,
        visibility: crate::ast::Visibility,
        params: Vec<(String, IrType)>,
        return_ty: IrType,
    ) {
        self.current_function = Some(IrFunction {
            name: name.to_string(),
            visibility,
            params,
            return_ty,
            blocks: Vec::new(),
        });
        self.current_block = None;
    }

    pub fn start_block(&mut self, label: &str) {
        self.current_block = Some(IrBlock {
            label: label.to_string(),
            insts: Vec::new(),
        });
    }

    pub fn emit_inst(&mut self, inst: IrInst) {
        if let Some(block) = self.current_block.as_mut() {
            block.insts.push(inst);
        } else {
            panic!("emit_inst called without an active block");
        }
    }

    pub fn finish_block(&mut self) {
        if let Some(block) = self.current_block.take()
            && let Some(func) = self.current_function.as_mut()
        {
            func.blocks.push(block);
        }
    }

    pub fn finish_function(&mut self) {
        if let Some(func) = self.current_function.take() {
            self.module.functions.push(func);
        }
    }

    pub fn add_interface(&mut self, name: String, fields: Vec<(String, IrType)>) {
        self.module.interfaces.push((name, fields));
    }

    /// Register a variable's register allocation.
    pub fn set_var_reg(&mut self, name: &str, reg: Reg) {
        self.var_regs.insert(name.to_string(), reg);
    }

    /// Get a variable's register, or create a new one if not yet allocated.
    pub fn get_var_reg(&mut self, name: &str) -> Reg {
        if let Some(reg) = self.var_regs.get(name) {
            *reg
        } else {
            let reg = self.fresh_reg();
            self.var_regs.insert(name.to_string(), reg);
            reg
        }
    }
}

pub fn program_to_ir_module(program: &crate::ast::Program) -> Result<IrModule, String> {
    let mut builder = IrBuilder::new();

    for item in &program.items {
        if let crate::ast::Item::Interface(decl) = item {
            let mut fields = Vec::new();
            for field in &decl.fields {
                let ir_ty = ast_type_to_ir(&field.ty)?;
                fields.push((field.name.clone(), ir_ty));
            }
            builder.add_interface(decl.name.clone(), fields);
        }
    }

    for item in &program.items {
        if let crate::ast::Item::Function(decl) = item {
            let mut params = Vec::new();
            for param in &decl.params {
                let ir_ty = ast_type_to_ir(&param.ty)?;
                params.push((param.name.clone(), ir_ty));
            }
            let return_ty = match &decl.return_type {
                Some(ty) => ast_type_to_ir(ty)?,
                None => IrType::Void,
            };
            builder.start_function(&decl.name, decl.visibility, params, return_ty);
            builder.start_block("entry");
            builder.emit_inst(IrInst::Return(None));
            builder.finish_block();
            builder.finish_function();
        }
    }

    Ok(builder.module().clone())
}

fn ast_type_to_ir(ty: &crate::ast::Type) -> Result<IrType, String> {
    match ty {
        crate::ast::Type::Named(name) => {
            if let Some(ir_ty) = resolve_primitive(name) {
                Ok(ir_ty)
            } else {
                Ok(IrType::Named(name.clone()))
            }
        }
        crate::ast::Type::Slice(element) => {
            let elem_ty = ast_type_to_ir(element)?;
            Ok(IrType::Slice(Box::new(elem_ty)))
        }
    }
}

fn resolve_primitive(name: &str) -> Option<IrType> {
    match name {
        "i8" => Some(IrType::Int(8)),
        "i16" => Some(IrType::Int(16)),
        "i32" => Some(IrType::Int(32)),
        "i64" => Some(IrType::Int(64)),
        "u8" => Some(IrType::Uint(8)),
        "u16" => Some(IrType::Uint(16)),
        "u32" => Some(IrType::Uint(32)),
        "u64" => Some(IrType::Uint(64)),
        "f32" => Some(IrType::Float(32)),
        "f64" => Some(IrType::Float(64)),
        "bool" => Some(IrType::Bool),
        "void" => Some(IrType::Void),
        _ => None,
    }
}
