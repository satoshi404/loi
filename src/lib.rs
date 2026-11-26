//! Loi compiler.

pub mod asm;
pub mod ast;
pub mod checker;
pub mod codegen;
pub mod complete;
pub mod const_eval;
pub mod diagnostic;
pub mod expand;
pub mod flow;
pub mod format;
pub mod imports;
pub mod ir;
pub mod lexer;
pub mod lint;
pub mod llvm;
pub mod lsp_info;
pub mod modules;
pub mod monomorphize;
pub mod parser;
pub mod pretty;
pub mod token;
pub mod types;

pub use asm::{generate_asm, generate_asm_from_program};
pub use ast::*;
pub use checker::{check, check_with_warnings, check_with_warnings_from};
pub use codegen::{
    CodegenError, generate_c, generate_c_subset, generate_header, generate_header_named,
    generate_ir,
};
pub use complete::{CompletionItem, completions, to_json as completions_to_json};
pub use diagnostic::{ColorChoice, Diagnostic, ErrorFormat, Severity};
pub use expand::expand_program;
pub use ir::{IrBuilder, IrModule};
pub use lexer::{Comment, LexError, Lexer, tokenize, tokenize_with_comments};
pub use llvm::generate_llvm_ir;
pub use lsp_info::{InfoKind, info_json as lsp_info_json};
pub use monomorphize::{instantiation_name, monomorphize, monomorphize_for_check};
pub use parser::{ParseError, Parser, parse};
pub use pretty::{format_expr, format_program, format_program_with_comments, format_type};
pub use token::{Token, TokenKind};
pub use types::{CallbackSig, Ty, assignable, common_numeric};
