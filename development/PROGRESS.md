# Loi Compiler — Implementation Progress

## Current Status

- **Phase:** 4 — the C11 backend is the production path. `loi gen` writes C,
  `loi build` compiles and links executables, objects and static libraries.
- **Current task (2026-11-18):** fixed the five reported library/runtime issues.
  `main` is now the only allocation entry point for the checker, so documented
  library flows and shapes are `open` and appear in generated C headers while
  internal helpers remain `sealed`; Loi call sites are still checked. Random
  zero seeds use the named nonzero `RNG_FALLBACK_SEED`. `crc32_extend` chains
  finalized CRC values zlib-style. `map_*_get_or` and `map_*_lookup` return a
  default or a `found` status in one scan. `vec_reserve` is exported and the
  docs explain pre-sizing because the arena retains older growth blocks.
- **Verification for the current task:** new runtime tests check the zero-seed
  state `11400714819323198485` and first draw `973819730272012410`, chained
  CRC chunks against `3421780262`, and status/default map lookups. A new test
  generates a header containing the exported `Vec`, `Rng`, `IntMap` and
  `StrMap` types plus the public flow prototypes. Library tests again passed
  strict C11, UBSan execution and ASan+UBSan compile-only checks; no ASan
  execution is claimed in this PRoot container. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **322 passed,
  0 failed** (22+6+1+114+42+51+2+8+33+26+17).
- **Previous task (2026-11-18):** expanded the canonical example set with two
  runnable Flow Syntax programs: `examples/marks.loi` covers a finite tag set,
  equality, state transitions and a mark stored in a shape; `examples/choices.loi`
  covers tagged payloads, `is` tests, checked payload reads, a mark payload,
  `give` into a shape and a nested choice field. Both are mirrored by
  `website/examples/`, linked from `README.md` and `website/docs.html`, and
  listed in `SINTAXE.md`; `loi build` plus execution printed `marks ok` and
  `choices ok` with exit 0.
- **Previous task (2026-11-18):** Loi-native sum types. Added `mark` (payload-free
  finite tag set), `choice` (tagged payload) and `is` (variant test) across the
  lexer, AST, parser, checker, formatter, monomorphization pass and C11 backend.
  Marks compare only with the same mark; choice constructors take exactly one
  `<-` payload; `value.Variant` reads are checked at runtime and abort with the
  declaration, variant and source position. Payloads are limited to numeric
  scalars, `bool`, marks and slice-free shapes. The C backend emits `uint32_t`
  marks with macros and choices as a tag plus C union with checked getters.
  Bindgen maps only zero-based contiguous C enums to marks through the C shim;
  sparse enums remain integer constants and arbitrary C unions remain skipped.
  That task's gate also passed: 316 tests, strict C11, sanitizer compile-only
  and a wrong-payload abort of
  `loi: choice 'Result' does not hold 'Err' at 20:8` with exit 1.
- **Previous task (2026-11-18):** LLVM optimization evaluation (analysis only;
  no compiler, language or CLI change). Confirmed that `clang 21.1.8`, `opt`,
  `llc`, `llvm-config`, `llvm-as`, `llvm-dis` and `lli` are available locally and
  that `Cargo.toml` still has no dependencies. `src/ir.rs` is a diagnostic
  three-address draft, not LLVM IR: it uses `ptr<T>` / `slice<T>` / `jump` /
  `imm`, has no SSA or globals, defaults inferred types to `i32`, lowers
  `ForRange` only partially and skips `ForEach`. Recommendation: keep the C11
  backend as the default and semantic oracle; prototype an **opt-in** textual
  LLVM lowering of the same checked, monomorphized AST, drive external
  `clang`/`opt` through `std::process::Command` to preserve the dependency-free
  Cargo policy, and link the existing runtime through exported C wrappers rather
  than reimplementing `loi_runtime_fail`, `loi_alloc_slice`, `loi_index` and
  `loi_slice_eq`. Gate any build-mode switch on stdout/stderr/exit parity. A
  local, non-committed probe compiled the same generated C with gcc 15.2.0 and
  clang 21.1.8, minimum of 5 pinned runs on CPU 6: `hash 0.6130/0.5623` (clang
  1.090× faster), `sort 0.2556/0.2644` (0.967×), `int_map 0.0729/0.1112`
  (0.655×), `random 0.4967/0.5316` (0.934×); outputs matched. Clang also emits
  `-Wparentheses-equality` warnings on generated C, so replacing `CC` is not a
  valid optimization strategy. No backend decision is recorded yet; see
  "Open Design Questions".
- **Previous task (2026-11-18):** safety/regression review of the shipping
  libraries and benchmark harness. The requested `ocr_review` tool is not
  installed as a skill, command or plugin in this environment, so the review was
  manual. Corrected `development/FINAL_REPORT.md`: 30 of 32 `tests/lib.rs`
  cases run a Loi program, and C variants are checked only where the helpers
  emit them. Hardened `benchmark/run.py` to derive the pin CPU from the current
  `taskset` affinity (preferring CPU 6), use a unique temporary build directory,
  split `CC` with shell-like rules and fall back from `lscpu` to
  `/proc/cpuinfo`; added key-capacity guards to the two map C references. The
  suspected `sort_ref.c` signed-conversion issue was checked and is defined
  because the modulus bounds values to 1,000,002. An accidental untracked `~/`
  npm-artifact directory created during this session was moved out of the repo
  to `/tmp/opencode/`. Re-measured minimum-of-nine ratios: `hash 1.001`,
  `crc32 1.108`, `sort 1.399`, `int_map 1.184`, `str_map 0.842`,
  `random 1.016`; none exceeded 1.5×. Final gate: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`
  (**294 passed, 0 failed**: 22+6+1+102+42+43+2+6+32+23+15),
  `python3 tools/sync_examples.py --check`, and `git diff --check` all exit 0.
- **Previous task (2026-11-18):** Phase 6, final measured benchmarks and report.
  Added six paired Loi/C workloads in `benchmark/`: FNV-1a hashing, CRC-32,
  heapsort, `IntMap<u64>`, `StrMap<u64>` and xorshift64*. `benchmark/run.py`
  rebuilds both sides, verifies identical checksums, pins every run to CPU 6,
  and times nine runs per binary. Exact command: `export CC=gcc; python3
  benchmark/run.py --output benchmark/results.json`. Result: Loi/C minimum
  ratios were `hash 1.001`, `crc32 1.108`, `sort 1.399`, `int_map 1.184`,
  `str_map 0.842`, `random 1.016`; none exceeded 1.5×, so no ranked
  generated-C cause analysis was required. All six benchmark programs' emitted
  C compiled with `-std=c11 -pedantic-errors -Wall -Wextra` with empty stderr;
  all six UBSan binaries ran with matching output; all six ASan+UBSan binaries
  built but were not run for the recorded PRoot shadow-memory reason. The full
  report is `development/FINAL_REPORT.md`. Final gate: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`
  (**294 passed, 0 failed**: 22+6+1+102+42+43+2+6+32+23+15),
  `python3 tools/sync_examples.py --check`, and `git diff --check` all exit 0.
  No further standard-library phase is planned; the LLVM evaluation is recorded
  in this section.
- **Previous task (2026-11-18):** Phase 5 standard-library expansion: seeded
  randomness. Added `lib/random.loi` test-first: four new `tests/lib.rs` cases
  cover known xorshift64* vectors, `u64` / `u32` / bounded draws, state
  progression, picking, deterministic in-place Fisher–Yates shuffling, exact
  flow-naming runtime errors, seeded comparisons against a Rust reference,
  duplicate imports, and two libraries importing the module. `Rng` is one
  `u64` inout state, so all flows are pure and need no `protect`; the library is
  explicitly not cryptographically secure. Public docs were updated in
  `SINTAXE.md` and `website/docs.html`. Every test program's C was compiled with
  `-std=c11 -pedantic-errors -Wall -Wextra` (warnings treated as failure),
  UBSan-only builds were compiled and run clean, and ASan+UBSan builds were
  compiled but not run because this PRoot container cannot map ASan shadow
  memory. Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test --all-targets` (**294 passed, 0 failed**:
  22+6+1+102+42+43+2+6+32+23+15), `python3 tools/sync_examples.py --check`, and
  `git diff --check` all exit 0. Next: Phase 6, measured benchmarks.
- **Previous task (2026-11-18):** Phase 4 standard-library expansion: maps.
  Added `lib/map.loi` test-first: five new `tests/lib.rs` cases cover every
  `map_int_*` and `map_str_*` flow with runnable programs, growth from 8 to 32
  entries, composite values, exact missing-key errors, seeded comparisons
  against Rust `HashMap`, duplicate imports, and two libraries importing the
  module. `IntMap<V>` is keyed by `u64`; `StrMap<V>` stores byte keys in one
  contiguous arena buffer with `starts` / `lengths` arrays, avoiding
  slice-of-slices ownership. Mutators are `sealed` and need the caller's
  `protect`. Public docs were updated in `SINTAXE.md` and `website/docs.html`.
  Every test program's C was compiled with `-std=c11 -pedantic-errors -Wall
  -Wextra` (warnings treated as failure), UBSan-only builds were compiled and
  run clean, and ASan+UBSan builds were compiled but not run because this PRoot
  container cannot map ASan shadow memory. Gate: `cargo fmt --check`, `cargo
  clippy --all-targets -- -D warnings`, `cargo test --all-targets` (**290
  passed, 0 failed**: 22+6+1+102+42+43+2+6+28+23+15),
  `python3 tools/sync_examples.py --check`, and `git diff --check` all exit 0.
  Next: Phase 5, continued standard-library expansion.
- **Previous task (2026-11-18):** Phase 3 standard-library expansion: CRC-32.
  Added `lib/crc32.loi` test-first: three new `tests/lib.rs` cases cover known
  vectors, `crc32_update` chunking, seeded data against both a Rust bitwise
  reference and an independent C shim, duplicate imports, and two libraries
  importing the module. The 16-entry reflected nibble table is generated by
  `comptime` flows and stored as a static `[u64]` slice with two entries per
  word; no `protect` block is needed. Comptime field assignment and packing
  eight `u32` entries into a `u64` are blocked and recorded in
  `development/FRICTION.md`. Public docs were updated in `SINTAXE.md` and
  `website/docs.html`. Every test program's C was compiled with
  `-std=c11 -pedantic-errors -Wall -Wextra` (warnings treated as failure),
  UBSan-only builds were compiled and run clean, and ASan+UBSan builds were
  compiled but not run because this PRoot container cannot map ASan shadow
  memory. Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test --all-targets` (**285 passed, 0 failed**:
  22+6+1+102+42+43+2+6+23+23+15), `python3 tools/sync_examples.py --check`, and
  `git diff --check` all exit 0. Next: Phase 4, generic `IntMap<V>` and
  `StrMap<V>`.
- **Previous task (2026-11-18):** Phase 2 standard-library expansion: strings
  and hashing. Added `lib/string.loi` and `lib/hash.loi` test-first: five new
  `tests/lib.rs` cases cover every flow with runnable programs, known FNV-1a
  vectors, seeded properties against a Rust reference, duplicate imports, two
  libraries importing the new modules, and the exact `str_repeat` overflow
  error. `str_*` transforming flows are `sealed` and need the caller's
  `protect`; queries are `open`. `hash_*` flows are pure 64-bit FNV-1a and hash
  `u64` values as little-endian bytes. Loi has no character literals; the probe
  and exact diagnostic are in `development/FRICTION.md`. Public docs were
  updated in `SINTAXE.md` and `website/docs.html`. Every test program's C was
  compiled with `-std=c11 -pedantic-errors -Wall -Wextra` (warnings treated as
  failure), UBSan-only builds were compiled and run clean, and ASan+UBSan builds
  were compiled but not run because this PRoot container cannot map ASan shadow
  memory. Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test --all-targets` (**282 passed, 0 failed**:
  22+6+1+102+42+43+2+6+20+23+15), `python3 tools/sync_examples.py --check`, and
  `git diff --check` all exit 0. Next: Phase 3, CRC32 with a `comptime` table.
- **Previous task (2026-11-18):** Phase 1 standard-library expansion. Added
  `lib/vec.loi` and `lib/slice.loi` test-first: eight new `tests/lib.rs` cases
  cover every flow with runnable programs, exact flow-naming runtime errors,
  seeded sort/find properties against Rust references and C `qsort`, duplicate
  imports, and two libraries importing the new modules. `Vec<T>` stores cloned
  values; allocator flows are `sealed` and require the caller's `protect`;
  `vec_sort` is heapsort (worst-case O(n log n)). Blocked designs are recorded
  in `development/FRICTION.md`. Gate: **277 passed, 0 failed**
  (22+6+1+102+42+43+2+6+15+23+15). Committed as `2fda361`.
- **Previous task (2026-11-17):** made lexer-error JSON ranges exclusive and
  agree with human underlines. `LexError` now carries the exclusive
  `end_column`, computed with the renderer's token-length scan, and the CLI
  threads it into `lex`, `fmt` and `check` diagnostics. Parser errors keep the
  existing line-scan fallback. A new failing-first `tests/cli.rs` case pins
  `outer = [4]` at column 17 with `end_column` 20 and `x <- "abc` at column 10
  with `end_column` 14, plus matching 3- and 4-caret underlines. Gate: `cargo
  fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
  --all-targets` (**269 passed, 0 failed**: 22+6+1+102+42+43+2+6+7+23+15),
  `python3 tools/sync_examples.py --check` all exit 0.
- **Previous task (2026-11-17):** fixed trailing-comment placement in `loi fmt`.
  The pretty-printer now separates standalone and trailing comments up front,
  records the output line anchored to each source line, and appends trailing
  comments after all lines for that source line are emitted — so they follow the
  last token (`]`, `{`, `=> T`) instead of the first. Return-type lines now carry
  their type's span. Three failing-first `tests/pretty.rs` cases pin this plus
  idempotence for own-line-before-item, between-fields, after-last-field,
  inside-empty-body, EOF-without-newline and `//` in a string; a fourth runs
  `fmt` over every `examples/*.loi` and `lib/*.loi` and asserts the lexer's
  comment list is unchanged and formatting is idempotent. Gate: `cargo fmt
  --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
  --all-targets` (**268 passed, 0 failed**: 22+6+1+102+41+43+2+6+7+23+15),
  `python3 tools/sync_examples.py --check` all exit 0.
- **Previous task (2026-11-17):** turned linker failures into source diagnostics.
  The CLI now captures linker stderr, maps `undefined reference to \`SYMBOL\` /
  `undefined symbol: SYMBOL` to the matching `extern flow` in the main file or
  any import, and points at its declaration (e.g. `lib/os.loi:20:1`). The
  message names both the Loi-facing flow and the C symbol; the help line says
  `this library ships a C shim: build with --c-source lib/os_shim.c` when a
  sibling `<stem>_shim.c` exists, otherwise `pass the C file that defines it
  with --c-source FILE`. Human, short and JSON all carry the help line;
  unmapped linker output is still printed after the diagnostic. Two new
  failing-first `tests/cli.rs` cases cover both help variants and all three
  formats. No auto-linking or syntax was added; the short
  `auto-link shims: options and trade-offs` proposal is in `DECISIONS.md`.
  Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --all-targets` (**265 passed, 0 failed**:
  22+6+1+102+41+43+2+6+7+23+12), `python3 tools/sync_examples.py --check`
  all exit 0.
- **Previous task (2026-11-17):** gave every `ast::Type` an explicit span (named
  token, slice brackets, generic name through `>`), recorded it in `parse_type`,
  and used it for unknown-type, generic-arity and annotation/cast mismatch
  diagnostics. Seven new `tests/errors/type_span_*.loi` cases cover annotation,
  parameter, return, shape-field and cast positions plus arity and mismatch;
  two old expectations were wrong (they pointed at the enclosing statement or
  the initializer instead of the type) and were corrected. Gate: `cargo fmt
  --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
  --all-targets` (**263 passed, 0 failed**: 22+6+1+102+39+43+2+6+7+23+12),
  `python3 tools/sync_examples.py --check` all exit 0.
- **Previous task (2026-11-17):** expanded the explicit library into modular
  files. `lib/math.loi` provides generic `math_min/max/clamp/sum<T>`, a
  `comptime math_factorial`, constants and float/integer helpers;
  `lib/ctype.loi` provides pure ASCII byte classification and decimal parsing.
  `lib/os.loi` and `lib/net.loi` are thin POSIX bindings with `lib/os_shim.c`
  and `lib/net_shim.c`: process/environment queries and a minimal TCP client
  with IPv4 helpers. No new intrinsics or language rules were added. Docs,
  playground library copying/linking and 7 new `tests/lib.rs` cases are in
  place; the existing `lib/std.loi` API is unchanged. Verified: `cargo fmt
  --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test
  --all-targets` (**263 passed, 0 failed**:
  22+6+1+102+39+43+2+6+7+23+12), `python3 tools/sync_examples.py --check`,
  `git diff --check`, `node --check website/server.mjs`, `cc -std=c11 -Wall
  -Wextra -fsyntax-only` on both shims and `loi check` on every library file
  all exit 0. A live local playground `/api/run` request importing `math.loi`
  and `os.loi` returned `ok:true` with the expected output.
- **Previous task (2026-11-17):** added `examples/audio_visualizer.loi`, a Loi
  adaptation of the C3 playground's "Audio Visualizer". It keeps the original
  110 BPM, 16-step sequencer and procedural kick/snare/bass/lead/solo state
  machine, then drives reactive bars, circles and a waveform. `examples/sound_shim.c`
  gained `loi_audio_feed`: native builds feed a raylib `AudioStream` with
  `UpdateAudioStream`; the browser queues back-to-back `AudioBufferSourceNode`
  buffers. No keyboard key starts the audio on either platform. The site now has
  a third preset/tab/iframe, and the docs cover both audio examples. Rebuilding
  both audio demos with Emscripten 7.0.0 exits 0; no browser was available
  locally, so audible output was not confirmed firsthand and native raylib link
  still needs a usable `libraylib.a`.
- **Previous task (2026-11-17):** added `printf` variadic format support
  and made the raylib audio demo auto-play. `printf` is a reserved compiler
  intrinsic: its first argument must be a string literal; the checker validates
  conversion count and argument kinds, rejects unknown conversions and dynamic
  `*` width/precision, and the C backend casts each vararg to the exact C type
  its conversion reads. A new `src/format.rs` parser is shared by both phases;
  `printf` is rejected in `comptime` like other I/O.
  `examples/sound_synth.loi` now queries the new `loi_audio_ready` shim and
  starts on the first ready frame (native: immediately after
  `InitAudioDevice`; web: as soon as the browser reports a running
  `AudioContext`). The generated wrapper tries to unlock WebAudio at load and
  again on the first click/tap, so no keyboard key is needed. Documentation,
  examples and the WebAssembly demos were updated.
- **Previous task (2026-11-17):** made the embedded WebAssembly view
  actually interactive, added the audio demo and made its sound play.
  `website/playground.html` now has two tabs (bolas / síntese de som) that
  switch iframes; selecting a matching preset switches the tab too.
  `examples/build_web.sh` accepts `WEB_C_SOURCES`, so `sound_synth.loi` builds
  for the web with `examples/sound_shim.c`. On the web the shim bypasses
  raylib/miniaudio — whose WebAudio backend needs AudioWorklets, Wasm workers
  and COOP/COEP headers — and copies the generated `Float32Array` straight into
  an `AudioBuffer`, started with a `BufferSourceNode`. This uses `EM_ASM`, so
  the build passes `-std=gnu11` (the CLI defaults to `-std=c11`, where
  Emscripten disables `EM_ASM`). Native builds still use raylib's `Sound`.
  `examples/raylib_web.loi` shows the tone name, a `PAUSADO` marker and a
  background color that follows the arrow keys, so input is visible.
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
| Library | Working | `lib/std.loi` (scalar/string helpers), `lib/math.loi` (generic and `comptime`), `lib/ctype.loi` (pure ASCII helpers), `lib/os.loi`/`lib/net.loi` (thin POSIX bindings with C shims); prefixed names, explicitly imported, 7 integration tests |
| Binding generator | Working | `tools/bindgen/bindgen.py` emits `opaque` handles, safe `shape`s, typed constants, `extern flow`s, a `memcpy` shim with `_Static_assert` checks and a skip report |
| Vendored bindings | Working (syntax-verified) | `vendor/sdl2/` (SDL2 2.32.10) and `vendor/raylib/` (raylib 5.5), each with generated `.loi`, shim, skip report and README |
| IR | Draft | `src/ir.rs`; `--emit-ir` writes text. Globals and some literals are not lowered |
| Assembly | Disabled draft | `src/asm.rs` cannot generate runnable code; the CLI rejects `--emit-asm` with exit 2 |
| CLI | Working | `lex`/`parse`/`fmt`/`check`/`gen`/`build`/`clean`, incremental stamps, profiles, source maps, `--arena-size`, `--c-source`, `--error-format`, `--color` |
| Website / playground | Working (local only) | Static site plus `website/server.mjs` driving the real compiler with `--error-format=short --color=never`; `127.0.0.1` by default with source/output/time limits |

## Verification

Last verified 2026-11-18 after the canonical example expansion with:

```sh
export PATH=/root/.cargo/bin:$PATH
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
python3 tools/sync_examples.py --check
git diff --check
node --check website/server.mjs
for f in lib/os_shim.c lib/net_shim.c; do cc -std=c11 -Wall -Wextra -fsyntax-only "$f"; done
for f in lib/std.loi lib/math.loi lib/ctype.loi lib/os.loi lib/net.loi; do ./target/debug/loi check "$f"; done
```

- `cargo fmt --check`: exit 0.
- `cargo clippy --all-targets -- -D warnings`: exit 0.
- `cargo test --all-targets`: **320 passed, 0 failed** (22+6+1+113+42+51+2+8+32+26+17 across all targets; compile-and-run tests run when `cc` is available).
- `python3 tools/sync_examples.py --check`: `website/examples` is in sync.
- `examples/marks.loi` and `examples/choices.loi`: `loi check` passed; built
  executables printed `marks ok` and `choices ok` and exited 0. Their generated
  C passed strict C11 and ASan+UBSan compile-only checks in `tests/codegen.rs`.
- `git diff --check`, `node --check website/server.mjs`, both strict-C11 shim checks and `loi check` on every library file: exit 0.
- `tests/lib.rs` builds and runs programs that import every module, including both shims, and checks that generic/comptime math, ASCII parsing, environment reads and IPv4/TCP helpers behave as documented. A local playground `/api/run` request importing `math.loi` and `os.loi` returned `ok:true` with `playground libs ok`.
- `node --check website/web/*.js`: exit 0 for both generated demos;
  `loi check examples/sound_synth.loi`: exit 0; `gcc -std=c11 -Wall -Wextra
  -fsyntax-only` on the generated synth C: exit 0 (warnings only).
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
- I/O intrinsics are limited to `print`, `eprint`, `panic`, numeric/`printf`
  formatting and whole-file / whole-stdin reads and writes. `printf` needs a
  literal format string and rejects dynamic `*` width/precision; user-declared
  variadic flows are not supported. `read_file`, `read_stdin` and `read_line`
  allocate in the current `protect` region and cannot escape it. There is no
  directory, seek or process API in the intrinsic set. `lib/os.loi` and
  `lib/net.loi` wrap a few POSIX calls through shims (process/environment
  queries and a minimal TCP client); those shims are POSIX-only, expose sockets
  as plain integer descriptors and have no `guard`/release discipline.
  Resources (`guard`) are specified but not implemented.
- Library limits: `lib/ctype.loi` classifies ASCII bytes only (Loi strings are
  UTF-8 bytes), and `math_factorial`/`math_pow_u64` follow unsigned Loi
  arithmetic, so large results wrap at 64 bits.
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
- Decide whether callbacks, user-declared variadics, arbitrary C unions or fixed
  arrays deserve first-class support or remain shim-only. Loi `choice` already
  covers tagged unions with safe payloads (`printf` is already a variadic
  intrinsic).
- Define the byte-slice/`cstr` conversion surface beyond string literals.
- Extend escape analysis beyond lexical blocks if programs need block-local
  slices to outlive their block without `alloc`.
- Revisit `<-` composite passing if concurrency or observable aliasing is ever
  introduced; today the C backend passes by value and the checker enforces
  read-only use.
- Finish or remove the IR/ASM drafts. Until then `--emit-ir` stays diagnostic
  and `--emit-asm` stays rejected.
- Adopt the opt-in LLVM path evaluated above, or stay C-only? If adopted, decide
  the staged CLI surface (`--emit-llvm` before any `--backend llvm`), the
  stdout/stderr/exit parity corpus and whether the C backend remains the
  permanent semantic oracle. No language syntax is involved.

## Next Steps

1. Add a real module/namespace design and enforce `open`/`sealed` across files.
2. Design linear resources (`guard`, `<Shape>_release`) and apply them to the
   socket descriptors now exposed by `lib/net.loi`; the shim alone does not
   manage lifetimes.
3. Design growable collections (`Vec<T>`, maps) as library code now that
   generics and explicit monomorphization exist.
4. Extend the binding generator for the C constructs that still require
   hand-written shims, and add window/render examples for SDL2 and raylib on a
   machine with the runtimes installed.
5. Evaluate the opt-in LLVM lowering from the new Open Design Question: start
   with scalar IR plus exported C runtime wrappers, keep the C backend default,
   and require parity on the existing runnable test corpus before any build-mode
   switch.
6. Finish IR/ASM or remove them from the published CLI surface.
7. Keep the docs in sync: `SINTAXE.md` is canonical, `website/docs.html`
   mirrors it, and `python3 tools/sync_examples.py --check` guards the example
   mirror.
