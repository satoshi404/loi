# loi

A compiler toolchain for the **loi** programming language — a small systems
language that mixes low-level constructs (pointers, structs, inline asm,
threads) with mathematical syntax and a metaprogramming layer built around
the `@` sigil.

The compiler is written in C and emits LLVM IR, which is optimized with
`opt -O3` and lowered to a native executable with `clang -O3` (AArch64 or
whatever host clang targets).

## Build

Requirements: `clang`, `opt` (LLVM), a C compiler, pthreads.

```sh
make          # builds ./loi
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
- preprocessor errors (`@(error ...)` / `@error ...`)
- missing `@(include ...)` files
- backend failures (clang/opt could not run)

With `--run`, the exit status of the compiled program is propagated. Note
exit codes are 8-bit (`return 300` becomes `44`).

## Language highlights

- **Hybrid declarations**: `x: int = 5;` or `int x = 5;`; functions are
  declared by assignment — `add = (a: int, b: int): int { return a + b; }`
- **Statements require semicolons** (keeps line-oriented expression merging
  impossible).
- **Pointers**: `int* p = &x; *p = 7; p[i]`, array syntax `int[5] a;`
- **Lexical memory regions**: `region buf(64) { ... }` — bump-allocated at
  block entry, freed at block exit (unless terminated by `return`).
- **Effect annotations**: `worker = (p: void*): void* !IO,Alloc { ... }` —
  recorded in the AST and **enforced at call sites** (a function calling
  an `!IO` function must itself declare `!IO`).
- **Structs with layout control**: `struct S @pack() { ... }`, `@align(n)`.
- **Control flow**: `if/else`, `while`, `do { } while (...)`, `for`, `switch/case/default`, `break`, `continue`.
- **Arrays**: prefix form `[n]T`, `[]T` and postfix `T[n]`, `T[]`.

## Metaprogramming (`@` sigil)

```loi
@(define LIMIT 8)              # object-like macro (also: @define LIMIT 8)
@(define ADD(a, b) ((a)+(b)))  # function-like macro
@(include "helpers/lib.loi")   # relative include (also: #include, @include)
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
| `examples/15_include.loi` | `@(include)` |
| `examples/16_metaprogramming.loi` | macros + `@if` |
| `examples/17_conditionals.loi` | `@if/@elif/@else` |
| `examples/18_branch_defines.loi` | branch-scoped defines |
| `examples/19_compiletime_codegen.loi` | `@repeat` / `@for` unrolling |
| `examples/20_reflection.loi` | `@sizeof` / `@offsetof` / `@typeof` |
| `examples/21_sections.loi` | `@section` placement |
| `examples/22_switch.loi` | `switch`/`case`/`default` |
| `examples/23_do_while.loi` | `do { } while (...)` loops |
| `examples/24_effects_enforced.loi` | effects enforcement demo |
| `examples/main.loi` | multi-file build with `@include` |

```sh
./loi examples/20_reflection.loi --run
```

## Source layout

- `src/token.*`, `src/lexer.*` — tokens and lexer
- `src/ast.*` — AST nodes, types, printing, freeing
- `src/parser.c` — recursive-descent parser (also resolves `@typeof`)
- `src/typecheck.c` — semantic checks (scopes, calls, assignments)
- `src/codegen.c` — LLVM IR emitter (incl. struct layout math)
- `src/main.c` — CLI, preprocessor (`@`/`#` directives, macros,
  conditionals, compile-time unrolling)
- `src/runtime.c` / `src/runtime.h` — runtime support (IO, threads,
  regions, math), linked into every executable

## Known limitations

- No C-standard-header ABI when including system headers; no include guards.
- No stringizing (`#x`), token pasting (`##`), or variadic macros.
- Effect annotations are recorded but not enforced.
- Not full C parity (no variadics, no function pointers typed in the type
  system).
