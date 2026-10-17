# loi

A compiler toolchain for the **loi** programming language — a small systems
language that mixes low-level constructs (pointers, structs, inline asm,
threads) with mathematical syntax and a metaprogramming layer built around
the `@` sigil.

The compiler is written in C and emits LLVM IR, which is optimized with
`opt -O3` and lowered to a native executable with `clang -O3` (for whatever
target the host `clang` uses, e.g. x86-64 or AArch64).

## Philosophy

loi is best described as:

**C with lexical memory regions + tracked effects + lexical capabilities +
lightweight metaprogramming + explicit layout control.**

Rather than piling on new runtime machinery, loi keeps a C-sized core and adds
a few orthogonal, compile-time-checked ideas:

- **Lexical regions** for arena allocation with a clear lifetime, instead of
  manual `malloc`/`free` everywhere.
- **Effect annotations** (`!IO`, `!Alloc`) that are *enforced* at call sites and
  on function-pointer assignments, so a pure function cannot silently perform
  I/O.
- **Lexical capabilities** (`@cap`, `@grant`, `@needs`, `@gives`) — linear,
  zero-cost permission tokens, and **dual effects** (`!Effects` for what a
  function *does* vs `@needs` for what it *requires*). See
  [`docs/capabilities.md`](docs/capabilities.md).
- **Lightweight metaprogramming** under a single `@` sigil: macros, conditional
  compilation, unrolling and compile-time reflection — all erased before
  codegen, so there is zero runtime cost.
- **Explicit layout control** (`@pack`, `@align`, `@section`) for systems work
  where ABI and placement matter.
- **Packages everywhere**: every file declares `@package(name)`; files sharing a
  package form one namespace; and the runtime is reached through the `std`
  package (`std.print_int`, `std.alloc`, …) instead of leaking `loi_`-prefixed
  symbols into source.

Everything else stays close to C so the generated LLVM IR and the mental model
remain predictable.

## Build

Requirements:

- a C compiler (`gcc` by default; override with `CC=...`) and `make`, to build
  the compiler itself;
- `clang` and `opt` (LLVM) on `PATH`, to compile loi programs (`--emit=ir`
  falls back to unoptimized IR if `opt` is missing, but `asm`, `exe` and `--run`
  need `clang`);
- pthreads and libm (linked into every executable).

```
make          # builds ./loi
make test     # builds and runs the test suite (needs clang)
make clean    # removes object files and ./loi
```

> **Note:** run `make clean` before `make` if the checkout contains object
> files built on another architecture; otherwise the link step fails with
> `file in wrong format`.

Before compiling programs, make sure the scratch directory used for
intermediate files exists, and run `./loi` from the repository root (it links
`src/runtime.c` by relative path):

```
mkdir -p /tmp/opencode
```

## Usage

```
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

Both `--emit=ir` and `--emit ir` (and likewise `--out=path` / `--out path`)
are accepted. Without `--out`, `--emit=exe` writes to `/tmp/opencode/loi_bin`.

### Exit codes

Compilation errors always fail the build with exit status `1`:

- parse errors (`Parse error ...` + `parse had errors`)
- type errors (`Type error ...` + `type checking failed`)
- preprocessor errors (`@(error ...)` / `@error ...`, unresolved `@inc`)
- a source file that does not declare `@package(...)`

With `--run`, the exit code of the compiled program is propagated (masked to
8 bits, like a shell). Success is `0` when the program returns `0`.

### Includes

All forms start with `@`. There is **no `import` keyword** and no
`#include`-style directive: all inclusions use `@inc` and `@cinc`.

```
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
- `@inc`/`@cinc` are *not* errors on their own — only an unresolved include is.

## Language highlights

- **Every loi file declares a package** with `@package(name)` (dotted names are
  allowed: `@package(core.utils)`). The entry file and every `@inc`-ed file are
  checked, so a missing declaration is an error. **Files that share a
  `@package` form one namespace**, so a `local` symbol defined in one is
  reachable from the others; symbols never leak across packages.
- **Visibility & attributes** — `@attr(...)` attaches extensible metadata to the
  next declaration. `@attr(local)` keeps a symbol private to its package
  (default is `global`) — this applies to functions and variables;
  `link_name("sym")`, `section("name")`, `inline`, `no_return`, `pure`,
  `must_use`, `root` and `extern` are recognised, and unknown attributes are
  recorded but ignored. `@attr(must_use)` warns (advisory) when a call's result
  is discarded. `@root` is sugar for `@attr(root)`.
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
  **enforced** at direct calls, indirect calls, assignment, global
  initializers, and when a value is passed as a callback argument or returned
  from a function. A function may only perform the effects it declares, and an
  `!IO` function cannot be stored in a variable whose type has no `!IO`.
- **Lexical capabilities & dual effects** (see below).
- **Structs with layout control**: `struct S @pack() { ... }`, `@align(n)`.
- **Control flow**: `if/else`, `while`, `do { } while (...)`, `for`,
  `switch/case/default`, `break`, `continue`.
- **Typed function pointers**: `type Cmp = (a: int, b: int) -> int;` — including
  effects on the pointer type (`(n: int) -> void !IO`), passed as callbacks and
  called indirectly.

### Capabilities

A capability is a *linear*, zero-cost, compile-time-only permission token. It
erases to nothing in the generated code.

```
@package(demo)
@capability(Read, Write)

poke = (buf: ptr, c: @cap(borrow Write)): void { *buf = 42; }
peek = (buf: ptr, c: @cap(borrow Read)): int { return *buf; }

main = (): int
{
    int x = 0;
    @grant(Read + Write) proof;        // main is a root: it may birth permissions

    poke(&x, proof);                   // borrow: proof stays live
    poke(&x);                          // argument elision: the single live proof
    @with_cap(Write) { poke(&x); }     // make a held capability ambient for a block
    peek(&x) with Read;               // ...or for a single statement

    @drop(proof);                      // discard explicitly
    return x;
}

// producers and @root functions may also birth capabilities:
@root
mk_write = (): @cap(Write) { @grant(Write) w; return w; }
```

- **Types**: `@cap(Write)` (owned), `@cap(Read + Alloc)` (a set),
  `@cap(borrow Read)` (valid only for the call).
- **Statements**: `@grant`, `@refine`, `@drop`, `@with_cap(...) { }`, and the
  postfix `expr with Cap;` clause.
- **Roots**: `@grant` is only allowed in `main`, in a function marked `@root`,
  in one that declares `@gives(...)`, or in a *producer* whose return type is
  the capability.
- **Signatures**: `@needs(A + B)` states what a function *requires* (the dual of
  `!Effects`, which states what it *does*); `@gives(A)` states what it may
  create. A live capability discharges both `!Effects` and `@needs`.
- **Regions** can carry capabilities: `region r(64) with Write as w { ... }`.

Full specification, typing rules and diagnostics:
[`docs/capabilities.md`](docs/capabilities.md).

## Metaprogramming (`@` sigil)

```
@package(demo)                 // every loi file declares a package

@(define LIMIT 8)              // object-like macro (also: @define LIMIT 8)
@(define ADD(a, b) ((a)+(b)))  // function-like macro
@(define SUM(...xs) ...)       // variadic macro; body may use @stringify/@paste
@inc "helpers/lib.loi"         // include a loi file or package (@inc / @cinc)
@cinc "stdio.h"                // verify a C header exists
@(define FLAG)                 // @undef / @error also available

@if(defined(FLAG)) { ... } @elif(LIMIT > 4) { ... } @else { ... }

@repeat(3) { total = total * 2; }      // compile-time unrolling
@for(i, 0, 5) { total = total + i; }   // unrolled with metavariable subst.
@for(k, 0, 10, 2) { ... }              // optional step

int sz = @sizeof(Point);               // compile-time reflection
int off = @offsetof(Point, z);
int al = @alignof(Point);
@typeof(sz) copy = sz;                 // type of an expression

@section("my_text")                    // place a function in an asm section
fast_path = (n: int): int { return n * 2; }

@attr(local, inline)                   // extendible attributes on the next decl
helper = (n: int): int { return n + 1; }

@attr(global, link_name("loi_die"))    // rename the emitted symbol
die = (): void { while (1) { } }
```

Comments in loi source are `// ...` and `/* ... */`.

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

See [`docs/syntax.md`](docs/syntax.md) for the full grammar and specification.

## Runtime and the `std` package

The runtime is reached through the `std` package — the internal `loi_` prefix
is never written in loi source:

```
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
matching runtime symbol (`std.print_int` → `loi_print_int`,
`std.region_alloc` → `loi_region_alloc`, …). The helpers live in
`src/runtime.c` and are linked into every executable automatically, so
`@inc std` documents intent and keeps the source honest about where the
symbols come from.

What the runtime provides (see [`src/runtime.h`](src/runtime.h) for the exact
signatures):

| Area          | Functions                                                                  |
| ------------- | -------------------------------------------------------------------------- |
| Memory        | `alloc`, `free`, `gc_alloc`, `gc_collect`, `gc_add_root`, `gc_remove_root` |
| Regions       | `region_new`, `region_alloc`, `region_free`                                |
| I/O           | `print_int`, `print_str`, `write_file`                                     |
| Threads       | `thread_spawn`, `thread_join` (pthreads)                                   |
| Channels      | `chan_new`, `chan_send`, `chan_recv`, `chan_close`, `chan_free`            |
| Math          | `abs`, `min`, `max`, `sqrt`, `pow`, `sin`, `cos`, `tan`, `exp`, `log`, `floor`, `ceil` |
| Strings/bytes | `strlen`, `memcpy`, `memset`                                               |

The standard library tree under `lib/` currently holds `std.loi` (the virtual
package marker) and the placeholder packages `std/math.loi` and `std/os.loi`,
which only declare their `@package` so far.

## Examples

| File                                          | Demonstrates                                                                    |
| --------------------------------------------- | ------------------------------------------------------------------------------- |
| `examples/01_basics.loi` .. `09_channels.loi` | core language, control flow, math, memory, threads, arrays, structs, contracts, channels |
| `examples/10_layout.loi`                      | struct layout / packed                                                          |
| `examples/11_regions.loi`                     | lexical regions                                                                 |
| `examples/12_effects.loi`                     | effect annotations                                                              |
| `examples/13_pointers.loi`                    | pointers & dereference                                                          |
| `examples/14_macros.loi`                      | `@(define)` macros                                                              |
| `examples/15_include.loi`                     | `@inc` (file + package forms)                                                   |
| `examples/16_metaprogramming.loi`             | macros + `@if`                                                                  |
| `examples/17_conditionals.loi`                | `@if/@elif/@else`                                                               |
| `examples/18_branch_defines.loi`              | branch-scoped defines                                                           |
| `examples/19_compiletime_codegen.loi`         | `@repeat` / `@for` unrolling                                                    |
| `examples/20_reflection.loi`                  | `@sizeof` / `@offsetof` / `@typeof`                                             |
| `examples/21_sections.loi`                    | `@section` placement                                                            |
| `examples/22_switch.loi`                      | `switch`/`case`/`default`                                                       |
| `examples/23_do_while.loi`                    | `do { } while (...)` loops                                                      |
| `examples/24_effects_enforced.loi`            | effects enforcement demo                                                        |
| `examples/25_fnpointers.loi`                  | typed function pointers (alias type, callback params, effects through pointers) |
| `examples/26_macro_trio.loi`                  | `@stringify` / `@paste` / variadic macros                                       |
| `examples/main.loi`                           | minimal entry point (`examples/helpers/lib.loi` is used by `15_include.loi`)    |

```
./loi examples/20_reflection.loi --run
```

There are no capability examples under `examples/` yet; the best executable
references for `@cap`, `@grant`, `@with_cap`, `@needs` and friends are
`tests/43_cap_basic.loi` .. `tests/60_cap_fail_with_stmt.loi`.

## Source layout

- `src/token.*`, `src/lexer.*` — tokens and lexer
- `src/ast.*` — AST nodes, types, printing, freeing
- `src/parser.c` — recursive-descent parser (also resolves `@typeof`)
- `src/typecheck.c` — semantic checks (scopes, calls, assignments, effects,
  visibility, capabilities)
- `src/codegen.c` — LLVM IR emitter (incl. struct layout math and `std.*`
  rewriting)
- `src/preprocessor.*` — the whole `@` layer in its own module: macros,
  `@if/@elif/@else`, `@repeat`/`@for`, `@inc`/`@cinc` includes, include guards,
  and the package-declaration check. Exposes a small API (`loi_preprocess`,
  `loi_source_declares_package`) so `main.c` only drives the pipeline.
- `src/main.c` — CLI / driver (file loading, flags, invoking lexer→parser→
  typecheck→codegen→`opt`/`clang`)
- `src/runtime.c` / `src/runtime.h` — runtime support (IO, threads, channels,
  regions, GC hook, math), linked into every executable
- `lib/` — loi standard library packages (`std`, `std.math`, `std.os`)
- `docs/` — language documentation: [`syntax.md`](docs/syntax.md) and
  [`capabilities.md`](docs/capabilities.md)
- `site/` — static documentation website (`index.html`, `docs.html`,
  `examples.html`)
- `examples/`, `tests/` — see above and below

## Tests

```
make test          # or: bash tests/run.sh
```

`tests/run.sh` compiles and runs every `tests/*.loi` (60 programs, plus two
extra checks that `@attr(must_use)` warns on a discarded result and stays
quiet when the result is used):

- tests whose name contains `fail` (plus a few explicitly listed, such as the
  parse, type, preprocessor and removed-syntax cases) must be rejected by the
  compiler (non-zero exit);
- all other tests must succeed, and most of them check a specific program exit
  code listed in the script.

`tests/lib/` holds helper files that the include and package tests pull in.

Coverage includes: effect enforcement (call sites, indirect calls, assignment,
callbacks, returns, global initializers), metaprogramming (`@if`, `@repeat`,
`@for`, macros, `@stringify`/`@paste`, variadics), reflection
(`@sizeof`/`@offsetof`/`@alignof`/`@typeof`), struct layout (`@pack`/`@align`),
regions, typed function pointers, arrays, include guards and `@inc` forms,
packages (missing/bare/dotted declarations, shared namespaces, `std`),
attributes (`link_name`, visibility for functions and variables, flags,
`must_use`), capabilities (`@grant`/`@refine`/`@drop`, linearity, `@needs`,
the effects bridge, regions with capabilities, `@with_cap`, `with` clauses and
argument inference), and error cases for the parser, typechecker and
preprocessor.

## Known limitations

- `--run` and `--emit=exe|asm` write intermediates to a fixed `/tmp/opencode/`
  directory, which must already exist.
- The driver links `src/runtime.c` by relative path, so it must be run from the
  repository root.
- `std.math` and `std.os` are placeholders; math helpers are available through
  `std.*` runtime functions instead.
- Capabilities are not yet usable as struct fields; fractional permissions and
  typed dataflow are planned (see the roadmap in
  [`docs/capabilities.md`](docs/capabilities.md)).
