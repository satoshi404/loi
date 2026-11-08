//! x86-64 assembly backend for Loi.
//!
//! Lowers an IR module to x86-64 assembly (AT&T syntax, ELF). This is a
//! minimal emitter: it supports the common integer and memory operations
//! needed by Loi programs, calling conventions for extern functions, and
//! basic control flow.

use crate::ast;
use crate::ir;

/// Emit an IR module as x86-64 assembly text.
pub fn generate_asm(module: &ir::IrModule) -> String {
    let mut out = String::new();

    out.push_str("\t.text\n");
    out.push_str("\t.globl\t_main\n");
    out.push('\n');

    for func in &module.functions {
        out.push_str(&emit_function(func));
    }

    out
}

/// Emit a single IR function as assembly.
fn emit_function(func: &ir::IrFunction) -> String {
    let mut out = String::new();

    let visibility = if func.visibility == crate::ast::Visibility::Visible {
        ".globl\t"
    } else {
        "\t.local\t"
    };
    out.push_str(&format!("{visibility}_{}\n", func.name));
    out.push_str(&format!("\t.type\t{}, @function\n", func.name));
    out.push_str(&format!("{}:\n", func.name));
    out.push_str("\tpushq\t%rbp\n");
    out.push_str("\tmovq\t%rsp, %rbp\n");

    // Emit prologue: stack space for locals (simple: 256 bytes)
    out.push_str("\tsubq\t$256, %rsp\n");

    for block in &func.blocks {
        out.push_str(&format!("{}:\n", block.label));
        for inst in &block.insts {
            out.push_str(&format!("\t{}\n", emit_inst(inst)));
        }
    }

    out.push_str("\tmovq\t%rbp, %rsp\n");
    out.push_str("\tpopq\t%rbp\n");
    out.push_str("\tret\n");
    out.push_str(&format!("\t.size\t{}, .-_{}\n", func.name, func.name));
    out.push('\n');

    out
}

/// Emit a single IR instruction as assembly.
fn emit_inst(inst: &ir::IrInst) -> String {
    match inst {
        ir::IrInst::Alloca(reg, ty) => {
            let size = ty.size_bytes();
            format!("\tleaq\t-{}(%rbp), {}\t# alloca {}", size, reg, ty)
        }
        ir::IrInst::Load(ty, reg, ptr) => {
            format!("\tmovq\t({}), {}\t# load {}", ptr, reg, ty)
        }
        ir::IrInst::Store(ty, ptr, val) => {
            format!("\tmovq\t{}, ({})\t# store {}", val, ptr, ty)
        }
        ir::IrInst::BinOp(op, ty, dst, lhs, _rhs) => {
            let op_asm = match op {
                ir::BinOp::Add => "addq",
                ir::BinOp::Sub => "subq",
                ir::BinOp::Mul => "imulq",
                ir::BinOp::Div => "idivq",
                ir::BinOp::And => "andq",
                ir::BinOp::Or => "orq",
                ir::BinOp::Xor => "xorq",
                ir::BinOp::Shl => "salq",
                ir::BinOp::LShr => "sarq",
                ir::BinOp::AShr => "sarq",
                _ => "addq",
            };
            format!("\t{} {}, {}\t# {}", op_asm, lhs, dst, ty)
        }
        ir::IrInst::UnOp(op, ty, dst, operand) => {
            let op_asm = match op {
                ir::UnOp::Neg => "negq",
                ir::UnOp::Not => "notq",
                ir::UnOp::FNeg => "negq",
                ir::UnOp::BitNot => "notq",
            };
            format!("\t{} {}, {}\t# {}", op_asm, operand, dst, ty)
        }
        ir::IrInst::Cmp(op, _ty, dst, lhs, rhs) => {
            let cmp_asm = match op {
                ir::CmpOp::Eq => "cmpeq",
                ir::CmpOp::Ne => "cmpne",
                ir::CmpOp::Slt => "cmplt",
                ir::CmpOp::Sle => "cmple",
                ir::CmpOp::Sgt => "cmpgt",
                ir::CmpOp::Sge => "cmpge",
                _ => "cmpq",
            };
            format!(
                "\t{} {}, {}\n\tmovq\t$0, {}\n\tcmov{}, {}, {}\t# cmp",
                cmp_asm,
                lhs,
                rhs,
                dst,
                op_cond(op),
                lhs,
                dst
            )
        }
        ir::IrInst::Call(reg, name, args, _ret_ty) => {
            let mut asm = String::new();
            // Push args (simplified: just push in reverse)
            for arg in args.iter().rev() {
                asm.push_str(&format!("\tpushq\t{}\n", arg));
            }
            asm.push_str(&format!("\tcall\t_{}\n", name));
            // Clean up stack
            if !args.is_empty() {
                asm.push_str(&format!("\taddq\t${}, %rsp\n", args.len() * 8));
            }
            asm.push_str(&format!("\tmovq\t%rax, {}\t# call result", *reg));
            asm
        }
        ir::IrInst::Return(Some(reg)) => {
            format!("\tmovq\t{}, %rax\n\tret", reg)
        }
        ir::IrInst::Return(None) => "\tret".to_string(),
        ir::IrInst::Jump(label) => {
            format!("\tjmp\t{}", label)
        }
        ir::IrInst::CondJump(cond, true_label, false_label) => {
            format!(
                "\ttestq\t{}, {}\n\tje\t{}\n\tjmp\t{}",
                cond, cond, true_label, false_label
            )
        }
        ir::IrInst::Label(label) => {
            format!("{}:", label)
        }
        ir::IrInst::Phi(reg, ty, _incoming) => {
            format!("\t# phi {}\t= phi {}", reg, ty)
        }
        ir::IrInst::BitCast(reg, _src_ty, dst_ty, operand) => {
            format!("\tmovq\t{}, {}\t# bitcast {}", operand, reg, dst_ty)
        }
        ir::IrInst::IntToPtr(reg, _src_ty, _dst_ty, operand) => {
            format!("\tmovq\t{}, {}\t# inttoptr", operand, reg)
        }
        ir::IrInst::PtrToInt(reg, _src_ty, _dst_ty, operand) => {
            format!("\tmovq\t{}, {}\t# ptrtoint", operand, reg)
        }
        ir::IrInst::AllocSlice(reg, _elem_ty, count, _len) => {
            format!("\tmovq\t{}, {}\t# allocslice", count, reg)
        }
        ir::IrInst::Imm(reg, _ty, value) => {
            format!("	movq	${}, {}	# imm", value, reg)
        }
        ir::IrInst::GetElemPtr(reg, _elem_ty, base, index) => {
            format!("\tleaq\t({}, {}), {}\t# getelementptr", base, index, reg)
        }
        ir::IrInst::IndexSlice(reg, _elem_ty, slice, index) => {
            format!("\tleaq\t({}, {}), {}\t# indexslice", slice, index, reg)
        }
    }
}

/// Map a comparison operator to a cmov condition suffix.
fn op_cond(op: &ir::CmpOp) -> &'static str {
    match op {
        ir::CmpOp::Eq => "e",
        ir::CmpOp::Ne => "ne",
        ir::CmpOp::Slt => "l",
        ir::CmpOp::Sle => "le",
        ir::CmpOp::Sgt => "g",
        ir::CmpOp::Sge => "ge",
        ir::CmpOp::Ult => "b",
        ir::CmpOp::Ule => "be",
        ir::CmpOp::Ugt => "a",
        ir::CmpOp::Uge => "ae",
        ir::CmpOp::Flt => "b",
        ir::CmpOp::Fle => "be",
        ir::CmpOp::Fgt => "a",
        ir::CmpOp::Fge => "ae",
    }
}

/// Generate x86-64 assembly from a checked Loi [`Program`].
pub fn generate_asm_from_program(program: &ast::Program) -> Result<String, String> {
    let ir_module = ir::program_to_ir_module(program)?;
    Ok(generate_asm(&ir_module))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asm_emit() {
        let mut builder = ir::IrBuilder::new();
        builder.start_function(
            "main",
            crate::ast::Visibility::Hidden,
            Vec::new(),
            ir::IrType::Int(32),
        );
        builder.start_block("entry");
        builder.emit_inst(ir::IrInst::Return(Some(ir::Reg(1))));
        builder.finish_block();
        builder.finish_function();

        let asm = generate_asm(builder.module());
        assert!(asm.contains(".text"));
        assert!(asm.contains("main:"));
        assert!(asm.contains(".globl"));
    }
}
