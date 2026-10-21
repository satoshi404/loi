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
TopLevel    := Attr* Declaration
Declaration := PackageDecl
              | TypeDecl
              | StructDecl
              | ExternDecl
              | VarDecl
              | FuncDecl
              | ProcDecl          // 'procedure > name(...) { ... }'
              | FuncDeclModern    // 'function > name(...) out T { ... }'

PackageDecl := '@package' '(' Ident ('.' Ident)* ')' (';')?
Attr        := '@' 'section' '(' String ')'
              | '@' 'attr' '(' AttrItem (',' AttrItem)* ')'
AttrItem    := Ident ('(' (String | Ident | Number) ')')?
```

Every loi file must declare a package. The entry file and every `@inc`-ed file
are checked: a file without an `@package(...)` declaration is rejected with

```
error: file "x.loi" must declare a package (e.g. '@package(name)')
```

Dotted names are allowed (`@package(core.utils)`). The name is shown in the AST;
runtime members still go through the `std` package (see [Runtime](#runtime)). The
old bare `package name` form is rejected with a pointer to `@package(name)`.

### Packages, namespaces, and visibility

Files that declare the **same** `@package(name)` form a single namespace: a
symbol defined in one of them is visible to the others without qualification.
Includes (`@inc`) splice textually, so the compiler re-asserts the enclosing
package after every included file; symbols never leak from one package into
another.

By default every top-level symbol is **`global`** — reachable from any package.
Mark a declaration `@attr(local)` to keep it private to its package:
references from a different package are then a type error.

```loi
@package(core)

@attr(local)
helper = (): int { return 7; }   // only visible inside `core`

@attr(global)
public_api = (): int { return helper(); }
```

```
Type error: 'helper' is local to package 'core' and is not visible here
```

The last visibility attribute wins when several are stacked
(`@attr(local, global)` == `global`).

#### Qualified names

A symbol of another package can be named with the package as a prefix, which
is how a binding such as `sdl2` is meant to be used:

```loi
@inc sdl2
w: sdl2.Window* = sdl2.CreateWindow("hi", sdl2.WINDOWPOS_CENTERED, 0, 640, 480, sdl2.WINDOW_SHOWN);
```

- `pkg.fn(...)` calls `fn` and is checked like any call (arity, argument types,
  effects, capabilities). If `fn` belongs to a different package the compiler
  reports `'fn' is not a member of package 'pkg'`.
- `pkg.NAME` selects a `::` constant (see below); `pkg.Type` names a type.
- `std.*` is handled by the runtime and is not checked this way.

Top-level names still share one flat namespace at link time, so two packages
cannot both define a function with the same name; generated bindings avoid
clashes by using the C symbol through `@attr(link_name(...))`.

There is no `import` keyword. Every inclusion — loi files, packages, and C
header checks — is a preprocessor directive introduced by `@` and handled
before parsing (see [`@inc` / `@cinc`](#inc--cinc-inlines-and-c-header-checks)):

```loi
@inc "path/to/file.loi"
@inc path/to/file.loi
@inc std
@inc std.math
@cinc <stdio.h>
@cinc "stdio.h"
```

## Types

```
Type        := BaseType TypeSuffix*
BaseType    := Primitive
             | Ident ('.' Ident)?          // user type, optionally package-qualified (`sdl2.Window`)
             | 'fn' '(' TypeList? ')' '->' Type ('!' EffectList)?
             | '(' Type ')'
             | '(' ParamName ':' Type (',' ParamName ':' Type)* ')' '->' Type ('!' EffectList)?
             | '@' 'typeof' '(' Expr ')'
             | '@cap' '(' 'borrow'? CapAtom ('+' CapAtom)* ')'   // lexical capability
             | 'type' Ident '=' Type         // alias (top-level)

Primitive   := 'int' | 'float' | 'double' | 'char' | 'bool'
             | 'byte' | 'short' | 'long' | 'void'
             | 'unsigned'? ( 'int' | 'char' | 'short' | 'long' | 'byte' )
             | 'i8' | 'i16' | 'i32' | 'i64'        // signed integers
             | 'u8' | 'u16' | 'u32' | 'u64'        // unsigned integers
             | 'f32' | 'f64'                       // floating point

TypeSuffix  := '*'                         // pointer
             | '[' Expr? ']'                // array / size (postfix, canonical)
```

Array types are written **postfix**: the length follows the element type, e.g.
`int[10] a;` or `int[] a;` (inferred length). The old prefix form `[10]int` is
not parsed.

**Multidimensional arrays** are written by stacking postfix suffixes:

```loi
i32[3][3] m;        // 3x3 matrix of i32
f64[4][2] table;    // 4x2 matrix of f64
```

Each index selects one level: `m[i][j]` accesses row `i`, column `j`. The type
of `m[i]` is `[3 x i32]` (an array), so `m[i][j]` is `i32`. Arrays can be
nested to any depth.

## Declarations

```
VarDecl     := Type Ident ('=' Expr)? ';'
             | Ident ':' Type ('=' Expr)? ';'
             | 'const' Type Ident '=' Expr ';'
             | 'static' Type Ident ('=' Expr)? ';'
             | 'let' Ident ':' Type ('=' Expr)? ';'

ConstDecl   := Ident '::' Literal ';'            // untyped compile-time constant

StructDecl  := 'struct' Ident '{' (Type Ident ';')+ '}'
             | 'union'  Ident '{' (Type Ident ';')+ '}'
             | 'enum'   Ident '{' Ident (',' Ident)* '}'

TypeDecl    := 'type' Ident '=' Type ';'

ExternDecl  := 'extern' Type Ident '(' ParamList? ')' ';'
             | 'extern' 'asm' String ';'
```

### Constants (`NAME :: literal;`)

```loi
FLAG_VIDEO :: 0x20;
BIG        :: 3221225472;     // past INT32_MAX
NEG        :: -5;
TITLE      :: "loi";
```

A `::` constant is a number, character or string literal (optionally negated)
with **no fixed type**: every use is replaced by the literal during type
checking, so it is free at run time and adapts to its context. `pkg.NAME`
selects the constant of a package. Generated bindings use them for C `#define`s
and `enum` members.

### Literal adaptation

A numeric literal (or a `::` constant, or arithmetic on them) converts to the
numeric type the context asks for: in a declaration (`unsigned int f = 32;`),
an assignment, a call argument, or against the other operand of a binary
operator (`flags | FLAG_VIDEO`). Only literals adapt. Two *typed* operands of
different types — `int` and `unsigned int`, say — are still a type error, and
a float literal never narrows to an integer.

### Unions

`union` members all start at offset 0. The union is as large and as aligned as
its biggest member, `@offsetof(U, member)` is `0`, and writing one member then
reading another reinterprets the same bytes:

```loi
union Event { unsigned int kind; KeyEv key; MouseEv mouse; char[56] pad; }
ev.key.sym = 27;
```

### Field access

`a.b.c`, `p.b` / `p->b` through a pointer to a struct (to any depth), `a.arr[i]`
for an array field, and `&a.b` all work as expressions and as assignment
targets (including `+=` and friends). The type of a field is its declared type.

## `when` — conditional dispatch (replaces `if`/`else` chains and `switch`)

`when` dispatches on a value using comparison patterns. It works both as a
statement and as an expression (its value is the last expression of the
selected branch).

```loi
when v {
  == 1  -> { std.print_str("X"); }
  == -1 -> { std.print_str("O"); }
  otherwise -> { std.print_str("-"); }
}

when value {
  < 0  -> { ... }
  > 10 -> { ... }
  otherwise -> { ... }
}
```

Each branch is `<comparison> <expr> -> { block }` or `otherwise -> { block }`.
Comparisons supported: `==`, `!=`, `<`, `<=`, `>`, `>=`. The `otherwise`
branch is optional and matches when no other branch did.

When used as an expression, all branches that produce a value must agree on
the result type.

```loi
sign = (v: i32) out i32 {
  when v {
    > 0 -> { 1 }
    < 0 -> { -1 }
    otherwise -> { 0 }
  }
}
```

`if`/`else` and `switch` continue to work for backward compatibility.

## Functions

Functions are declared by assignment, matching the example:

```
FuncDecl    := Attribute* Ident FuncSignature? '=' Block
Attribute   := '@' 'section' '(' String ')'   // place fn in an assembly section
             | '@' 'root'                     // entry point (may @grant)
             | '@' 'attr' '(' AttrItem (',' AttrItem)* ')'
FuncSignature := '(' ParamList? ')' (':' Type | '->' Type)?
                 ('!' EffectList)? ('@' 'needs' '(' CapSet ')')? ('@' 'gives' '(' CapSet ')')?

ParamList   := Param (',' Param)*
Param       := Type Ident
             | Ident ':' Type
             | Type

CapSet      := CapAtom ('+' CapAtom)*
CapAtom     := Ident

Block       := '{' Stmt* '}'
```

### Attributes (`@attr`)

`@attr(...)` attaches metadata to the **next** declaration (function, `extern`,
global, struct, or field). Modifiers stack, and several lists may precede one
declaration. Items are bare flags (`inline`) or named with a single argument
(`link_name("...")`).

```loi
@attr(local, inline)
fast = (n: int): int { return n + 1; }

@attr(global, no_return, link_name("loi_die"))
die = (): void { while (1) { } }

@attr(extern, link_name("malloc"), section("text"))
extern ptr malloc(size: int);
```

Recognised attributes:

| attribute            | effect                                                        |
|----------------------|---------------------------------------------------------------|
| `local` / `global`   | visibility within the package (default `global`)              |
| `link_name("sym")`   | emit the definition/call under the given symbol name          |
| `section("name")`    | place the function in an assembly section (same as `@section`)|
| `inline`             | emit LLVM `alwaysinline`                                      |
| `no_return`          | emit LLVM `noreturn`                                          |
| `pure`               | emit LLVM `readnone` (no side effects / memory reads)         |
| `must_use`           | warn (advisory) when a call's result is discarded             |
| `extern`             | marks the declaration as externally defined                   |

The system is **extensible**: unknown attribute names are recorded in the AST
(the default dump prints `attrs=...`) and otherwise ignored, so new attributes
can be added without breaking existing code.

`@attr(must_use)` produces an advisory diagnostic (it does **not** fail the
build) when the result of a call is thrown away:

```loi
@attr(must_use, pure)
double_it = (n: int): int { return n * 2; }

main = (): int {
    double_it(21);            // Warning: unused result of 'double_it'
    int x = double_it(21);    // ok — result used
    return x - 42;
}
```

A `void` function never warns, even with `must_use`.

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

The canonical function form is the **assignment form** above. The equivalent
signature-first form `name (params): Ret = { ... }` is also accepted, and `:`
may be written `->` before the return type in a declaration. Prefer the
assignment form for consistency.

### Modern function forms (`procedure` / `function`)

A modern, more visual syntax is available for declaring procedures (no return)
and functions (with return). The `>` after the keyword is part of the syntax:

```loi
procedure > draw_board(board: i32[9]) {
  range 0..9 -> i {
    std.print_int(board[i]);
  }
}

function > add(a: i32, b: i32) out i32 {
  a + b
}

function > add(a: i32, b: i32) out i32 -> a + b
```

`procedure` declares a `void` function. `function` declares a function with
the return type given after `out`. The compact `-> expr` form is sugar for a
block containing a single `return expr`.

Both forms work alongside the classic assignment form (`name = (params) -> Ret { ... }`).

**The `main` entry point** can use either the classic form or the modern forms:

```loi
// Classic form
main = () -> int { return 0; }

// Modern function form
function > main() out i32 { return 0; }

// Modern procedure form (void)
procedure > main() { }
```

### Typed function pointers

Function types are first-class and carry effects:

```loi
type CmpFn = (a: int, b: int) -> int;
type Handler = (n: int) -> void !IO;

apply = (f: (a: int, b: int) -> int, x: int, y: int): int { return f(x, y); }

cmp: CmpFn = null;         // typed function-pointer variable, null-initialised
```

They may be assigned function names (or other pointers), passed as callbacks and
called indirectly. Effects on the pointer type are enforced on assignment and at
the indirect call site (see *Effects* below).

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
             | GrantStmt
             | RefineStmt
             | DropStmt
             | WithCapStmt

IfStmt      := 'if' '(' Expr ')' Block ('else' Block)?
WhileStmt   := 'while' '(' Expr ')' Block
ForStmt     := 'for' '(' (VarDecl | Expr)? ';' Expr? ';' Expr? ')' Block
RangeStmt   := 'range' Expr '..' Expr '->' Ident Block
ForeachStmt := 'foreach' Expr '->' Ident Block
ReturnStmt  := 'return' Expr? ';'
BreakStmt   := 'break' ';'
ContinueStmt:= 'continue' ';'
AsmStmt     := 'asm' '(' String ')' ';'
ExprStmt    := Expr ';'
GrantStmt   := '@' 'grant' '(' CapSet ')' Ident ';'   // birth a capability (roots only)
RefineStmt  := '@' 'refine' '(' CapAtom '->' CapAtom ')' Ident ';'
DropStmt    := '@' 'drop' '(' Ident ')' ';'
WithCapStmt := '@' 'with_cap' '(' CapSet ')' Block   // ambient borrow of live caps
```

### `range` and `foreach`

`range` iterates over an integer range (end-exclusive), binding the loop
variable to each value:

```loi
range 0..9 -> i {
  std.print_int(i);
}
```

`foreach` iterates over an array variable, binding the loop variable to each
element (the loop variable holds the index; index the array manually):

```loi
i32[3] nums;
foreach nums -> n {
  std.print_int(nums[n]);
}
```

Both support `break` and `continue`. The classic C-style `for` remains
available but `range`/`foreach` are the preferred forms.

Semicolons are optional at the end of a block (before `}`). The last
expression of a function block can serve as an implicit return when the
function has a non-void return type.

### `std` I/O functions

The `std` package provides several functions for output:

| Function | Description |
|----------|-------------|
| `std.print_str(s)` | Print a string followed by a newline |
| `std.print_str_raw(s)` | Print a string **without** a trailing newline |
| `std.print_bytes(arr, n)` | Print `n` bytes from array `arr` (no newline) |
| `std.print_bytes_nl(arr, n)` | Print `n` bytes from array `arr` followed by a newline |

```loi
u8[5] data;
data[0] = 72; data[1] = 101; data[2] = 108;
data[3] = 108; data[4] = 111;  // "hello"

std.print_str_raw("raw: ");
std.print_bytes(data, 5);
std.print_str("\n");

std.print_bytes_nl(data, 5);  // prints "hello" with newline
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

- Memory helpers from `std`: `std.alloc`, `std.free`, `std.memcpy`,
  `std.memset`, plus `std.gc_alloc` / `std.gc_collect`.

## Mathematical features

Standard operators `+ - * / % **`. The runtime provides math builtins,
available via `@inc std`:

- integer: `std.abs(v)`, `std.min(a, b)`, `std.max(a, b)`
- double: `std.sqrt`, `std.sin`, `std.cos`, `std.tan`, `std.exp`, `std.log`,
  `std.floor`, `std.ceil`, and `std.pow(a, b)`

Vector/matrix math is expressed with arrays and library types, e.g.:

```loi
@inc std

hypot = (a: double, b: double): double
{
    double s = a * a + b * b;
    return std.sqrt(s);
}
```

## Memory regions (lexical scope)

```loi
RegionStmt := 'region' Ident ('(' Expr ')')? Block
```

`region` introduces a lexical allocator scope. The name is bound to a region handle
inside the block:

```loi
@inc std

main =
{
    region r(64) {
        void* p = std.region_alloc(r, 8);
        *p = 7;
        std.print_int(*p);
    }  // std.region_free(r) is emitted here
    return 0;
}
```

If no capacity is given, a default (currently 4096 bytes) is used:

```loi
main =
{
    region r {
        void* p = std.region_alloc(r, 4);
    }
    return 0;
}
```

## Runtime

The runtime is exposed through the standard package. Load it with `@inc std`
and call its members with the `std.` qualifier — the internal `loi_` prefix is
never written in loi code:

```loi
@package(demo)

@inc std

main = () -> int !IO
{
    std.print_int(42);
    std.print_str("hello");
    void* p = std.alloc(16);
    std.free(p);
    return 0;
}
```

`std` is a virtual package: `std.NAME` is rewritten by the code generator to the
matching runtime symbol (for example `std.print_int` → `loi_print_int`,
`std.region_alloc` → `loi_region_alloc`). The helpers are always linked into
executables, so `@inc std` mainly documents intent and keeps the source honest
about where the symbols come from.

## Effects annotations (experimental)

Function signatures may carry declared effects after the return type using `!`:

```loi
worker = (arg: void*): void* !IO,Alloc
{
    std.print_str("side effect");
    void* p = std.region_alloc(arg, 8);
    return p;
}
```

Grammar:

```loi
FuncSignature ::= '(' ParamList? ')' (':' Type | '->' Type)? ('!' Ident (',' Ident)*)?
```

The compiler records `effects` on every function and function-pointer *type* and
**enforces** them wherever function values meet:

- a **direct call** to a function with effects requires the caller to declare a
  superset of those effects;
- an **indirect call** through a function-pointer value whose type carries
  effects is checked the same way;
- **assignment** to a function-pointer target is rejected when the value's
  effects are not covered by the target type (you cannot silently hide `!IO`
  behind a pure function pointer);
- **passing a callback** as an argument is checked against the parameter's
  function type, so an `!IO` function cannot be passed where a pure callback is
  expected;
- **returning** a function value is checked against the declared return type.

The declared type is the contract: a caller may only do what its own effects
permit.

Effect annotations are excluded from structural type equality, so two function
types that differ only in effects still unify for overload/assignment purposes;
the separate effect check is what makes them safe.

## Lexical capabilities (zero-cost permissions)

`@cap(...)`, `@grant`, `@refine`, `@drop`, `@with_cap`, `@needs`, `@gives` and
`region ... with` form loi's **lexical capabilities** and **dual effects**. Full
guide: [`capabilities.md`](capabilities.md) and [`needs.md`](needs.md).

```
@capability(Read, Write)            // declare the kinds (optional)
@cap(Write) w;                      // owned proof (linear)
@cap(borrow Read) r;                // borrowed proof (non-consuming)
@grant(Write) w;                    // birth at a root (main / @root / producer)
@refine(Write -> Read) w;           // weaken in place
@drop(w);                           // consume
@with_cap(Write) { ... }            // ambient borrow of a live proof, block-scoped
write = (buf: ptr, c: @cap(Write)): void { *buf = 1; }
write(&buf);                        // elided: inferred from the one live Write
load  = (): int @needs(Filesystem) { ... }
region r(64) with Write as w { ... }  // ambient cap released with the region
```

Capabilities are **linear** (an owned proof must be consumed, returned, or
`@drop`'ed), **refinable**, and **zero-cost** (erased at runtime — they lower to
an `i8` placeholder the optimizer removes).

## Pointers and dereference assignment

In addition to pointer types `T *p`, address-of `&x`, and reads `*p`, loi also
supports assignment through a dereference:

```loi
@inc std

main =
{
    void* p = std.alloc(8);
    *p = 42;
    int v = *p;
    std.print_int(v);
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
- `@(inc "relative/or/absolute.loi")` — inlines a loi file (or package), recursive up to a
  depth limit. Relative to the file's directory, `./lib`, `./packages`, the standard
  `/usr/share/loi` paths, and any entry in `LOI_PATH`.
- `@(inc name)` — package form, resolved through the search paths above (e.g.
  `@(inc std)` looks for `std.loi`).
- `@(cinc <system/header.h>)` — verifies a C header exists (searching `.` then
  `/usr/include`, `/usr/local/include`,
  `/usr/include/x86_64-linux-gnu`); a trace marker is emitted instead of the
  header, since C ABI declarations belong to `extern`.
- `@(undef NAME)`, `@(error message)` — remove a macro, abort compilation

Each of these also has a *line form* that reads to end-of-line (useful when a
macro body contains unbalanced parentheses):

- `@define NAME replacement`
- `@inc "file"`
- `@cinc <header.h>`
- `@undef NAME`
- `@error message`

The accepted inclusion forms are exactly:

```loi
@inc "path/to/file.loi"
@inc path/to/file.loi
@inc std
@inc std.math
@cinc <stdio.h>
@cinc "stdio.h"
```

A bare (unquoted) package path has its dots mapped to directory separators
(`@inc std.math` → `std/math`), while a path containing `/` or ending in `.loi`
is used verbatim. There is no `import` keyword; a leftover `import` is a
parse-time error.

Example:

```loi
@package(demo)

@(define LIMIT 8)
@inc "helpers/helper.loi"

main =
{
    return double_it(LIMIT);
}
```

Includes are guarded automatically: a given file (by real path) is inlined at
most once per compilation, so libraries need no manual `#ifndef` guards and
include cycles terminate. All inclusions use `@inc` (loi files/packages) or
`@cinc` (C header checks). C standard-header ABIs are still modelled
via `extern` declarations rather than textual inclusion.

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

Arguments are expanded at the point of substitution (after the macro body's other identifiers and meta-operators have been prepared), so macro arguments are typically expanded before their use.  Parentheses are not added automatically; add them in the definition.

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

### Stringizing, pasting, and variadic macros (`@stringify` / `@paste` / `...`)

`@(`define`) supports three meta-operators that turn a macro *into* a code
generator:

- `@stringify(x)` – expand to the string literal containing the raw token
  text of the argument.  Inside `@stringify` the argument is not macro-expanded,
  matching C's `#` (e.g. `@(define STR(x) @stringify(x))`).
- `@paste(a, b)` – concatenate the token text of `a` and `b` into a single
  identifier or token (like C's `##`).  Nested `@stringify`/`@paste` are
  reduced first, and the pasted result is rescanned for further macro expansion.
- `...name` (or just `...`) as the last parameter of a function-like macro
  binds all remaining arguments to `name` joined by commas, giving a variadic
  macro.  `@if` is honoured in macro bodies (it is resolved in a second
  conditional pass after expansion, so the branch chosen depends on the
  substituted value).

```loi
@(define STR(x) @stringify(x))
@(define CONCAT(a, b) @paste(a, b))
@(define LOG(level, ...msg) { @if(level >= 1) { std.print_str(msg); } })

int foobar = 77;

main = () -> int !IO
{
    std.print_str(STR(hello));        // "hello"
    std.print_str(STR(LIMIT));         // "LIMIT"
    int foo = 1, bar = 2;
    std.print_int(CONCAT(foo, bar));  // foobar -> 77
    LOG(1, "hi\n");                  // prints "hi\n"
    return 0;
}
```

### Limitations

- Object/function-like macros only.
- `@(define)` reads to the matching parenthesis, so bodies with unbalanced
  parentheses need the line form `@define`.
- Conditions evaluate integers and macro bodies; function-like macro names alone
  evaluate to `0`.
- Though `@stringify`/`@paste`/`...` are now supported, they are still C-style-
  adventurous features; nesting them deeply or combining with late-expansion
  effects is not always well-defined.
- `@(define)` / `@(undef)` are processed left-to-right while conditionals are
  evaluated, so macros declared inside a skipped branch are never registered
  (and an `@(undef)` in a skipped branch does not apply).

### `@inc` / `@cinc` (inlines and C-header checks)

```loi
@inc "helpers/lib.loi"
@inc std
@cinc <stdio.h>
@cinc "stdint.h"
```

- `@inc "..."` includes are resolved relative to the including file first, then
  `./lib`, `./packages`, the standard `/usr/share/loi` paths, and any entry in
  `LOI_PATH`.
- `@inc name` (no path) looks for `name.loi` in those search paths, enabling
  lightweight package use.
- `thirdparty/` holds bindings to external C libraries. After `./lib`, a bare
  name is also tried as `./thirdparty/name/name.loi` and
  `./thirdparty/name.loi` (so `@inc sdl2` finds `thirdparty/sdl2/sdl2.loi`),
  then under `/usr/share/loi/thirdparty`, `/usr/local/share/loi/thirdparty` and
  `$LOI_PATH/thirdparty`. `lib/` is the language runtime and always wins.
  See [`bindings.md`](bindings.md).
- `@cinc <...>` searches `.`, `/usr/include`, `/usr/local/include`, and
  `/usr/include/x86_64-linux-gnu`; since C preprocessor ABI is not modeled, a
  trace marker (`/* @cinc: … */`) is emitted instead of inlining, and the
  corresponding ABI functions are declared with `extern`.
- All inclusions go through `@inc` (loi files/packages) or `@cinc` (C headers).
- Includes are guarded automatically: a given file (by real path) is inlined at
  most once per compilation, so repeating `@inc` on the same file is safe and
  include cycles terminate.

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
@inc std

main =
{
    int a = 3;
    int b = 4;
    return a * a + b * b;
}
```
