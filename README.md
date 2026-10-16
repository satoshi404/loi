# loi

A compiler toolchain for the **loi** programming language — a small systems
language that mixes low-level constructs (pointers, structs, inline asm,
threads) with mathematical syntax and a metaprogramming layer built around
the `@` sigil.

The compiler is written in C and emits LLVM IR, which is optimized with
`opt -O3` and lowered to a native executable with `clang -O3` (AArch64 or
whatever host clang targets).

## Philosophy

loi is best described as:

> **C with lexical memory regions + tracked effects + lightweight
> metaprogramming + explicit layout control.**

Rather than piling on new runtime machinery, loi keeps a C-sized core and adds
a few orthogonal, compile-time-checked ideas:

- **Lexical regions** for arena allocation with a clear lifetime, instead of
  manual `malloc`/`free` everywhere.
- **Effect annotations** (`!IO`, `!Alloc`) that are *enforced* at call sites and
  on function-pointer assignments, so a pure function cannot silently perform
  I/O.
- **Lightweight metaprogramming** under a single `@` sigil: macros, conditional
  compilation, unrolling and compile-time reflection — all erased before
  codegen, so there is zero runtime cost.
- **Explicit layout control** (`@pack`, `@align`, `@section`) for systems work
  where ABI and placement matter.

Everything else stays close to C so the generated LLVM IR and the mental model
remain predictable.

## Build

Requirements: `clang`, `opt` (LLVM), a C compiler, pthreads.

```sh
make          # builds ./loi
make test     # builds and runs the test suite
make clean
```

## Usage

```sh
./loi file.loi                  # parse and print the AST (type-checked)
./loi < file.loi                # read source from stdin
./loi file.loi --tokens         # dump the token stream
./loi file.loi --emit=ir        # print optimized LLVM IR to stdout
./loi file.loi --emit=ir --out=out.ll   # write optimized IR to a file
./loi file.loi --emit=asm       # print assembly to stdout
./loi file.loi --emit=asm --out=out.s   # write assembly to a file
./loi file.loi --emit=exe --out=prog    # build a native executable
./loi file.loi --run            # build and run; propagates the exit code
```

### Exit codes

Compilation errors always fail the build with exit status `1`:

- parse errors (`Parse error ...` + `parse had errors`)
- type errors (`Type error ...` + `type checking failed`)
- preprocessor errors (`@(error ...)` / `@error ...`, unresolved `@inc`)

With `--run`, the exit code of the compiled program is propagated (masked to
8 bits, like a shell). Success is `0` when the program returns `0`.

`@inc` / `@cinc` are *not* errors — they are the inclusion directives (see
below). Every inclusion starts with the `@` sigil: there is **no `import`
keyword**, and `@include` / `#include` are reported as errors pointing at their
replacement.

### Includes

All forms start with `@`:

```loi
@inc "path/to/file.loi"   // quoted path
@inc path/to/file.loi     // bare path
@inc std                  // package form
@inc std.math             // dotted package (dots become path separators)
@cinc <stdio.h>           // verify a C header exists
@cinc "stdio.h"
```

- `@inc` inlines a loi library or package. Resolution searches the including
  file's directory, `./lib`, `./packages`, `/usr/share/loi`,
  `/usr/local/share/loi`, and every entry in `LOI_PATH`. `@inc pkg.sub` maps
  dots to path separators (`pkg/sub`). Recursion is capped at depth 16.
- **Include guards**: each distinct real path is included at most once per
  compilation (like `#pragma once`), so libraries need no manual guards and
  include cycles terminate safely.
- `@cinc <header.h>` / `@cinc "header.h"` — verifies a C header exists
  (searching `.`, `/usr/include`, `/usr/local/include`,
  `/usr/include/x86_64-linux-gnu`); the header is *not* inlined. Since C ABIs
  are declared in loi via `extern`, `@cinc` leaves a trace marker in the source.

## Language highlights

- **Declarations**: variables are `x: int = 5;` or `int x = 5;`. Functions use
  the **canonical assignment form** `name = (params...): Ret { ... }`; the
  equivalent signature-first form `name (params...): Ret = { ... }` is also
  accepted. `:` and `->` are interchangeable before a return type in a
  declaration (`name = (...): int` / `name = (...) -> int`).
- **Statements require semicolons** (keeps line-oriented expression merging
  impossible).
- **Pointers**: `int* p = &x; *p = 7; p[i]`.
- **Arrays**: one canonical form only — the length follows the element type:
  `int[10] a;` (fixed) or `int[] a;` (inferred). The old prefix form
  `[10]int` is **not supported**.
- **Lexical memory regions**: `region buf(64) { ... }` — bump-allocated at
  block entry, freed at block exit (unless terminated by `return`).
- **Effect annotations**: `worker = (p: void*): void* !IO,Alloc { ... }` —
  **enforced** at direct calls, indirect calls, assignment, and when a value is
  passed as a callback argument or returned from a function. A function may only
  perform the effects it declares, and an `!IO` function cannot be stored in a
  variable whose type has no `!IO`.
- **Structs with layout control**: `struct S @pack() { ... }`, `@align(n)`.
- **Control flow**: `if/else`, `while`, `do { } while (...)`, `for`, `switch/case/default`, `break`, `continue`.
- **Typed function pointers**: `type Cmp = (a: int, b: int) -> int;` — including
  effects on the pointer type (`(n: int) -> void !IO`), passed as callbacks and
  called indirectly.

## Metaprogramming (`@` sigil)

```loi
@(define LIMIT 8)              # object-like macro (also: @define LIMIT 8)
@(define ADD(a, b) ((a)+(b)))  # function-like macro
@(define SUM(...xs) ...)       # variadic macro; body may use @stringify/@paste
@inc "helpers/lib.loi"         # include a loi file or package (@inc / @cinc)
@cinc "stdio.h"                # verify a C header exists
@(define FLAG)                 # @undef / @error also available

@if(defined(FLAG)) { ... } @elif(LIMIT > 4) { ... } @else { ... }

@repeat(3) { total = total * 2; }      # compile-time unrolling
@for(i, 0, 5) { total = total + i; }   # unrolled with metavariable subst.
@for(k, 0, 10, 2) { ... }              # optional step

int sz = @sizeof(Point);               # compile-time reflection
int off = @offsetof(Point, z);
int al = @alignof(Point);
@typeof(sz) copy = sz;                 # type of an expression

@section("my_text")                    # place a function in an asm section
fast_path = (n: int): int { return n * 2; }
```

Conditional blocks `@if/@elif/@else` select code at compile time; a
`@(define)` inside a skipped branch is never registered. `@repeat`/`@for`
expand to real code before parsing, so there is no runtime loop overhead.

**Compile-time reflection**: `@sizeof(T)`, `@alignof(T)`, `@offsetof(T, f)`,
`@typeof(expr)` — all fold to constants before codegen.

**Section placement**: `@section("name")` places a function into a named
assembly section (emits `section "name"` in LLVM IR, `.section name` in asm).

**Macro trio**: `@stringify(x)` turns the raw token text of the argument into a
string (`@stringify(1 + 2)` -> `"1 + 2"`), and `@paste(a, b)` joins two token
fragments into a single identifier (`@paste(foo, bar)` -> `foobar`). Variadic
macros capture the trailing arguments under the declared name.

See `docs/syntax.md` for the full grammar and specification.

## Examples

| File | Demonstrates |
|------|--------------|
| `examples/01_basics.loi` .. `09_channels.loi` | core language, memory, threads, arrays, structs, contracts, channels |
| `examples/10_layout.loi` | struct layout / packed |
| `examples/11_regions.loi` | lexical regions |
| `examples/12_effects.loi` | effect annotations |
| `examples/13_pointers.loi` | pointers & dereference |
| `examples/14_macros.loi` | `@(define)` macros |
| `examples/15_include.loi` | `@(inc)` |
| `examples/16_metaprogramming.loi` | macros + `@if` |
| `examples/17_conditionals.loi` | `@if/@elif/@else` |
| `examples/18_branch_defines.loi` | branch-scoped defines |
| `examples/19_compiletime_codegen.loi` | `@repeat` / `@for` unrolling |
| `examples/20_reflection.loi` | `@sizeof` / `@offsetof` / `@typeof` |
| `examples/21_sections.loi` | `@section` placement |
| `examples/22_switch.loi` | `switch`/`case`/`default` |
| `examples/23_do_while.loi` | `do { } while (...)` loops |
| `examples/24_effects_enforced.loi` | effects enforcement demo |
| `examples/25_fnpointers.loi` | typed function pointers (alias type, callback params, effects through pointers) |
| `examples/26_macro_trio.loi` | `@stringify` / `@paste` / variadic macros |
| `examples/main.loi` | multi-file build with `@inc` |

```sh
./loi examples/20_reflection.loi --run
```

## Source layout

- `src/token.*`, `src/lexer.*` — tokens and lexer
- `src/ast.*` — AST nodes, types, printing, freeing
- `src/parser.c` — recursive-descent parser (also resolves `@typeof`)
- `src/typecheck.c` — semantic checks (scopes, calls, assignments, effects)
- `src/codegen.c` — LLVM IR emitter (incl. struct layout math)
- `src/preprocessor.*` — the whole `@`/`#` layer in its own module: macros,
  `@if/@elif/@else`, `@repeat`/`@for`, `@inc`/`@cinc` includes, include guards.
  Exposes a small API (`loi_preprocess`) so `main.c` only drives the pipeline.
- `src/main.c` — CLI / driver (file loading, flags, invoking lexer→parser→
  typecheck→codegen→clang)
- `src/runtime.c` / `src/runtime.h` — runtime support (IO, threads,
  regions, math), linked into every executable

## Tests

```sh
make test          # or: bash tests/run.sh
```

`tests/run.sh` compiles and runs every `tests/*.loi`:

- tests whose name contains `fail` (plus a few explicitly listed) must be
  rejected by the compiler (non-zero exit);
- all other tests must succeed with a specific program exit code.

Coverage includes: effect enforcement (call sites, indirect calls, assignment),
metaprogramming (`@if`, `@repeat`, `@for`, macros, `@stringify`/`@paste`,
variadics), reflection (`@sizeof`/`@offsetof`/`@alignof`/`@typeof`), struct
layout (`@pack`/`@align`), regions, typed function pointers, include guards, and
error cases for parser, typechecker and preprocessor.

## Runtime

- Built-in runtime functions (`loi_print_*`, math, threads, regions, allocation) are linked into every executable automatically; `@inc std` is optional.
- The `std` package (in `lib/std.loi` and `lib/std/*.loi`) can be loaded via `@inc std` when you want package-form wrappers.
