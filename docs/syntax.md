# loi — Syntax Specification

loi is a hybrid, statically typed language that combines low-level control
(pointers, structs, inline assembly) with mathematical expressions and builtins.

## Lexical

- Identifiers: `[A-Za-z_][A-Za-z0-9_]*`
- Keywords are reserved (see lexer).
- Comments: `// ...` and `/* ... */`.
- Numbers: decimal, hex (`0x`), binary (`0b`), floats with optional exponent.
- Strings: `"..."` with escapes (`\n \t \r \\ \" \0 \xHH`).
- Characters: `'...'`.

## Program structure

```
Program     := TopLevel*
TopLevel    := ImportDecl
             | TypeDecl
             | StructDecl
             | ExternDecl
             | VarDecl
             | FuncDecl

ImportDecl  := 'import' Ident ('.' Ident)* ('as' Ident)? (';')?
```

## Types

```
Type        := BaseType TypeSuffix*
BaseType    := Primitive
             | Ident                       // user type
             | 'fn' '(' TypeList? ')' '->' Type
             | '(' Type ')'
             | '@' 'typeof' '(' Expr ')'   // type of an expression

Primitive   := 'int' | 'float' | 'double' | 'char' | 'bool'
             | 'byte' | 'short' | 'long' | 'void'
             | 'unsigned'? ( 'int' | 'char' | 'short' | 'long' | 'byte' )

TypeSuffix  := '*'                         // pointer
             | '[' Expr? ']'                // array / size
```

## Declarations

```
VarDecl     := Type Ident ('=' Expr)? ';'
             | Ident ':' Type ('=' Expr)? ';'
             | 'const' Type Ident '=' Expr ';'
             | 'static' Type Ident ('=' Expr)? ';'
             | 'let' Ident ':' Type ('=' Expr)? ';'

StructDecl  := 'struct' Ident '{' (Type Ident ';')+ '}'
             | 'union'  Ident '{' (Type Ident ';')+ '}'
             | 'enum'   Ident '{' Ident (',' Ident)* '}'

TypeDecl    := 'type' Ident '=' Type ';'

ExternDecl  := 'extern' Type Ident '(' ParamList? ')' ';'
             | 'extern' 'asm' String ';'
```

## Functions

Functions are declared by assignment, matching the example:

```
FuncDecl    := Attribute? Ident FuncSignature? '=' Block
Attribute   := '@' 'section' '(' String ')'   // place fn in an assembly section
FuncSignature := '(' ParamList? ')' (':' Type | '->' Type)?

ParamList   := Param (',' Param)*
Param       := Type Ident
             | Ident ':' Type
             | Type

Block       := '{' Stmt* '}'
```

Examples:

```loi
main =
{
    return 0;
}

add = (a: int, b: int): int
{
    return a + b;
}

factorial = (n: int) -> int
{
    if (n <= 1) { return 1; }
    return n * factorial(n - 1);
}
```

## Statements

```
Stmt        := VarDecl
             | IfStmt
             | WhileStmt
             | ForStmt
             | ReturnStmt
             | BreakStmt
             | ContinueStmt
             | AsmStmt
             | ExprStmt
             | Block

IfStmt      := 'if' '(' Expr ')' Block ('else' Block)?
WhileStmt   := 'while' '(' Expr ')' Block
ForStmt     := 'for' '(' (VarDecl | Expr)? ';' Expr? ';' Expr? ')' Block
ReturnStmt  := 'return' Expr? ';'
BreakStmt   := 'break' ';'
ContinueStmt:= 'continue' ';'
AsmStmt     := 'asm' '(' String ')' ';'
ExprStmt    := Expr ';'
```

## Expressions

Precedence (lowest to highest):

```
Expr        := Assign
Assign      := LValue AssignOp Expr | Ternary
AssignOp    := '=' | '+=' | '-=' | '*=' | '/=' | '%='
             | '&=' | '|=' | '^=' | '<<=' | '>>='

Ternary     := LogOr ('?' Expr ':' Expr)?
LogOr       := LogAnd ('||' LogAnd)*
LogAnd      := BitOr ('&&' BitOr)*
BitOr       := BitXor ('|' BitXor)*
BitXor      := BitAnd ('^' BitAnd)*
BitAnd      := Eq ('&' Eq)*
Eq          := Rel ('==' | '!=' Rel)*
Rel         := Shift (('<' | '>' | '<=' | '>=' ) Shift)*
Shift       := Add ('<<' | '>>' Add)*
Add         := Mul (('+' | '-') Mul)*
Mul         := Pow (('*' | '/' | '%') Pow)*
Pow         := Unary ('**' Pow)?            // right assoc
Unary       := ('!' | '~' | '+' | '-' | '*' | '&') Unary
             | Postfix
Postfix     := Primary ( '++' | '--'
             | '[' Expr ']'                // index
             | '(' ArgList? ')'            // call
             | '.' Ident                   // field
             )*
Primary     := Number | String | Char | Ident
             | '(' Expr ')'
             | 'sizeof' '(' Type ')'
             | 'alignof' '(' Type ')'
             | '@' 'sizeof'   '(' Type ')'
             | '@' 'alignof'  '(' Type ')'
             | '@' 'offsetof' '(' Type ',' Ident ')'
             | '@' 'typeof'   '(' Expr ')'     // yields the expression itself
```

LValues: identifiers, `*expr`, `expr[expr]`, `expr.ident`.

## Low-level features

- Pointers: `int* p`, `&x`, `*p`, `p[i]`.
- Arrays: the length follows the element type — `int[10] a;` — then `a[i]`.
- Struct access: `p.x`, `p->x` (lowered to `(*p).x`).
- Inline assembly (a statement, so it takes a semicolon):

```loi
main =
{
    asm("nop");
    return 0;
}
```

- Memory helpers from `std`: `loi_alloc`, `loi_free`, `loi_memcpy`,
  `loi_memset`, plus `loi_gc_alloc` / `loi_gc_collect`.

## Mathematical features

Standard operators `+ - * / % **`. The runtime provides math builtins,
available via `import std`:

- integer: `loi_abs(v)`, `loi_min(a, b)`, `loi_max(a, b)`
- double: `loi_sqrt`, `loi_sin`, `loi_cos`, `loi_tan`, `loi_exp`, `loi_log`,
  `loi_floor`, `loi_ceil`, and `loi_pow(a, b)`

Vector/matrix math is expressed with arrays and library types, e.g.:

```loi
import std

hypot = (a: double, b: double): double
{
    double s = a * a + b * b;
    return loi_sqrt(s);
}
```

## Memory regions (lexical scope)

```loi
RegionStmt := 'region' Ident ('(' Expr ')')? Block
```

`region` introduces a lexical allocator scope. The name is bound to a region handle
inside the block:

```loi
import std

main =
{
    region r(64) {
        void* p = loi_region_alloc(r, 8);
        *p = 7;
        loi_print_int(*p);
    }  // loi_region_free(r) is emitted here
    return 0;
}
```

If no capacity is given, a default (currently 4096 bytes) is used:

```loi
main =
{
    region r {
        void* p = loi_region_alloc(r, 4);
    }
    return 0;
}
```

Notes:
- Region handles are normal pointers; the language does not yet enforce no-escape.
- `loi_region_alloc` is available from `std`.

## Effects annotations (experimental)

Function signatures may carry declared effects after the return type using `!`:

```loi
worker = (arg: void*): void* !IO,Alloc
{
    loi_print_str("side effect");
    void* p = loi_region_alloc(arg, 8);
    return p;
}
```

Grammar:

```loi
FuncSignature ::= '(' ParamList? ')' (':' Type | '->' Type)? ('!' Ident (',' Ident)*)?
```

The compiler records `effects` in the AST and prints them with `--print-tree` or
default output, but does not yet perform call-site effect checking. This is
intentional for incremental experimentation.

## Pointers and dereference assignment

In addition to pointer types `T *p`, address-of `&x`, and reads `*p`, loi also
supports assignment through a dereference:

```loi
import std

main =
{
    void* p = loi_alloc(8);
    *p = 42;
    int v = *p;
    loi_print_int(v);
    return v;
}
```

LValues supported on the left of `=`:

- `IDENT`
- `*expr`
- `expr[expr]`
- `expr.ident`

Pointer targets are lowered conservatively as `i32` or `ptr` depending on context;
true byte/struct pointee types are a later extension.

## Preprocessor (basic)

Before lexing, `loi` performs a lightweight preprocessing pass. All directives
use the `@` sigil so the metaprogramming layer is visually unified (with
`@if`/`@align`/`@pack`):

- `@(define NAME replacement)` — object-like and function-like macros
- `@(include "relative/or/absolute.h")` — inlines the file, recursive up to a
  depth limit
- `@(include <system/path>)` — searched in `.` then `/usr/include` and
  `/usr/include/x86_64-linux-gnu`
- `@(undef NAME)`, `@(error message)` — remove a macro, abort compilation

Each of these also has a *line form* that reads to end-of-line (useful when a
macro body contains unbalanced parentheses):

- `@define NAME replacement`
- `@include "file"`
- `@undef NAME`
- `@error message`

Example:

```loi
@(define LIMIT 8)
@(include "helpers/helper.loi")

main =
{
    return double_it(LIMIT);
}
```

Limitations: no include guards, no C standard-header ABI modeling.

## Metaprogramming / Preprocessor

loi provides a lightweight, pre-lexer preprocessing stage for metaprogramming.

### Object-like macros

```loi
@(define LIMIT 8)
```

Line form (rest of line becomes the body):

```loi
@define LIMIT 8
```

### Function-like macros

```loi
@(define ADD(x, y) ((x) + (y)))
@(define SQUARE(x) ((x) * (x)))
```

Arguments are fully expanded before substitution; argument repetition is allowed.
The compiler does not add parentheses for you, use them explicitly.

The `@(define ...)` form reads to the *matching* parenthesis, so a body may
contain balanced parentheses (as in the examples above). If a body would contain
unbalanced parentheses, use the line form `@define`, which reads to end-of-line.

### Conditional compilation (`@if` blocks)

Conditionals use a macro-like block syntax with `@`:

```loi
@(define DEBUG 1)
@(define TRACE 0)

@if(DEBUG) {
    int x = 1;
} @elif(TRACE) {
    int x = 2;
} @else {
    int x = 0;
}
```

Supported inside the condition expression:

- integer literals and arithmetic: `+ - * / %`;
- comparisons: `== != < <= > >=`;
- logic: `! && ||` and parentheses;
- `defined(NAME)` — true if the macro exists;
- identifiers expand to their macro body and are evaluated (truthy when non-zero);
- `0` is false, anything else is true.

`@elif(...)` and `@else` are optional; `@endif` is optional when braces close the
chain. Blocks can be nested:

```loi
@if(defined(DEBUG) && !defined(RELEASE)) {
    @if(DEBUG >= 1) {
        int verbose = 1;
    }
}
```

`@(define)`, `@(undef)`, `@(error)` are also supported (as are the `@define`,
`@undef`, `@error` line forms):

```loi
@(define FLAG)
@(undef FLAG)
@(error something went wrong)
```

Notes:

- `@if` blocks remove their own surrounding braces, so they do not introduce a
  lexical scope — declarations inside a taken branch stay in the enclosing scope.
- Macros are scoped to the branch that declares them: a `@(define)` / `@(undef)`
  inside a skipped branch are ignored, which makes feature flags and conditional
  configuration safe:

```loi
@if(ENABLE_GPU) {
    @(define BACKEND gpu)
} @else {
    @(define BACKEND cpu)
}
```

### Include

```loi
@(include "helpers/lib.loi")
@(include <stdio.h>)
```

- `"..."` includes are resolved relative to the including file first, then the current
  working directory.
- `<...>` is searched in `.`, `/usr/include`, and
  `/usr/include/x86_64-linux-gnu`.

### Limitations

- Object/function-like macros only.
- No stringizing (`#x`), token pasting (`##`), or variadic macro args yet.
- Conditions evaluate integers and macro bodies; function-like macro names alone
  evaluate to `0`.
- `@(define)` / `@(undef)` are processed left-to-right while conditionals are
  evaluated, so macros declared inside a skipped branch are never registered
  (and an `@(undef)` in a skipped branch does not apply).
- No C preprocessor ABI guarantees for `@(include <...>)` headers.

## Compile-time code generation

Metaprogramming directives can emit code during the preprocessor pass, so a
block is *unrolled* into real statements before parsing. There is no runtime
loop overhead.

### `@repeat(count) { ... }`

Copies its block `count` times. `count` is any integer expression the
conditional evaluator understands (literals, arithmetic, macros):

```loi
main =
{
    int total = 1;
@repeat(3) {
    total = total * 2;      /* total = 8 */
}
    return total;
}
```

### `@for(var, start, stop [, step]) { ... }`

Unrolls the block once per iteration. The loop variable is substituted with its
integer value as a whole word:

```loi
main =
{
    int total = 0;
@for(i, 0, 5) {
    total = total + i;          /* 0+1+2+3+4 = 10 */
}
@for(k, 0, 10, 2) {
    total = total + k;          /* +0+2+4+6+8 = 20 */
}
    return total;
}
```

- `stop` is exclusive.
- `step` defaults to `1`; negative steps iterate downward.
- The variable is textually substituted only where it appears as a complete
  identifier (so it will not clobber names like `sum` or `total`).

Loops nest and may contain `@if` (the variable is substituted first, so
`@if(i == 2)` picks the right branch per iteration).

See `examples/19_compiletime_codegen.loi`.

## Reflection builtins

Compile-time introspection over types; all fold to integer constants before
codegen. Struct layout respects field alignment and `@pack`, so the values match
what the backend actually lays out.

- `@sizeof(T)` — byte size of `T`
- `@alignof(T)` — required alignment of `T`
- `@offsetof(T, field)` — byte offset of `field` within struct `T`
- `@typeof(expr)` — the type of `expr`, usable anywhere a type is expected

```loi
struct Packed @pack() { char c; int i; }
struct Point  { int x; int y; double z; }

main =
{
    int sz   = @sizeof(Point);       /* 16 */
    int offz = @offsetof(Point, z);  /*  8 */
    int algn = @alignof(Point);      /*  8 */
    int psz  = @sizeof(Packed);      /*  5 (packed) */

    @typeof(sz) copy = sz;           /* copy is an int */
    return sz + offz + algn + psz + copy;
}
```

The plain keywords `sizeof(T)` / `alignof(T)` also remain valid (without `@`).

See `examples/20_reflection.loi`.

## `@section` placement

`@section("name")` places the next function into a named assembly section. It
survives to codegen, appears as `section "name"` in the emitted LLVM IR, and as
`.section <name>` in the final assembly.

```loi
@section("my_text")
fast_path = (n: int): int
{
    return n * 2;
}
```

Useful for hot/cold code splitting, cache-line grouping, or placing a function
into a ROM/text region. See `examples/21_sections.loi`.

## Example

```loi
import std

main =
{
    int a = 3;
    int b = 4;
    return a * a + b * b;
}
```
