# Loi Compiler — Implementation Progress

## Current Status

- **Phase:** 4 — the C11 backend is the production path. `loi gen` writes C,
  `loi build` compiles and links executables, objects and static libraries.
- **Last completed task (2026-11-17):** made the embedded WebAssembly view
  actually interactive and added the audio demo. `website/playground.html` now
  has two tabs (bolas / síntese de som) that switch iframes; selecting a
  matching preset switches the tab too. `examples/build_web.sh` accepts
  `WEB_C_SOURCES`, so `sound_synth.loi` builds for the web with
  `examples/sound_shim.c`, and its generated wrapper focuses the canvas on the
  first click (GLFW keyboard input) while a hint explains that the click also
  unlocks the suspended WebAudio context. `examples/raylib_web.loi` now shows
  the selected tone name, a `PAUSADO` marker and a background color that
  changes with the arrow keys, so input is visible. Verified by rebuilding both
  demos with Emscripten 7.0.0 and checking that the generated JS parses.
- **Previous task (2026-11-17):** integrated the raylib examples into the
  website playground. The preset menu now offers the three examples, whose
  sources load from `website/examples/`; **Check** and **Format** work on them
  because the playground server also copies `vendor/raylib/raylib.loi` into each
  temporary workspace. **Run** detects a raylib import in the source and shows
  build guidance instead of trying to link a native window in the browser. The
  playground embeds a prebuilt WebAssembly demo in an iframe below the editor;
  `examples/build_web.sh` now emits `.js`/`.wasm` plus a CSP-friendly HTML
  wrapper (no inline scripts) into `website/web/`, and the server sends
  `frame-ancestors 'self'` / `script-src 'self' 'wasm-unsafe-eval'`. Verified:
  `node --check` on the JS, the server serves the three presets through
  `/api/check` (all `ok:true`) and the demo with `application/wasm`, and an
  Emscripten 7.0.0 + raylib `PLATFORM_WEB` build completes.
- **Previous task (2026-11-17):** added three interactive raylib examples
  and an Emscripten build path. `examples/raylib_web.loi` is a bouncing-ball demo
  that runs natively and compiles to WebAssembly with `examples/build_web.sh`
  (`CC=emcc`, raylib built for `PLATFORM_WEB`, `-sASYNCIFY`).
  `examples/text_editor.loi` edits a byte-slice buffer with the keyboard,
  draws a blinking cursor through `examples/editor_shim.c` and saves with
  `write_file`. `examples/sound_synth.loi` generates sine/square/saw/triangle
  waves sample by sample with math intrinsics plus `extern flow c_sin as sin`,
  draws them like an oscilloscope and plays them through `examples/sound_shim.c`
  (which hides raylib's skipped `Sound` record behind static state). All three
  pass `loi check`, generate C that passes `gcc -fsyntax-only`, and are mirrored
  to `website/examples/`; the website documents their build commands.
- **Previous task (2026-11-17):** expressions now carry full source
  spans from their first token through their last, including literals, calls,
  field/index chains, slice/shape literals, `give`/`clone`/`comptime`/`alloc`
  and multi-line expressions. Human diagnostics underline the whole first line
  with `...` for multi-line spans (tabs and UTF-8 stay aligned), and JSON
  includes `end_line`/`end_column` for primary and secondary labels. The
  parser's `Span` records an explicit `end_line`; short-format output is
  unchanged. Lowered the comptime call-depth limit from 128 to 64 because the
  wider spans increased debug-build stack frames enough to overflow at 128 on
  the recursive-comptime corpus case.
- **Previous task (2026-11-17):** made "did you mean" suggestions
  namespace-aware (variables/parameters/globals for names, flows for calls,
  shapes for types) with adjacent transpositions costing one edit, and poisoned
  lvalues after an unknown name so one root cause produces one error.
  `cuonter.value += 1` now suggests `counter` and emits only the unknown-name
  diagnostic; its golden case was re-blessed and removed from `KNOWN_BAD.txt`.
- **Previous task (2026-11-17):** routed lexer and parser errors through
  `Diagnostic`, so `loi lex/parse/fmt/check` all honor `--error-format`
  (human/short/json) and `--color`. Short output is now exactly
  `file:line:column: message`, with one file name and no `loi:` prefix;
  newlines in parser messages are described as `end of line`. The three
  previously known-bad lexer/parser corpus cases were re-blessed and removed
  from `KNOWN_BAD.txt`, with CLI tests covering all formats for one lexer and
  one parser error.
- **Previous task (2026-11-17):** added a golden corpus for `loi check
  --error-format short --color never` under `tests/errors/`, a runner that
  diffs every `NAME.loi` against its `NAME.expected` (`LOI_BLESS=1` rewrites
  them), and a clean-check test over every non-error example. `KNOWN_BAD.txt`
  records cases whose current output will be fixed by the following diagnostic
  commits. Also fixed a stray `prant("j")` typo in `examples/language_basics.loi`
  (and its website mirror) so the example suite is clean again.
- **Previous task (2026-11-17):** added a rich diagnostic renderer in
  `src/diagnostic.rs` with human, short and JSON output, ANSI colors,
  source-line underlines, secondary labels and edit-distance name suggestions.
  Existing message texts are unchanged; moved-value, protect/block escape and
  immutable-assignment errors now point at both the failing use and the
  originating declaration. `--error-format` and `--color` are available on
  every mode, and the playground requests short, uncolored output. Parser
  spans now carry an exact end column, with source-line scanning retained for
  synthetic and cross-file spans.
- **Overall health:** good; the full test suite, `cargo fmt --check` and
  `cargo clippy --all-targets -- -D warnings` pass.
- **Not done:** resources/`guard`, real module namespaces, growable collections
  (`Vec<T>`/maps), the IR/ASM backends and browser integration tests.

## Component Status

| Component | Status | Notes |
|---|---|---|
| Lexer | Working | Canonical Flow Syntax only; strings with decoded escapes; decimal/hex/binary integers; floats; `<-`/`->`/`<->`/`=>` glyphs; bitwise/shift/compound tokens; `//` comments collected for `fmt` |
| AST / parser | Working | Shapes and generic shapes, flows and generic flows, `extern flow ... as`, `opaque`, imports, globals, slice/shape literals, `give`/`clone`/`alloc`, `protect`, loops, bitwise/shift precedence, `comptime flow`/`comptime expr`, explicit type arguments |
| Pretty-printer | Working | Emits canonical Flow Syntax including generics and `comptime`; idempotent on all canonical files; `loi fmt` preserves `//` comments (`format_program` remains comment-free) |
| Type checker | Working | Types, literals, directions, calls, globals, intrinsics, `opaque`/`cstr`, constant folding, shape-cycle detection, bitwise/shift rules, I/O intrinsic signatures and allocation effects; monomorphization feeds concrete declarations into the checker and diagnostics show source generic spellings |
| Compile-time evaluation | Working | `src/const_eval.rs`: pure `comptime flow` interpreter with step/depth limits, constant globals and shape defaults, `comptime expr`, purity diagnostics |
| Diagnostics | Working | `src/diagnostic.rs`: human output with full source underlines (multi-line spans get `...`), secondary labels and an error summary; `short` keeps `path:line:column: message`; `json` emits structured objects with `end_line`/`end_column`; ANSI colors honor `NO_COLOR`/`CLICOLOR_FORCE`; unknown names get namespace-aware edit-distance suggestions; parser spans carry exact token ends and multi-line end lines with a line-scan fallback |
| Ownership | Working | Give liveness, definite assignment/return, exclusive `->`/`<->`, cross-flow consumption, `protect` taints and lexical-block escape analysis for slice literals, `read_file`/`read_stdin`/`read_line`; gives from globals are rejected and move state is per function body |
| C backend | Working | One C11 translation unit, ordered shape layouts, checked arithmetic, bitwise/shift helpers, growable arena, bounds checks, defaults, headers, source maps, multi-file objects, statics, file/stdin I/O and numeric printing |
| C interop | Working | `opaque` → `void *`, `cstr` → `const char *`, bodyless `extern flow`, `--c-source`, `--cc-flag`; string literals coerce to `cstr` |
| Binding generator | Working | `tools/bindgen/bindgen.py` emits `opaque` handles, safe `shape`s, typed constants, `extern flow`s, a `memcpy` shim with `_Static_assert` checks and a skip report |
| Vendored bindings | Working (syntax-verified) | `vendor/sdl2/` (SDL2 2.32.10) and `vendor/raylib/` (raylib 5.5), each with generated `.loi`, shim, skip report and README |
| IR | Draft | `src/ir.rs`; `--emit-ir` writes text. Globals and some literals are not lowered |
| Assembly | Disabled draft | `src/asm.rs` cannot generate runnable code; the CLI rejects `--emit-asm` with exit 2 |
| CLI | Working | `lex`/`parse`/`fmt`/`check`/`gen`/`build`/`clean`, incremental stamps, profiles, source maps, `--arena-size`, `--c-source`, `--error-format`, `--color` |
| Website / playground | Working (local only) | Static site plus `website/server.mjs` driving the real compiler with `--error-format=short --color=never`; `127.0.0.1` by default with source/output/time limits |

## Verification

Last verified 2026-11-17 with:

```sh
export PATH=/root/.cargo/bin:$PATH
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
python3 tools/sync_examples.py --check
```

- `cargo fmt --check`: exit 0.
- `cargo clippy --all-targets -- -D warnings`: exit 0.
- `cargo test --all-targets`: **251 passed, 0 failed** (19+6+1+101+39+42+2+6+23+12 across all targets; compile-and-run tests run when `cc` is available).
- `python3 tools/sync_examples.py --check`: `website/examples` is in sync.
- Playground integration: `node --check website/assets/playground.js` and
  `node --check website/server.mjs` exit 0; with `node website/server.mjs`
  running, `GET /playground.html`, `/assets/playground.js`, `/web/raylib_web.html`
  and `/web/raylib_web.js` return 200, `/web/raylib_web.wasm` returns 200 with
  `application/wasm`, and `POST /api/check` returns `ok:true` for the three
  raylib examples. Emscripten 7.0.0 with raylib built for `PLATFORM_WEB`
  completes `sh examples/build_web.sh examples/raylib_web.loi` into
  `website/web/` (no browser was available locally to exercise the iframe).
  The same command with `WEB_C_SOURCES=examples/sound_shim.c` and
  `examples/sound_synth.loi` also exits 0; `website/web/` now holds both demos
  and `node --check` passes for every generated `.js`.
- New examples: `raylib_web.loi`, `text_editor.loi` and `sound_synth.loi`
  (with `examples/editor_shim.c`, `examples/sound_shim.c` and
  `examples/build_web.sh`), mirrored to `website/examples/` and documented in
  `website/docs.html`. Each passes `loi check` and `loi gen` followed by
  `gcc -std=c11 -fsyntax-only -Ivendor/raylib/include`; full link/run needs a
  native raylib plus `-lraylib -lm` (unavailable locally).
- New coverage: full expression span extents (literals, calls, field chains,
  multi-line spans), multi-line underlines with `...`, JSON end positions,
  tab/UTF-8 alignment, namespace-separated suggestions, transposition-aware edit
  distance and single-error cascade suppression for unknown names; all three
  diagnostic formats plus color for one lexer and one parser error, and 31 golden error cases (ownership, global give, aliasing,
  comptime, generics, lexer/parser errors, line-1/mid-file/last-line, UTF-8,
  tabs and CRLF) plus a clean check of every example; diagnostic rendering to strings without colors (target layout,
  token extent scanning, exact span ends, short and JSON formats,
  sorting/summary and suggestions), parser token extents, CLI `--error-format`
  and `--color` behavior, generic shapes and flows (including nested
  instantiations, `>>` token splitting and backtracking, comparisons around
  `<`/`>`), monomorphized C symbols, recursive-instantiation depth limits,
  generated-name collisions, validation of unused templates, pure `comptime`
  flows and expressions with signed overflow, unsigned wrap, cast and `f32`
  parity, `comptime`-initialized globals, strict-C11 composite globals,
  formatter idempotence for generics and `comptime`, global-`give` rejection
  for slices, fields, elements and strings, per-function move state independent
  of declaration order, `clone`/reads of globals, and source-spelling
  diagnostics for generic instantiations. The previous bitwise/shift,
  file/stdin I/O, numeric printing and comment-preserving `fmt` coverage still
  passes unchanged.
- `examples/systems.loi` builds and runs (`2215315943 1193046 1.5`), and
  `loi check` passes on `examples/raylib_window.loi`.
- `loi check vendor/sdl2/sdl2.loi` and `loi check vendor/raylib/raylib.loi`:
  exit 0.
- `clang -std=c11 -fsyntax-only` on both generated shims: exit 0.
- `python3 tools/sync_examples.py --check`: `website/examples` is in sync.
- `loi check` on the complete program snippets in `website/docs.html` and the
  README example, plus the build/run of the README example: exit 0. This
  includes the new generics and `comptime` snippets. Shorter documentation
  fragments were reviewed against `SINTAXE.md`.
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
- Globals may use any compile-time constant: scalars, strings, slices and
  nested shapes. The backend folds them once and emits strict C11 static
  initializers: brace/designated initializers for shapes and named `static`
  arrays for slice data, with no compound literals.
- `==`/`!=` are defined for scalars, `bool` and `[u8]` only.
- Imports are textual inclusion without namespaces or cross-file visibility
  enforcement. Multi-file builds give sealed flows external linkage so
  cross-file calls link.
- I/O is limited to `print`, `eprint`, `panic`, numeric printing and whole-file
  / whole-stdin reads and writes. There is no socket, directory, seek,
  environment or process API. `read_file`, `read_stdin` and `read_line`
  allocate in the current `protect` region and cannot escape it. Resources
  (`guard`) are specified but not implemented.
- `comptime` is implemented for pure flows, expressions, globals and shape
  defaults. Freshness, spatial tiers and ordering blocks are not implemented.
- Generic declarations are templates; only instantiations reach checking and
  code generation. There is no type inference and no user-visible `Type::Generic`
  after monomorphization. Unused templates are validated once with `i32`
  placeholders in the checker only; recursive instantiation stops at a depth
  limit, and generated-name collisions are diagnosed with the readable
  instantiation and the mangled name.
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
2. Design linear resources (`guard`, `<Shape>_release`) before adding file or
   socket APIs.
3. Design growable collections (`Vec<T>`, maps) as library code now that
   generics and explicit monomorphization exist.
4. Extend the binding generator for the C constructs that still require
   hand-written shims, and add window/render examples for SDL2 and raylib on a
   machine with the runtimes installed.
5. Finish IR/ASM or remove them from the published CLI surface.
6. Keep the docs in sync: `SINTAXE.md` is canonical, `website/docs.html`
   mirrors it, and `python3 tools/sync_examples.py --check` guards the example
   mirror.
