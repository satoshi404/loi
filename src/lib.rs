//! Loi compiler.

pub mod asm;
pub mod ast;
pub mod checker;
pub mod codegen;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod pretty;
pub mod token;
pub mod types;

pub use asm::{generate_asm, generate_asm_from_program};
pub use ast::*;
pub use checker::{Diagnostic, check};
pub use codegen::{CodegenError, generate_c, generate_c_subset, generate_header, generate_ir};
pub use ir::{IrBuilder, IrModule};
pub use lexer::{LexError, Lexer, tokenize};
pub use parser::{ParseError, Parser, parse};
pub use pretty::{format_expr, format_program, format_type};
pub use token::{Token, TokenKind};
pub use types::{Ty, assignable, common_numeric};
