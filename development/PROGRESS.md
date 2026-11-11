# Loi Compiler — Implementation Progress

## Current Status

- **Phase:** 4 — the C11 backend is the production path. `loi gen` writes C,
  `loi build` compiles and links executables, objects and static libraries.
- **Last completed task (2026-11-11):** made Loi comfortable for systems
  programs:
  1. added bitwise/shift operators (`&`, `|`, `^`, `~`, `<<`, `>>`), compound
     forms (`%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`) and checked shift semantics;
  2. added file/stdin I/O (`read_file`, `write_file`, `read_stdin`,
     `read_line`) and numeric output (`print_int`, `print_float`) as
     intrinsics, with the reading intrinsics participating in `protect`
     escape analysis;
  3. made `loi fmt` preserve `//` comments and stay idempotent.
  Growable collections (`Vec<T>`, maps) remain deferred pending the
  generics/comptime decision.
- **Overall health:** good; the full test suite, `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` pass.
- **Not done:** `comptime`, resources/`guard`, real module namespaces,
  composite globals, growable collections (`Vec<T>`/maps, deferred pending the
  generics/comptime decision), the IR/ASM backends and browser integration
  tests.

## Component Status

| Component | Status | Notes |
|---|---|---|
| Lexer | Working | Canonical Flow Syntax only; strings with decoded escapes; decimal/hex/binary integers; floats; `<-`/`->`/`<->`/`=>` glyphs; bitwise/shift/compound tokens; `//` comments collected for `fmt` |
| AST / parser | Working | Shapes, flows, `extern flow ... as`, `opaque`, imports, globals, slice/shape literals, `give`/`clone`/`alloc`, `protect`, loops, bitwise/shift precedence |
| Pretty-printer | Working | Emits canonical Flow Syntax; idempotent on all canonical files; `loi fmt` preserves `//` comments (`format_program` remains comment-free) |
| Type checker | Working | Types, literals, directions, calls, globals, intrinsics, `opaque`/`cstr`, constant folding, shape-cycle detection, bitwise/shift rules, I/O intrinsic signatures and allocation effects |
| Ownership | Working | Give liveness, definite assignment/return, exclusive `->`/`<->`, cross-flow consumption, `protect` taints and lexical-block escape analysis for slice literals, `read_file`/`read_stdin`/`read_line` |
| C backend | Working | One C11 translation unit, ordered shape layouts, checked arithmetic, bitwise/shift helpers, growable arena, bounds checks, defaults, headers, source maps, multi-file objects, statics, file/stdin I/O and numeric printing |
| C interop | Working | `opaque` → `void *`, `cstr` → `const char *`, bodyless `extern flow`, `--c-source`, `--cc-flag`; string literals coerce to `cstr` |
| Binding generator | Working | `tools/bindgen/bindgen.py` emits `opaque` handles, safe `shape`s, typed constants, `extern flow`s, a `memcpy` shim with `_Static_assert` checks and a skip report |
| Vendored bindings | Working (syntax-verified) | `vendor/sdl2/` (SDL2 2.32.10) and `vendor/raylib/` (raylib 5.5), each with generated `.loi`, shim, skip report and README |
| IR | Draft | `src/ir.rs`; `--emit-ir` writes text. Globals and some literals are not lowered |
| Assembly | Disabled draft | `src/asm.rs` cannot generate runnable code; the CLI rejects `--emit-asm` with exit 2 |
| CLI | Working | `lex`/`parse`/`fmt`/`check`/`gen`/`build`/`clean`, incremental stamps, profiles, source maps, `--arena-size`, `--c-source` |
| Website / playground | Working (local only) | Static site plus `website/server.mjs` driving the real compiler; `127.0.0.1` by default with source/output/time limits |

## Verification

Last verified 2026-11-11 with:

```sh
export PATH=/root/.cargo/bin:$PATH
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

- `cargo test --all-targets`: **196 passed, 0 failed** (suite breakdown is
  printed by Cargo; compile-and-run tests skip when `cc` is unavailable).
- New coverage: bitwise/shift/compound operators, the shift-count trap at the
  count's full width, file round-trips, stdin/`read_line`, numeric printing,
  comment preservation and idempotence in `loi fmt`, comment positions in the
  lexer, and I/O intrinsic signatures/allocation escape in the checker.
- `examples/systems.loi` builds and runs (`2215315943 1193046 1.5`), and
  `loi check` passes on `examples/raylib_window.loi`.
- `loi check vendor/sdl2/sdl2.loi` and `loi check vendor/raylib/raylib.loi`:
  exit 0.
- `clang -std=c11 -fsyntax-only` on both generated shims: exit 0.
- `python3 tools/sync_examples.py --check`: `website/examples` is in sync.
- `loi check` on the complete program snippets in `website/docs.html` and the
  README example, plus the build/run of the README example: exit 0. Shorter
  documentation fragments were reviewed against `SINTAXE.md`.
- Re-running the raylib generator produces byte-identical `.loi`, shim and skip
  report files.
- Full SDL2/raylib link-and-run is blocked locally by missing runtime
  dependencies (X11, Wayland, ALSA, GL, ...); the bindings and shims are
  syntax-verified only.

## Known Bugs / Limitations

### Semantics and analysis

- Initialization and ownership-transfer tracking are whole-path based. Writing any field of
  an uninitialized `->` parameter initializes the whole value; joining a
  consumed value with a live one keeps the conservative (consumed) state. Loops are
  treated as possibly executing zero times.
- Cross-flow consumption is summarized per whole parameter. Partial consumption
  (for example only `p.pos`) is widened to the whole argument.
- `protect` taints follow locals, fields, ownership transfers, shape literals,
  `->` arguments and returns, and are conservatively forwarded from consumed
  `<->` arguments to every slice-containing output or composite result. They
  are not tracked through callees that only mutate `<->` storage in place.
- The entry-point heuristic treats `main` and every uncalled non-`sealed`
  flow as an entry for the allocation check; an uncalled `sealed` helper may
  rely on a caller's `protect`.
- Slice literals are rejected when they escape their block through a return, a
  variable declared outside the block or a shape that escapes. This is lexical
  escape analysis, not a full interprocedural lifetime system.
- `clone` of any value containing a slice is rejected, recursively through
  shapes.
- There are no implicit conversions between named numeric types. Unsuffixed
  literals coerce; a literal that does not fit the expected type is rejected,
  while explicit `as` casts truncate without a range check.
- Constant folding covers literals, unary negation and `+`, `-`, `*`, `/`,
  `%` for diagnostics only. Comparisons, casts, floats and identifiers are not
  folded. The generated C evaluates the original expression.
- An inferred integer literal defaults to `i32`; wider constants need an
  explicit annotation.
- Shape default expressions are checked in an empty environment, so they
  cannot refer to other fields or parameters.
- `%` is restricted to integer operands. Shape equality is not structural;
  compare fields explicitly.

### Arithmetic

- Signed `+`, `-`, `*` and unary `-` trap on overflow; unsigned `+`, `-`, `*`
  wrap at the declared width; `/` and `%` trap on zero; `MIN / -1` traps and
  `MIN % -1` is `0`. Compound assignments follow the binary rules.
- `&`, `|`, `^` and `~` require integer operands. `<<` and `>>` require an
  integer count in `[0, width)`; out-of-range counts trap. Left shifts wrap at
  the declared width, unsigned right shifts are logical and signed right shifts
  are arithmetic. `as` remains the explicit unchecked truncating cast.
- Runtime traps print `loi: <message> at <line>:<column>` and exit 1. There is
  no recovery or unwinding.

### Values, memory and the runtime

- The arena is process-global, not thread-safe, and never returns memory to the
  OS. It grows in chunks; `--arena-size` controls only the first chunk. Leaving
  a `protect` block restores the logical offset.
- Slice indexing is bounds-checked through `loi_index`. There is no recovery
  after a runtime abort.
- Strings are decoded UTF-8 `[u8]` values; the C backing array has a trailing
  NUL that is not part of the Loi length. `\xNN` escapes are ASCII-only.
- `cstr` and `opaque` are FFI-only handles. They can be stored, passed,
  returned and compared by identity but not indexed, measured, printed or
  dereferenced. Byte access requires a hand-written shim.
- globals are limited to scalars, `bool` and `[u8]` initialized from a string.
  Composite globals need a runtime constructor.
- `==`/`!=` are defined for scalars, `bool` and `[u8]` only.
- Imports are textual inclusion without namespaces or cross-file visibility
  enforcement. Multi-file builds give sealed flows external linkage so
  cross-file calls link.
- I/O is limited to `print`, `eprint`, `panic`, numeric printing and whole-file
  / whole-stdin reads and writes. There is no socket, directory, seek,
  environment or process API. `read_file`, `read_stdin` and `read_line`
  allocate in the current `protect` region and cannot escape it. Resources
  (`guard`) are specified but not implemented.
- `comptime` is lexed but not parsed. Freshness, spatial tiers and ordering
  blocks are not implemented.
- The pretty-printer prints shape literals on one line. `loi fmt` reattaches
  `//` comments to source-line anchors, so comments inside multi-line
  expressions can move to the next statement boundary; output becomes stable
  after that first normalization. Newlines are statement separators, not free
  whitespace inside expressions.

### Backends and tooling

- `loi gen` rejects a `main` with parameters or a non-integer return type
  because the emitted entry point is `int main(void)`.
- Generated C assumes a checked program; calling `generate_c` on an unchecked
  AST may emit invalid C.
- `loi build` depends on `cc`/`$CC` and `ar`/`$AR`. Incremental stamps hash
  content and tool paths, not mtimes; `--rebuild` is the escape hatch.
- `--cc-flag` values are one literal argument each and are not passed to `ar`.
- The IR backend does not lower globals or every literal; the assembly backend
  is disabled at the CLI. Neither is a production path.
- The binding generator is conservative. Callbacks, variadics, unions,
  bitfields, inline arrays, pointer-to-pointer, mutable `char *` buffers, wide
  strings and pointers to unsafe records are skipped and reported.
- Full SDL2/raylib link-and-run needs their runtime dependency closure, which
  this development environment does not provide.
- The playground executes arbitrary Loi with the server process's privileges.
  It is a local development tool, not a sandbox, and must not be exposed
  publicly without real isolation.

## Open Design Questions

Language-level questions are resolved in `development/LOI_SPEC.md` section 5.
Remaining implementation or secondary choices:

- Enforce cross-file visibility and replace textual imports with a real module
  and namespace design.
- Decide how composite globals are constructed at startup.
- Define structural equality for shapes, or keep the current field-by-field
  rule permanently.
- Decide whether callbacks, variadics, safe unions or fixed arrays deserve
  first-class Loi syntax or remain shim-only.
- Define the byte-slice/`cstr` conversion surface beyond string literals.
- Extend escape analysis beyond lexical blocks if programs need block-local
  slices to outlive their block without `alloc`.
- Revisit `<-` composite passing if concurrency or observable aliasing is ever
  introduced; today the C backend passes by value and the checker enforces
  read-only use.
- Finish or remove the IR/ASM drafts. Until then `--emit-ir` stays diagnostic
  and `--emit-asm` stays rejected.

## Next Steps

1. Add a real module/namespace design and enforce `open`/`sealed` across files.
2. Implement `comptime` per 5.1: parser support, a step-limited constant
   evaluator and constant contexts.
3. Design linear resources (`guard`, `<Shape>_release`) before adding file or
   socket APIs.
4. Extend the binding generator for the C constructs that still require
   hand-written shims, and add window/render examples for SDL2 and raylib on a
   machine with the runtimes installed.
5. Finish IR/ASM or remove them from the published CLI surface.
6. Keep the docs in sync: `SINTAXE.md` is canonical, `website/docs.html`
   mirrors it, and `python3 tools/sync_examples.py --check` guards the example
   mirror.
