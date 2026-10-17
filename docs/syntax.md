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
TopLevel    := PackageDecl
              | TypeDecl
              | StructDecl
              | ExternDecl
              | VarDecl
              | FuncDecl

PackageDecl := 'package' Ident (';')?
```

Every loi file must declare a package. The entry file and every `@inc`-ed file
are checked: a file without a `package` declaration is rejected with

```
error: file "x.loi" must declare a package (e.g. 'package name')
```

The package name is metadata: it names the unit and is shown in the AST, but it
does not change name resolution (members are reached through the `std` package,
see [Runtime](#runtime)).

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
             | Ident                       // user type
             | 'fn' '(' TypeList? ')' '->' Type ('!' EffectList)?
             | '(' Type ')'
             | '(' ParamName ':' Type (',' ParamName ':' Type)* ')' '->' Type ('!' EffectList)?
             | '@' 'typeof' '(' Expr ')'
             | 'type' Ident '=' Type         // alias (top-level)

Primitive   := 'int' | 'float' | 'double' | 'char' | 'bool'
             | 'byte' | 'short' | 'long' | 'void'
             | 'unsigned'? ( 'int' | 'char' | 'short' | 'long' | 'byte' )

TypeSuffix  := '*'                         // pointer
             | '[' Expr? ']'                // array / size (postfix, canonical)
```

Array types are written **postfix**: the length follows the element type, e.g.
`int[10] a;` or `int[] a;` (inferred length). The old prefix form `[10]int` is
not parsed.

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

The canonical function form is the **assignment form** above. The equivalent
signature-first form `name (params): Ret = { ... }` is also accepted, and `:`
may be written `->` before the return type in a declaration. Prefer the
assignment form for consistency.

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
package demo

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
package demo

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
@(inc "helpers/lib.loi")
@(inc std)
@(cinc <stdio.h>)
```

- `@inc "..."` includes are resolved relative to the including file first, then
  `./lib`, `./packages`, the standard `/usr/share/loi` paths, and any entry in
  `LOI_PATH`.
- `@inc name` (no path) looks for `name.loi` in those search paths, enabling
  lightweight package use.
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
