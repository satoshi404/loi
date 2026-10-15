#ifndef LOI_CODEGEN_H
#define LOI_CODEGEN_H

#include <stdio.h>
#include "ast.h"
#include "parser.h"

/* Emit LLVM IR for a parsed program to `out`. */
void codegen_program(ASTNode *prog, FILE *out);

#endif
