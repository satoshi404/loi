# Loi Compiler — Implementation Progress

## Current Status
- Phase: 4 — C backend **in progress** (generates C, `loi gen -o` writes it, `loi build` compiles and links a runnable executable through `cc`)
- Last completed task: Completed the **cleanup** phase of the Flow Syntax migration and added the reference **website** (`website/`) with full documentation and a local playground backed by the real compiler. The legacy keywords and forms were removed; `lib/std.loi`, every example and every parseable test source now use canonical Flow Syntax only. The reference files `examples/particles.loi` and `examples/flow_syntax.loi` are byte-identical canonical programs. `fmt` is idempotent on all canonical files and `loi check` accepts them. `examples/tictactoe.loi` adds a complete deterministic self-play game.
- Overall health: Good
- Remaining in Phase 4: enforce cross-file visibility, support composite globals through a runtime constructor, finish the IR/ASM backends, and continue allocator/source-map hardening. Allocator failure policy, source maps, header generation, intrinsic I/O/math, literal-range checking, range-loop edge tests, negative-index diagnostics, integer constant folding, runtime source positions, shape cycle detection, multi-file builds, `while true` definite return, `break`/`continue`, hex/bin literals, arena failure policy, canonical diagnostics and the website/playground are done.
- Phase 3 `comptime` is no longer blocked by design: 5.1 defines it as a compile-time guarantee. Implementation still needs a constant evaluator and parser support.
- Pointers are decided: Loi has none (5.2). The partial implementation stays reverted; graphs use handles (5.5) and resources use linear close (5.6).
- Flow Syntax (5.10) is complete: canonical Flow Syntax is the only accepted surface, `fmt` emits it, and the legacy keywords, brace literals, `[]T{}` slice literals and the `&` token are gone.

## Component Status
| Component       | Status            | Notes |
|-----------------|-------------------|-------|
| Project structure | Done            | Cargo package `loi`, library + CLI, source modules separated by stage; `lib/std.loi` now ships with the project |
| Lexer           | Working (v0.5)    | Canonical Flow keywords (`shape`, `flow`, `open`, `sealed`, `mut`, `give`, `clone`, `protect`, `alloc`, `in`), identifiers, decimal/hex (`0x`)/binary (`0b`) integer/float literals, double-quoted strings with decoded escapes (`\n`, `\t`, `\r`, `\0`, `\\`, `\"`, ASCII `\xNN`), comments, longest-match operators including `<->`, `<-`, `=>`, newlines, EOF, source positions; legacy keywords and `&` are rejected |
| AST             | Working (v0.5)    | Declarations, statements and expressions carry start spans; shapes, defaults, flows, `give`/`clone`/`alloc`, imports, globals, string literals and slice literals. Canonical surface only; `give`/`clone` lower to the internal `Move`/`Copy` variants |
| Parser          | Working (v0.5)    | Recursive descent; canonical `examples/particles.loi` and `examples/flow_syntax.loi`, top-level `import` plus flow/global bindings, inline and vertical `flow` signatures, `[T]` types, `[a, b]` / `[T: a, b]` / `[T:]` slice literals, bracket shape literals and `=>` returns; no legacy forms |
| Pretty-printer  | Working (v0.5)    | Emits canonical Flow Syntax with minimal parentheses: `shape [ ... ]`, `flow` vertical clauses, `<-`/`->`/`<->`, `=>`, `mut`, `give`/`clone`, `open`/`sealed`, `[T]` and `Name[ ... ]`; both reference files format to the same text |
| Symbol table    | Working (v0.1)    | Two-pass declarations; nested local scopes plus a base global scope; parameters, loop variables and mutable/initialized state |
| Semantic types  | Working (v0.1)    | Primitives, shapes, slices, unsuffixed literals, `void`, error recovery; coercion helpers |
| Type Checker    | Working (v0.9)    | Shapes/defaults/literals, flows, directions, calls, statements, expressions, casts, `give`/`clone`/`alloc`, `for`, `protect`, integer-literal range checks with constant folding, constant division-by-zero errors, negative constant slice indices, shape value-cycle detection, reserved builtin intrinsics (I/O/math/`len`), `while true` definite return, globals (constant initializers, scalar/`[u8]` only), string/slice literal typing, `len`/`.len`, scalar and byte-slice equality, disjoint constant-index ownership paths; canonical glyph diagnostics |
| Ownership / memory checks | Done (v1.1) | Path-sensitive give liveness, branch/loop definite assignment, definite return, `->`/`<->` exclusivity (constant `xs[0]`/`xs[1]` are disjoint), cross-function parameter consumption, taint propagation through consumed `<->` arguments, no implicit composite copies, scalar implicit copy, borrowed-composite return rejection, `protect`/`alloc` region escape checks and rejection of returning a slice literal |
| Code Generator  | Working (v0.8)    | Phase 4: one C11 translation unit, shape layout, prototypes, functions, expressions/statements, `protect` bump arena, runtime bounds checks, `->` defaults, sequenced `alloc`, tested range-loop edges, C header generation (`--emit-h`), `open`/`sealed` visibility in codegen, multi-file object compilation with merged checking and `generate_c_subset`, source maps (`--source-map`), static string arrays, C globals, slice-literal compound literals, `loi_slice_eq`, and I/O/math/`len` intrinsics |
| IR              | Working (draft)   | `src/ir.rs`: three-address IR types/instructions/blocks/functions, `IrBuilder`, and `program_to_ir_module`; emitted as text with `--emit-ir`. String/slice literals and globals are not lowered yet |
| Assembly backend| Working (draft)   | `src/asm.rs`: lowers IR to x86-64 AT&T assembly (`--emit-asm`); covers common integer, memory, call and control-flow instructions with simplified lowerings |
| CLI             | Working (v0.16)   | `loi lex|parse|fmt|check FILE`, `loi gen FILE [-o out.c] [--emit-h header.h] [--emit-ir ir] [--emit-asm asm] [--source-map map.json]`, `loi build FILE... [-o out] [--emit exe|obj|lib] [--debug|--release] [--rebuild] [--emit-c c] [--emit-h header.h] [--emit-ir ir] [--emit-asm asm] [--arena-size BYTES] [--source-map map.json] [--cc-flag F]...` (defaults to `./a.out`, `<stem>.o` or `lib<stem>.a`; accepts multiple input files; recursively loads `import`s; checks the merged program once and compiles one translation unit per explicit input; invokes `cc`/`$CC` and `ar`/`$AR`, whose values may contain shell-like arguments; skips unchanged builds), `loi clean FILE [-o out]` (also removes retained `--emit-c` files), `--help`/`--version` |
| Website         | Working (v1.0)    | `website/` contains the landing page, complete documentation, 404 page, shared CSS/JS and mirrored `examples/`. Strict static assets with no build step and no npm dependencies. |
| Playground      | Working (v1.0)    | `website/server.mjs` serves the site and drives the real `loi` binary through `POST /api/check`, `/api/format` and `/api/run`, plus `GET /api/health`. Programs run in temporary workspaces with the standard library copied in; local-only (`127.0.0.1`), bounded source/output/time and four concurrent jobs. |

## Verification
Last verified 2026-11-08.

- Command: `cargo test --all-targets`
- Result: `165 passed; 0 failed` (9 library unit tests, 6 CLI-binary unit tests, 71 checker integration tests, 31 CLI integration tests, 22 codegen integration tests, 5 lexer integration tests, 14 parser integration tests, 7 pretty-printer integration tests)
- Command: `cargo fmt --check`
- Result: pass
- Command: `cargo clippy --all-targets -- -D warnings`
- Result: pass
- Command: `./target/debug/loi check examples/particles.loi`
- Result: exit 0; the reference program type-checks after the checker repair
- Command: `./target/debug/loi gen examples/particles.loi --emit-ir /tmp/opencode/particles.ll --emit-asm /tmp/opencode/particles.s -o /tmp/opencode/particles.c`
- Result: exit 0; writes C (207 lines), IR (131 lines) and x86-64 assembly (81 lines)
- Command: `cargo run --quiet -- build examples/math_demo.loi -o /tmp/opencode/math_demo && /tmp/opencode/math_demo`
- Result: exit 0; prints "Sqrt 5.0"; exercises `sqrt`, `print`, `alloc`, `protect` and `[u8]` indexing
- Command: `cargo run --quiet -- build examples/iteration.loi -o /tmp/opencode/iteration && /tmp/opencode/iteration`
- Result: exit 0; exercises `for <- p in` (read) and `for <-> p in` (mutate) over slices, `alloc`, `protect`, shape literals
- Command: `cargo run --quiet -- build examples/geometry.loi -o /tmp/opencode/geometry && /tmp/opencode/geometry`
- Result: exit 0; exercises shapes with `i32` fields, range loops with `u32` bounds, `alloc` of composite slices, flow calls with `<-`/`->`
- Command: `cargo run --quiet -- check examples/cycle_interface.loi` (rejected)
- Result: exit 1; `shape 'B' contains itself by value through a cycle: B -> A -> B`
- Command: `cargo run --quiet -- check examples/negative_index_error.loi` (rejected)
- Result: exit 1; `slice index cannot be negative`
- Command: `cargo run --quiet -- check examples/literal_range_error.loi` (rejected)
- Result: exit 1; `integer constant is out of range for u8`
- Command: `cargo run --quiet -- check examples/constant_folding_error.loi` (rejected)
- Result: exit 1; `integer constant is out of range for u8` (from `200 + 100`)
- Command: `cargo run --quiet -- parse examples/particles.loi`
- Result: exit 0; prints the complete AST (3 shapes, 6 flows)
- Command: `cargo run --quiet -- fmt examples/particles.loi > /tmp/particles.formatted.loi`
- Result: exit 0; formatted source parses successfully
- Command: `cargo run --quiet -- parse /tmp/particles.formatted.loi`
- Result: exit 0
- Command: `cargo run --quiet -- check examples/flow_syntax.loi`
- Result: exit 0; the canonical Flow Syntax reference type-checks
- Command: `cargo run --quiet -- build examples/flow_syntax.loi -o /tmp/opencode/flow_demo && /tmp/opencode/flow_demo`
- Result: exit 0; builds and runs the Flow Syntax reference
- Command: `cargo run --quiet -- fmt examples/particles.loi > /tmp/opencode/particles_flow.loi && cargo run --quiet -- check /tmp/opencode/particles_flow.loi && cargo run --quiet -- build /tmp/opencode/particles_flow.loi -o /tmp/opencode/particles_flow && /tmp/opencode/particles_flow`
- Result: exit 0; the legacy reference is converted to canonical Flow Syntax and still checks, builds and runs
- Command: `cargo run --quiet -- fmt examples/flow_syntax.loi` twice and compare
- Result: byte-identical; formatting is idempotent
- Command: `cargo run --quiet -- check lib/std.loi`
- Result: exit 0; the canonical standard library type-checks
- Command: `cargo run --quiet -- check` on `examples/flow_syntax.loi`, `geometry.loi`, `iteration.loi`, `language_basics.loi` and `math_demo.loi`
- Result: exit 0; every canonical example type-checks
- Command: `loi fmt` twice and compare on `lib/std.loi` and the canonical examples
- Result: byte-identical for every file; the ported sources are format-stable
- Command: `cargo run --quiet -- check examples/particles.loi`
- Result: exit 0; the retained legacy-surface reference still checks
- Command: `diff <(loi fmt examples/particles.loi) <(loi fmt examples/flow_syntax.loi)`
- Result: no differences; both references describe the same program canonically
- Command: `cargo run --quiet -- gen examples/particles.loi > /tmp/particles.c`
- Result: exit 0; 186 lines of C11
- Command: `cc -std=c11 -Wall -Wextra -o /tmp/particles /tmp/particles.c && /tmp/particles`
- Result: compiles, runs and exits 0; the only warning is the reference program's own unused `mass` local (`-Wunused-but-set-variable`)
- Command: `cargo run --quiet -- gen examples/particles.loi -o /tmp/particles.c`
- Result: exit 0; writes the C file and prints nothing to stdout
- Command: `cargo run --quiet -- build examples/particles.loi -o /tmp/particles-built && /tmp/particles-built`
- Result: exit 0; `build` generates temporary C, invokes `cc -std=c11`, links the executable and removes the temporary file
- Command: `cargo run --quiet -- --help` / `cargo run --quiet -- --version`
- Result: both exit 0; help prints the usage and options, version prints `loi 0.1.0`
- Command: `cargo run --quiet -- build examples/particles.loi -o /tmp/particles-built --emit-c /tmp/particles-retained.c`
- Result: exit 0; builds the executable and retains the generated C at the requested path
- Command: `cargo run --quiet -- build demo.loi --emit lib` (in a scratch directory)
- Result: exit 0; writes `libdemo.a`, which links with `cc libdemo.a -o demo` and runs with the expected exit code
- Command: `cargo run --quiet -- build examples/particles.loi -o /tmp/particles-debug --debug`
- Result: exit 0; passes `-g` to the C compiler
- Command: `cargo run --quiet -- build examples/particles.loi -o /tmp/particles-release --release`
- Result: exit 0; passes `-O2` to the C compiler
- Command: `loi build` twice on unchanged source through a `CC` wrapper that logs each invocation, then once with `--rebuild` (covered by `build_skips_unchanged_outputs`)
- Result: the first run invokes `cc`, the second is skipped, `--rebuild` invokes `cc` again, and editing the source invokes `cc` once more
- Command: `loi clean examples/particles.loi` after build/obj/lib builds (covered by `clean_removes_default_artifacts`)
- Result: exit 0; removes `a.out`, `<stem>.o` and `lib<stem>.a` with their stamps, preserving the source
- Command: `loi clean <file> -o <custom-output>` with invalid source (covered by `clean_removes_an_explicit_output_without_reading_the_source`)
- Result: exit 0; removes the output and stamp without parsing the source; `-o` equal to the input exits 2
- Command: `loi build demo.loi --emit-c retained.c && loi clean demo.loi` (covered by `clean_removes_retained_generated_c` and the manual check below)
- Result: exit 0; the stamp records `emit-c=retained.c`, and cleaning removes `a.out`, its stamp and `retained.c` while preserving the source
- Command: build twice with different `--emit-c` paths and a pre-existing new path (covered by `changing_emit_c_path_forces_rebuild`)
- Result: the second build recompiles and regenerates the new C file instead of trusting a stamp that names the old path
- Command: `CC='"/path/with space/cc wrapper.sh" -DLOI_CC_ARG=1' loi build ...` and `AR='"..." --loi-test' loi build --emit lib` (covered by `tool_environment_values_may_contain_arguments`)
- Result: exit 0; quoted tool paths and their extra arguments are forwarded, including across spaces
- Command: `CC='cc -O1' loi build demo.loi` and `CC='' loi build demo.loi`
- Result: the first builds and runs with exit 3; the empty value is rejected with exit 2
- Command: `cargo test --test cli`
- Result: `22 passed; 0 failed`; `build` tests are skipped when `cc` or `ar` is unavailable
- Command: `cargo test --test codegen alloc_sequences_target_count_and_store`
- Result: pass; generated C captures the target address, evaluates the count into a `size_t` local, then stores the allocation through the pointer
- Command: `cargo test --test codegen`
- Result: `11 passed; 0 failed`; the compile-and-run tests `#include` the generated C in a C harness and are skipped when `cc` is unavailable
- Command: `cargo test --test codegen compiled_range_loop_edges`
- Result: pass; signed `-3..3` sums to `-3`, descending and equal bounds run zero times, unsigned `0..5` sums to 10, and ranges ending at `UINT64_MAX` iterate without wrapping the induction variable
- Command: `loi gen` on `for i in -3..3`
- Result: the emitted loop uses `int64_t` for the end temporary and induction variable, and `-3` is preserved through the cast
- Command: `loi check` on constant slice indices (covered by `rejects_negative_constant_slice_indexes`)
- Result: `xs[-1]` and an assignment to `xs[-42]` report `slice index cannot be negative` at the index span; `xs[0 - 1]` is now folded and rejected too; `xs[-0]` and `xs[--1]` are accepted, and a variable index still type-checks
- Command: `loi check` on folded integer constants (covered by `folds_constant_expressions_for_range_checks`)
- Result: `200 + 100`, `512 / 2`, `16 * 16`, `0 - 1`, `-128 - 1`, `(100 + 100) * 2` and `18446744073709551615 + 1` are rejected with `integer constant is out of range for ...`; `127 + 128` in `u8`, `-64 - 64` in `i8` and `1000 % 256` in `u8` pass
- Command: `loi check` on constant zero divisors (covered by `rejects_constant_division_by_zero`)
- Result: `1 / 0`, `x / (2 - 2)` and `x % (1 - 1)` report `division by zero` or `remainder by zero` at the divisor
- Command: `loi check` on boundary and over-range literals (covered by `accepts_boundary_integer_literals`, `rejects_integer_literals_out_of_range`, `rejects_negative_range_start_with_unsigned_bound`)
- Result: `255`/`u8`, `-128`/`i8`, `18446744073709551615`/`u64` and `-2147483648`/`i32` pass; `256`, `-129`, `300` for `u8`, `70000` for `u16`, an inferred `5000000000`, a 129-bit literal, `u64 > -1` and `u8 + 300` are rejected with source spans
- Command: `loi build` on a program comparing a `u64` local with `18446744073709551615`, then run it
- Result: exit 0; the C literal carries an unsigned suffix and the comparison is correct
- Command: `cargo run --quiet -- build examples/language_basics.loi -o /tmp/opencode/language_basics && /tmp/opencode/language_basics`
- Result: exit 0; prints `Loi language basics` and `language basics ok`; exercises an import, string/scalar globals, `[i32]`/inferred slice literals, `len`/`.len`, `print` and `lib/std.loi` string helpers
- Command: `loi build /tmp/opencode/usemain.loi` where `usemain.loi` imports `add.loi` and returns `add(2, 3)`
- Result: exit 5; the driver recursively loaded the imported file
- Command: `loi build /tmp/opencode/crossmain.loi /tmp/opencode/crossadd.loi` where `main` calls `add` from the second file
- Result: exit 9; the merged checker accepted the cross-file call and `generate_c_subset` produced one object per input (`crossmain.o`, `crossadd.o`)
- Command: `loi build /tmp/opencode/stdprobe.loi` importing `lib/std.loi`
- Result: exit 0; `min_i32`, `max_i32`, `abs_i32`, `clamp_i32`, `abs_f64`, `str_eq`, `str_starts_with`, `str_find` and `str_len` behave as expected
- Command: `./target/debug/loi check examples/tictactoe.loi && ./target/debug/loi build examples/tictactoe.loi -o /tmp/opencode/tictactoe && /tmp/opencode/tictactoe`
- Result: exit 0; the checker, build and run all succeed. The output contains the self-play board and ends with `X wins!` / `Result: X wins` (covered further by `tictactoe_example_type_checks` and `tictactoe_example_compiles_and_runs`)
- Command: `node website/server.mjs` then `curl http://127.0.0.1:4173/api/health`
- Result: `{"ok":true,"compiler":"loi 0.1.0"}`; `/`, `/docs.html`, `/playground.html` and the mirrored examples return 200, unknown paths return the 404 page, and path traversal is blocked
- Command: `POST /api/check`, `/api/format` and `/api/run` with valid, invalid, standard-library-importing and tic-tac-toe sources
- Result: valid programs check and run, invalid programs return checker diagnostics with exit code 1, formatting returns canonical source, `import "lib/std.loi"` and `import "../lib/std.loi"` both resolve, a program exiting 3 is reported as `ok:false` with `exitCode:3`, and empty source returns HTTP 400
- Command: `loi check` on every code snippet embedded in `website/docs.html`
- Result: exit 0 for the hello, directions, globals, regions and standard-library snippets; the inline signature `flow f <- x: i32 => i32 { => x }` also checks
- Command: `diff -r examples website/examples`
- Result: no differences; the website example mirror is in sync

Build note: the Rust target is `aarch64-unknown-linux-gnu`, but this shell is a PRoot Ubuntu userspace whose `PATH` starts with Termux tools. Cargo therefore picks the Termux `cc` (Android clang), which fails to link (`-lgcc_s`, then glibc symbols). Install the host toolchain with `apt-get install gcc libc6-dev` and build/test with `RUSTFLAGS="-C linker=/usr/bin/gcc"`; no project configuration change is needed.

## Recent Changes
- 2026-11-08: Added the **Loi website** in `website/`: landing page, complete documentation, playground, 404 page, shared CSS/JS assets, logo/favicon, mirrored `examples/` and a dependency-free Node server. `website/server.mjs` serves the static site and drives the real `loi` binary through `/api/check`, `/api/format` and `/api/run`, so the playground compiles and runs native programs without WASM. `website/README.md` documents the pages, run instructions, API shape, limits and local-only security posture.
- 2026-11-08: Added `examples/tictactoe.loi`, a complete deterministic tic-tac-toe program. Because Loi has no stdin or file API, it plays a heuristic self-play game (win/block/center/corner) and then a fixed scripted game where X completes the top row; output ends with `X wins!`. Added `tictactoe_example_type_checks` and `tictactoe_example_compiles_and_runs`, and `examples/particles.loi` remains byte-identical to `examples/flow_syntax.loi`.
- 2026-11-08: Canonicalized the remaining user-facing diagnostics: `shape` replaces `interface`, slices print as `[T]`/`[u8]`, and direction/ownership messages use `<-`/`->`/`<->`, `give` and `clone`. Updated the corresponding checker/codegen tests. The internal AST variants and source-map `"kind":"interface"` remain internal artifacts.
- 2026-11-08: Completed the **cleanup** phase of the Flow Syntax migration. Removed the legacy token kinds and keywords (`interface`, `fun`, `visible`, `hidden`, `let`, `var`, `out`, `inout`, `move`, `copy`, `return`), the `&` token, brace shape bodies/literals, the legacy `[]T{}` slice-literal path and the legacy `let`/`var` global/parser machinery. The parser now accepts only `shape`/`flow`, `open`/`sealed`, `[T]`, `Name[ ... ]`, `<-`/`->`/`<->`, `=>`, `mut`, `give`/`clone`; `in` remains only as the `for`/range separator. Converted `examples/particles.loi` to be byte-identical to `examples/flow_syntax.loi`, updated the lexer token-count test to 642, converted the remaining test fixtures, and made direction/ownership diagnostics use canonical glyphs and verbs. Updated spec sections 3, 4, 5, 6 and 7 to canonical-only. All 165 tests pass; `cargo fmt --check` and clippy are clean with `RUSTFLAGS="-C linker=/usr/bin/gcc"`.
- 2026-11-08: Completed the **transitive** Flow Syntax migration. Ported `lib/std.loi`, `examples/geometry.loi`, `iteration.loi`, `language_basics.loi`, `math_demo.loi`, the error examples and `cycle_interface.loi` to canonical Flow Syntax, and rewrote the parseable Loi source strings in the checker/codegen/CLI/parser test suites. Added a canonical-input pretty-printer round-trip test; all tests passed and `fmt` is idempotent on every canonical file.
- 2026-11-08: Adopted and implemented **Loi Flow Syntax** additively. New tokens/keywords: `<-`, `<->`, `=>`, `shape`, `flow`, `open`, `sealed`, `mut`, `give`, `clone`. The parser accepts bracket shape bodies/literals, `[T]` types, `[T: a, b]`/`[T:]` slice literals, inline and vertical `flow` signatures, `=>` returns, `alloc(-> target, T, n)` and bracket literals in `if`/`while`/`for` headers without parentheses. `fmt` now emits the canonical Flow Syntax; legacy `fun`/`interface`/`let`/`var`/`in`/`out`/`inout`/`move`/`copy`/`visible`/`hidden`/`return`/`[]T`/`Name{...}` forms still parse and check. Added `examples/flow_syntax.loi`, a byte-identical canonical mirror of `particles.loi`, plus lexer/parser/pretty tests.
- 2026-11-08: Resolved all nine former open design questions in `development/LOI_SPEC.md` section 5. Decisions: `comptime` is a compile-time guarantee (5.1); no pointers/`nil`, graphs use handles (5.2, 5.5); `hidden` is the default visibility (5.3); long-lived data uses lexical `protect`, no named regions (5.4); non-memory resources use linear explicit release with future `guard` sugar (5.6); scalar implicit copy and rejection of slice-containing `copy` are confirmed (5.7); `.loi` is final (5.8); intrinsics plus explicit `lib/std.loi` are final (5.9). Recorded the "Loi Flow Syntax" surface in 5.10 (arrows for directions, brackets for data, braces for scopes); it was subsequently adopted additively on the same day.
- 2026-11-08: Made intrinsic names reserved in the checker; `len` can no longer be shadowed by user code, matching the other intrinsics and the 5.9 decision. Added a checker test.
- 2026-11-05: Removed the Python prototype and restarted the compiler in Rust with Cargo; decision and reason recorded in `DECISIONS.md`.
- 2026-11-05: Implemented `src/token.rs` and `src/lexer.rs` from `LOI_SPEC.md` section 7, with 9 lexer tests.
- 2026-11-05: Implemented `src/ast.rs` and `src/parser.rs` (recursive descent, newline separators) and 11 parser tests.
- 2026-11-05: Added `src/pretty.rs` and `loi fmt`; the reference program and four focused cases round-trip through parse → format → parse.
- 2026-11-05: Phase 0 complete (project structure, lexer, parser/AST, pretty-printer/AST dump).
- 2026-11-05: Added `Span { line, column }` to statements, expressions, declarations, fields and parameters; parser and pretty-printer updated.
- 2026-11-05: Added `src/types.rs` with `Ty`, literal coercion, `assignable` and `common_numeric`.
- 2026-11-05: Added `src/checker.rs` with a three-pass checker and 15 integration tests; `loi check FILE` prints `path:line:column: message` diagnostics and exits 1 on errors.
- 2026-11-05: Phase 1 type-system core is complete.
- 2026-11-05: Phase 2 core ownership checks: path-sensitive `move` liveness (whole variables and fields/elements), reachability-aware branch and loop state merging for definite assignment, exclusive `out`/`inout` call arguments (with disjoint fields allowed), rejection of implicit composite copies, and rejection of returning composite values borrowed from `in` parameters. Added 11 checker tests.
- 2026-11-05: Phase 2 `protect`/`alloc` analysis: region IDs with parent links, allocation taints per ownership path, escape checks for `alloc`, `let`, assignment, `out` arguments and `return`, a function allocation-effect prepass that permits callees to allocate under the caller's `protect`, and rejection of entry functions (including `main`) that allocate without a region. Added 8 checker tests.
- 2026-11-05: Phase 2 definite-return analysis: a non-void function whose body can fall through is rejected, using the existing reachability tracking (branches, `return` and unreachable tails). Added 3 checker tests.
- 2026-11-05: Phase 2 cross-function ownership: a prepass computes which parameters each function consumes (direct `move`, returned composite places, or calls that consume them), and call sites mark consumed `out`/`inout` arguments as moved. Added 5 checker tests.
- 2026-11-05: Phase 2 allocation-taint propagation: protect regions carried by consumed `inout` arguments flow into slice-containing `out` arguments and composite call results, with escape checks at the destination. Added 2 checker tests.
- 2026-11-05: Phase 2 entry-function heuristic: uncalled `hidden` functions are no longer treated as entry points, because they are only reachable through Loi code and may rely on a caller's `protect`; `main` and uncalled visible/default functions remain entries. Added 3 checker tests.
- 2026-11-05: Phase 2 front-end review complete. Confirmed scalar implicit copy, duplicate `in` arguments, rejection of `copy` for slice-containing values, ownership transfer on composite return, rejection of returning values borrowed from `in`, and caller-owned allocations in callees. Added a scalar-copy test. Runtime slice bounds checks are deferred to codegen.
- 2026-11-05: Phase 4 C backend draft: added `src/codegen.rs` (one C11 translation unit, interface layout ordering, prototypes/functions, statements, expressions, `protect` bump arena, `loi_index` bounds checks, `out` interface defaults) and `loi gen FILE`.
- 2026-11-05: Added eight codegen integration tests: generated-text checks, interface ordering/cycle rejection, `main` return lowering, unsupported `main` signature rejection, and three compile-and-run tests under `cc` covering slices, `out`/`inout`, defaults, control flow, calls, `protect` restoration and return after a `protect`.
- 2026-11-05: Verified the reference program end to end: `loi gen examples/particles.loi` produces C11 that compiles with `cc -std=c11 -Wall -Wextra` and runs to exit 0.
- 2026-11-05: Codegen hardening: `return` with no value in `main` emits `return 0;`, return values are evaluated into a temporary before `protect` restores the arena, `loi_index` takes the slice header by value (single evaluation of the base), and generated helpers use `static inline` so unused-helper warnings do not break `-Werror` builds.
- 2026-11-05: CLI pipeline: `loi gen FILE -o OUTPUT` writes the generated C, and `loi build FILE [-o EXE]` generates temporary C, invokes `cc` (or `$CC`), links an executable (default `./a.out`) and removes the temporary file. Added five CLI integration tests, including end-to-end build-and-run tests skipped when `cc` is unavailable.
- 2026-11-05: Codegen hardening: `alloc` now lowers to `loi_alloc_slice(count, element_size)` in both statement and expression positions, with `SIZE_MAX` multiplication/alignment overflow checks; range loops use `uint64_t` when the checked common type is `u64` (otherwise `int64_t`), fixing bounds above `i64::MAX`; `gen`/`build` reject an output path equal to the input. Added one codegen and one CLI test.
- 2026-11-05: Build-driver ergonomics: added `-h`/`--help` and `-V`/`--version`; `loi build --emit-c PATH` retains the generated C instead of deleting it; repeatable `--cc-flag FLAG` passes extra arguments to the C compiler; `--emit-c` and `--cc-flag` are rejected outside `build`, and `--emit-c` may not equal the input or the executable. Added four CLI integration tests.
- 2026-11-05: Build outputs: `loi build --emit exe|obj|lib` produces an executable, a relocatable object (`-c`) or a static archive (object plus `ar rcs`). Defaults are `./a.out`, `<input stem>.o` and `lib<input stem>.a`; `$AR` overrides `ar`. Added three CLI integration tests, including linking and running both emitted artifacts.
- 2026-11-05: Build profiles: `--debug`/`-g` passes `-g` and `--release` passes `-O2` to the C compiler; the two are mutually exclusive and build-only. A `CC` wrapper records argv in a test to verify the exact flags. Added two CLI integration tests.
- 2026-11-05: Incremental builds: `loi build` hashes the generated C, compiler/archiver path, artifact kind, profile and `--cc-flag` values with FNV-1a and stores the result in `<output>.loi-stamp`. If the output and retained C exist and the stamp matches, compilation is skipped; the stamp is written only after a successful build. Added one CLI integration test using a `CC` wrapper to count invocations.
- 2026-11-05: `loi build --rebuild` bypasses the stamp and always invokes the toolchain; it is rejected outside `build`. Extended the incremental test to cover the forced rebuild and updated `build_source` to take a `BuildRequest` struct, avoiding an oversized argument list.
- 2026-11-05: `loi clean FILE [-o OUTPUT]` removes build artifacts and `.loi-stamp` sidecars without parsing or checking the source. Without `-o` it removes the default `./a.out`, `<stem>.o` and `lib<stem>.a` only when their stamps exist; with `-o` it removes that output and stamp, and rejects an output equal to the input. Added three CLI integration tests.
- 2026-11-05: Tool arguments: `CC` and `AR` are parsed as POSIX-shell-like command lines (whitespace splitting, single/double quotes and backslash escapes), so wrappers such as `CC='ccache cc'` or a quoted tool path with spaces work. Extra tool arguments are prepended before `-std=c11`, profile, `--cc-flag` and artifact flags and are included in the incremental fingerprint. Empty values are rejected with exit 2. Added four binary unit tests and one CLI integration test.
- 2026-11-05: Allocation sequencing: statement-position `alloc` lowers to a captured target address, a single count temporary, and a store through that pointer, so target/count side effects no longer depend on C's unspecified evaluation order. Added one codegen integration test. The unreachable expression-position lowering of `alloc` (its checked type is `void`) still uses the old form.
- 2026-11-05: Retained-C cleanup: build stamps now record the `--emit-c` path on a second line (`emit-c=...`, with `%`, CR and LF percent-encoded), so `loi clean` removes generated C along with the artifact and stamp. Stamp matching also compares the recorded path, so switching `--emit-c` targets forces a rebuild even when the new path already exists. Added two binary unit tests and two CLI integration tests.
- 2026-11-05: Integer-literal range checking: unsuffixed decimal literals (including a directly negated literal) are rejected when they do not fit the expected integer type at `let`, assignment, compound assignment, `in` call arguments, `return`, interface fields/defaults and range bounds. Binary operands are checked when the other operand fixes a concrete integer width (so `u64 > -1` and `u8 + 300` are rejected). Untyped `let` checks against the inferred `i32` default, and literals larger than `u128` saturate to an out-of-range diagnostic. Explicit `as` casts remain unchecked truncation. Added three checker integration tests (extended with binary-operand cases).
- 2026-11-05: Range-loop edge coverage: a compile-and-run test exercises signed negative-to-positive bounds, descending and equal bounds (zero iterations), unsigned bounds, and ranges ending exactly at `UINT64_MAX`, confirming the last increment does not wrap. It also asserts that signed and unsigned loops emit `int64_t`/`uint64_t` induction variables respectively. Added one codegen integration test.
- 2026-11-05: Negative constant slice indices are rejected by the checker with `slice index cannot be negative`, including when the index is an assignment target. `-0` and even numbers of negations remain valid because their value is non-negative. Added one checker integration test. The runtime `loi_index` still checks the upper bound, since a slice carries no compile-time length.
- 2026-11-05: Integer constant folding: the checker evaluates expressions made only of decimal literals, unary negation and `+`, `-`, `*`, `/`, `%`, then applies the fixed-width range check to the folded value. This catches `let x: u8 = 200 + 100`, `512 / 2` and similar cases, and extends negative-index detection to expressions such as `xs[0 - 1]`. Constant divisors that fold to zero are reported as `division by zero`/`remainder by zero`. Results beyond `i128` saturate, which keeps them out of range of every supported type. Added two checker integration tests.
- 2026-11-05: Source positions on runtime failures: `loi_index`, `loi_alloc_slice` and `loi_alloc` now receive `line` and `column` constants and emit `loi: <message> at <line>:<column>` before aborting. Arena exhaustion and allocation overflow report their source position the same way. Codegen inserts `expr.span.line`/`expr.span.column` at each call site.
- 2026-11-05: Runtime/I/O surface via intrinsics (temporary decision for spec question 9): `print(in []u8)` lowers to `fwrite(msg.data, 1, msg.len, stdout)` and `sqrt(in f64) -> f64` lowers to `sqrt(x)` from `<math.h>`. Executable builds link with `-lm`. Intrinsics are pre-declared in the checker with fixed signatures and mapped to C calls in the codegen. Added two codegen integration tests and three checker integration tests.
- 2026-11-05: C header generation (A1): `loi gen FILE --emit-h header.h` and `loi build FILE ... --emit-h header.h` generate a C header with declarations for `visible` items only. Functions `visible` are emitted as global (non-`static-inline`) in the .c; `hidden`/unspecified items stay in the .c only. Header includes interface typedefs/structs, function prototypes, `main`, and intrinsic declarations (`print`, `sqrt`). Added two codegen integration tests.
- 2026-11-05: Multi-file builds (A2): `loi build FILE1 FILE2... [-o OUT] [--emit exe|obj|lib] [--emit-h ...]` compiles each `.loi` to a `.o` next to the source, generates a `.h` header next to each source, then links into an executable, object, or static library. Single-file builds retain stamp-based incremental builds, `--emit-c`, `--rebuild`, and retained-C cleanup. Multi-file builds require `-o OUTPUT` (added CLI validation). Added three CLI integration tests.
- 2026-11-05: Interface value-cycle detection in the checker: self-referential and mutually recursive interfaces by value (e.g. `A { b: B }` / `B { a: A }`, `A { a: A }`) are rejected with a clear diagnostic at the interface span. Slice fields (`[]B`) do not create a value dependency because they store a pointer. Acyclic chains are allowed. This moves the check from the backend (where it only emitted a generic error) to the checker, where it belongs. Added one checker integration test.
- 2026-11-05: `while true` with a body that always returns satisfies definite return
- 2026-11-05: `break` and `continue` statements validated to appear only inside loops
- 2026-11-05: Hexadecimal and binary integer literals with underscore separators
- 2026-11-05: Allocator failure policy — `--arena-size BYTES` flag added to `loi build` to configure the protect arena size (default 64 MiB). The size is passed to the C compiler as `-DLOI_ARENA_SIZE=...` and participates in the incremental fingerprint, so changing it forces a rebuild. Rejected outside `build` with exit 2. Added three CLI integration tests.
- 2026-11-05: Source maps / debug-info — `loi gen FILE --source-map map.json` and `loi build FILE ... --source-map map.json` generate a JSON Lines source map mapping generated C line:column back to source Loi line:column. Inline `// loi:line:col` comments are also emitted at function/interface definitions in the generated C. Added one codegen integration test.
- 2026-11-06: IR (Intermediate Representation) — `src/ir.rs` adds `IrType`, `IrInst`, `IrBlock`, `IrFunction`, `IrModule`, `IrBuilder` for constructing IR, and `program_to_ir_module` to convert checked `Program` to `IRModule`.
- 2026-11-06: Assembly backend (`asm.rs`) — `generate_asm` converts `IrModule` to x86-64 AT&T assembly; `generate_asm_from_program` converts `Program` → `IR` → ASM; supports `alloc`, `load`, `store`, `gep`, `binop`, `unop`, `cmp`, `call`, `return`, `jump`, `condjump`, `label`, `phi`, `bitcast`, `inttoptr`, `ptrtoint`, `allocslice`, `indexslice`.
- 2026-11-06: CLI — `loi gen FILE --emit-ir IR_FILE` and `loi build FILE --emit-ir IR_FILE` emit IR text; `loi gen FILE --emit-asm ASM_FILE` and `loi build FILE --emit-asm ASM_FILE` emit x86-64 assembly.
- 2026-11-08: Repaired the broken working tree: reconstructed the deleted `check_expr`, `check_base_type`, `check_place_state`, `check_interface_literal` and `collect_index_reads` in `src/checker.rs`, fixed stray braces in `check_call_aliasing`, `collect_value_regions` and `collect_accesses`, and removed the obsolete `fix_*.py` scripts plus duplicate `src/ir.rs.bak*` files. `check_interface_literal` also skips `missing field` diagnostics for fields that declare a default, matching the reference program.
- 2026-11-08: Reverted the unfinished pointer experiment: removed `ExprKind::AddrOf`/`Deref`/`Nil` and `UnaryOp::AddrOf`/`Deref` from the AST/parser, and `Ty::Ptr`/`is_pointer` from the type system. The spec now decides that Loi has no pointers (5.2), so this keeps the tree consistent instead of carrying a half-implemented feature.
- 2026-11-08: Ran `cargo fmt` over the tree and cleared all `cargo clippy --all-targets -- -D warnings` findings in `asm.rs`, `codegen.rs` and `ir.rs` (unused bindings/imports, `push('\n')`, `Default` impls, collapsible `if`).
- 2026-11-08: P0 literals/length: added `TokenKind::String` with decoded escapes (`\n`, `\t`, `\r`, `\0`, `\\`, `\"`, ASCII `\xNN`), `ExprKind::String` typed as `[]u8`, and static byte-array emission in C. Added `ExprKind::SliceLiteral` for `[a, b]` (inferred) and `[]T{ a, b }` (explicit), lowered to C compound literals, with a checker rejection for returning a slice literal. Added the `len(xs)` intrinsic and `xs.len` member, both `u64`.
- 2026-11-08: Fixed a false-aliasing bug by including constant index values in ownership paths (`xs[0]`/`xs[1]` are disjoint), while dynamic indices remain conservatively `xs[]`. `out`/`inout` calls with disjoint constant indices now type-check.
- 2026-11-08: P1 globals/imports: top-level `let`/`var` declarations are checked in a base scope with constant-initializer enforcement and duplicate/function-conflict diagnostics; the C backend emits static globals (or `extern` in subset builds) for scalars and `[]u8` strings. Added `import "path"` parsing and recursive driver-side loading with canonical-path deduplication. Multi-file builds now check one merged program and emit one C unit per explicit input via `generate_c_subset`, with imported items owned by the first unit that pulls them in.
- 2026-11-08: P2 runtime/library: expanded intrinsics to `print`, `eprint`, `panic`, `sqrt`, `floor`, `ceil`, `fabs`, `pow`, `fmod` and `len`; added `lib/std.loi` with `min_i32`/`max_i32`/`abs_i32`/`clamp_i32`, float equivalents, and `str_len`/`str_eq`/`str_starts_with`/`str_find`. Restricted `==`/`!=` to scalars and `[]u8` and lowered byte-slice equality to `loi_slice_eq` (length plus `memcmp`), closing the invalid C struct-comparison gap.
- 2026-11-08: Added `examples/language_basics.loi` and integration coverage for string escapes, slice literals, `len`/`.len`, globals, imports, cross-file calls/globals, the standard library, byte-slice equality, disjoint-index aliasing and pretty-printing of the new syntax. Suite now has 157 passing tests.

## Known Bugs / Limitations
- Initialization tracking is per variable and whole-path based. Writing any field of an uninitialized `out` parameter marks the whole variable initialized; partial initialization of individual fields is not modelled.
- Move state is flow-sensitive but joins are conservative: a value moved on any continuing branch or loop iteration is treated as dead afterwards. Branches that end in `return` are excluded from the join. Loops are assumed to execute zero times for definite assignment.
- `move` liveness is tracked for variables, fields and elements inside one function. Across calls, parameter consumption is summarized per whole parameter: if a function may move from a parameter anywhere, the caller's matching `out`/`inout` argument is treated as moved even if the callee reinitializes it. Partial consumption (for example only `p.pos`) is conservatively widened to the whole argument.
- `protect` escape analysis tracks allocations through locals, fields, moves, interface literals, `out` arguments and `return`. It conservatively forwards taints from consumed `inout` arguments to every slice-containing `out` argument and composite result of the call. It does not track taints through callees that only mutate `inout` storage in place, and it may over-taint unrelated outputs. The limitation is deliberate and recorded here.
- The allocation-effect prepass treats `main` and every uncalled visible/default function as an entry point. An uncalled `hidden` function with an unguarded `alloc` may rely on a caller's `protect`, under the assumption that `hidden` means it cannot be reached from outside Loi code.
- `clone` of a value containing a slice is rejected recursively through shapes; other ownership-sensitive `clone`/`give` rules are provisional.
- There are no implicit conversions between named numeric types; mixing `i32` and `u32` requires an explicit `as` cast. Unsuffixed integer literals coerce to any numeric type, but a literal that does not fit the expected fixed-width type is now rejected; explicit `as` casts still truncate.
- Integer constant folding covers decimal/hex/binary literals, unary negation and `+`, `-`, `*`, `/`, `%`; comparisons, casts, float expressions and identifiers are not folded. Folding is used for diagnostics only, so the generated C still contains the original expression. Arithmetic that overflows `i128` saturates to the corresponding extreme and is therefore rejected for every supported type. Explicit `as` casts still truncate without a range check. Constant division/remainder by zero is an error, but a non-constant zero divisor is left to the C runtime. Untyped `let x = <constant>` uses the provisional `i32` default, so an otherwise valid `u64` constant needs an explicit type annotation.
- Casts are only allowed between numeric types; `bool` and interface casts are rejected.
- `%` is provisionally restricted to integer operands.
- Interface default expressions are type-checked in an empty environment, so they cannot refer to other fields or parameters.
- String literals are decoded UTF-8 byte slices with no implicit trailing NUL. `\xNN` escapes are restricted to ASCII bytes; non-ASCII escapes need raw UTF-8 text. Strings are static data and may initialize `[]u8` globals.
- Slice literals use automatic-storage backing arrays. The checker rejects returning one, but it does not yet detect every dangling case (for example storing a block-local literal slice into an outer-scope variable). Use `alloc` for slices that must outlive their block. Empty inferred literals (`[]`) do not parse; write `[T:]`.
- Globals are limited to scalars, `bool`, and `[u8]` initialized from a string literal. Composite/global slice-literal initializers need a runtime constructor and are rejected. Global initializers must be compile-time constants; global give/aliasing semantics are not separately analyzed.
- Imports are textual inclusion with no namespaces. A file is included once per compilation unit (canonical-path deduplication), and visibility is not enforced across files. Multi-file builds give hidden functions external linkage so cross-file calls link; the first unit that pulls an imported item in owns its definition.
- `==`/`!=` are defined only for scalar numerics, `bool` and `[u8]` (structural byte equality). Other shapes and slices must be compared field by field.
- The intrinsic list is the closed set from `LOI_SPEC.md` 5.9. `panic(msg)` writes to stderr and exits with status 1; there is no unwinding.
- The pretty-printer discards comments (the lexer does not retain them) and prints shape literals on one line, which can produce long lines. It emits canonical Flow Syntax.
- Newlines are separators, not whitespace inside expressions: a binary expression split across lines without parentheses is not accepted.
- Interface fields separated only by spaces (no comma or newline) are currently accepted leniently. Call arguments enforce comma/newline separation.
- `comptime` is lexed but not parsed. Section 5.1 now defines its semantics (a compile-time guarantee), but the constant evaluator and parser support are Phase 3 work. Freshness, spatial tiers and ordering blocks are not implemented.
- Slice indexing and iteration emit a runtime upper-bound check through `loi_index`, which now reports `loi: slice index out of bounds at <line>:<column>` before aborting. Arena exhaustion and allocation overflow report their source position the same way. There is no recovery after a runtime abort. Constant non-negative indices cannot elide the check because a slice value carries no compile-time length, but negative literal indices (other than `-0` and equivalent even negations) are rejected during checking instead of aborting at runtime.
- Numeric literals accept `_` separators, exponents (`2.5e-4`), hex (`0x`/`0X`) and binary (`0b`/`0B`) prefixes; the spec does not define numeric syntax yet.
- The canonical reference is `examples/flow_syntax.loi`; `examples/particles.loi` is byte-identical to it and remains the historical plural-named reference. The old spec/`AGENTS.md` filename mismatch (`particle.loi` / `particle.lum`) is obsolete.
- Flow Syntax is canonical-only. Legacy keywords and forms are no longer accepted; new code uses `shape`/`flow`/`<-`/`->`/`<->`/`=>`/`mut`/`give`/`clone`/`open`/`sealed` (spec 5.10 cleanup complete).
- Single-file builds emit one C translation unit; multi-file builds emit one unit per explicit input plus generated headers. `loi build` shells out to `cc` (or `$CC`) for executables/objects and to `ar` (or `$AR`) for static libraries, and depends on those external tools being installed. Source maps are available through `--source-map`.
- Incremental builds are keyed by content and tool paths, not mtimes: changing the `cc`/`ar` binary in place, or editing a retained `--emit-c` file manually, does not invalidate the stamp. Use `--rebuild` (or delete `<output>.loi-stamp`/the output) to force a rebuild. The stamp is a sidecar file next to every build artifact and records the retained C path (when any) so cleaning can remove it.
- `loi clean` without `-o` only considers the three default artifact names and only removes them when a stamp exists; custom outputs require `-o`. It removes generated C only when the stamp records it (a C file retained by an older stamp-less build is left alone). Cleaning is per current working directory, matching the default build locations.
- `loi build` always passes `-std=c11`; extra flags go through repeatable `--cc-flag` values (one literal argument each, not shell-split), and those flags are not forwarded to `ar`. `--debug`/`--release` are fixed shorthands for `-g`/`-O2`; there are no other optimization or sanitizer profiles. `$CC` and `$AR` are split with POSIX-shell-like rules (quotes and backslash escapes), so a path containing spaces must be quoted; non-UTF-8 values are treated as a single executable path without arguments. Without `--emit-c` the temporary C file lives in the system temp directory and is removed afterwards; library builds also use a temporary object file that is removed. `loi gen -o` and `loi build --emit-c` do not create missing parent directories.
- `<-` composite parameters are passed to C by value (slice headers still point at the caller's data), not "by its own address" as the spec describes. The checker already forbids storing or returning borrowed values, so single-threaded semantics are preserved.
- The backend uses one global 64 MiB bump arena. It is not thread-safe, cannot grow, aborts on exhaustion and only restores on `protect` exit, `continue`, `return` and `break`; there is no `break` statement missing anymore—it is implemented.
- `loi gen` rejects a `main` with parameters or a non-integer return type because the emitted C entry point is `int main(void)`.
- `alloc` lowers to `loi_alloc_slice(count, element_size)` with `SIZE_MAX` overflow checks. Statement-position allocations capture the target address in a pointer temporary, evaluate the count once into a `size_t` local, then store through the pointer, so evaluation order is explicit. Expression-position allocations are unreachable in checked programs because `alloc` has type `void`; that branch still emits the compact form.
- Shape layouts are ordered by value dependency; any cycle of shapes containing each other by value is now diagnosed by the checker (not just rejected by the backend).
- Range loops are ascending only: `for i in a..b` with `a >= b` runs zero times, and there is no reverse-iteration form. Induction uses `uint64_t` when the checked common type is `u64` and `int64_t` otherwise, so ranges up to `UINT64_MAX` are handled without the final increment wrapping; negative starts with signed bounds work, while a negative literal combined with an unsigned bound is rejected by literal-range checking.
- Fixed-width arithmetic uses C integer promotions in the backend. The checker may narrow an expression's type (for example `i8 + i8` stays `i8`), but C evaluates it at `int` width before a store, so overflow in an un-stored subexpression can differ from the checker's narrow view. Assignment and `as` casts truncate at the destination.
- The backend assumes a checked program and does not re-run semantic validation; calling `generate_c` on an unchecked AST may emit invalid C.
- Generated C I/O/math is limited to the intrinsic surface (`print`, `eprint`, `panic`, `sqrt`, `floor`, `ceil`, `fabs`, `pow`, `fmod`, `len`) plus `lib/std.loi`. Compile-and-run tests call the generated `static` functions from a C harness; Loi code itself cannot read input or open files.
- Lexer, parser and checker report lexer/parser errors at the first failure; semantic checking collects multiple diagnostics but there is no recovery inside the parser.

- Pointers (`&expr`, `*expr`, `nil`) are intentionally absent (`LOI_SPEC.md` 5.2). The partial AST/parser/type-system support was reverted on 2026-11-08 and should not be restored without a new decision. Cyclic structures use handles (5.5).
- The IR (`src/ir.rs`) and x86-64 assembly (`src/asm.rs`) backends are drafts. IR lowering covers interfaces, functions, calls, returns and a simplified range-loop shape; string and slice literals currently return a codegen error in the IR path, and globals are not lowered. The assembly emitter uses simplified stack-based calls and does not yet implement every IR instruction faithfully. The C backend remains the production path.
- The playground executes arbitrary Loi programs with the permissions of the server process and is a local development tool, not a sandbox. It binds to `127.0.0.1` by default and must not be exposed publicly without real isolation. Its limits are 200 KB of source, 128 KB of output, 25 s to check, 30 s to build, 10 s to run and four concurrent jobs. Browser-level JavaScript behavior was reviewed statically and the API was exercised with Node/curl; no automated browser test suite is included.
- `website/examples/` is a duplicate of `examples/` used by the site and playground. The two must be updated together; `diff -r examples website/examples` is the check.

## Open Design Questions

The nine language-level questions are resolved in `development/LOI_SPEC.md` section 5. The items below are remaining implementation or secondary-design choices.

- Imports are textual (`import "file.loi"`) with no namespaces or cross-file visibility enforcement. Temporary decision: the driver merges every imported item into one program and gives multi-file sealed functions external linkage. The default-visibility rule is settled (`sealed`, 5.3); enforcement and module semantics are not.
- Globals are a small, provisional addition: scalar/`bool` and `[u8]`-from-string only, with constant initializers. Composite globals await a runtime constructor and a decision on global ownership.
- Equality is restricted to scalars and `[u8]`; byte slices compare structurally. Shape equality is deliberately left to explicit field comparisons until the language defines structural equality.
- Standard functions are settled as the closed intrinsic set plus explicitly imported `lib/std.loi` (5.9). How the library is namespaced once modules exist is still open.
- String literals have static storage and no NUL terminator; `\xNN` is ASCII-only. Revisit if the language needs C interop strings or arbitrary byte escapes.
- Slice literals have automatic storage and are rejected only when directly returned. Full escape analysis for block-local backing arrays is not implemented.
- Newline handling: tokens vs. parser-level line tracking. Temporary decision: emit `Newline` tokens and use them as separators.
- Numeric literal grammar is not specified. Temporary decision: decimal integers, floats, `_` separators and exponents.
- Numeric coercion is provisional: unsuffixed integer literals may coerce to any numeric type, float literals to float types, and named numeric widths never convert implicitly. Literal values and literal-only integer arithmetic are range-checked against the expected type when it is known.
- An untyped/inferred integer literal resolves to `i32`, so its literal must fit `i32`; a wider value needs an explicit annotation such as `mut x: u64 <- ...`. This follows the provisional default type rather than widening on demand.
- Constant expressions in shape defaults: temporary decision is to allow any type-correct expression with no variables in scope; a stricter constant-only rule can be added later.
- Definite return is implemented with reachability, but loops are treated as possibly executing zero times. Temporary decision: a function that only returns inside an infinite loop is rejected; add condition/termination analysis later if needed.
- Cross-function gives use whole-parameter consumption summaries: any `give` from a parameter, or returning a composite place rooted at it, marks it consumed, and callers mark the matching `->`/`<->` argument moved. This is conservative for partial moves and reinitialization.
- Protect-region taints propagate from consumed `<->` arguments to all slice-containing `->` arguments and composite results of the call. This is conservative but avoids a full interprocedural dataflow analysis.
- Entry functions for the allocation check are `main` and every uncalled function except uncalled `sealed` helpers. This assumes `sealed` is not externally reachable and that any Loi caller of such a helper provides a `protect` when needed.
- Definite assignment uses intersection at branch joins, excludes branches ending in `=>`, and treats loops as possibly executing zero times. Give/move state uses union over continuing paths, which is conservative.
- `->`/`<->` call exclusivity compares ownership paths, so disjoint fields of the same variable are allowed; the spec sentence "no other use of it" is interpreted per value, not per variable.
- Loop variable types: `for i in a..b` uses the common integer type of the bounds (`i32` when both are unsuffixed literals).
- `<-` composite parameters: the spec says "read-only access ... by its own address, no copy", but the C backend passes composites by value. Temporary decision: rely on the checker's borrow rules and single-threaded execution; revisit when aliasing or concurrency matters.
- `->` composite parameters: the backend starts them from shape field defaults (including nested defaults) before the body runs. Temporary decision: makes `World.tick = 0` observable even when only some fields are written; the spec does not state this explicitly.
- Language-level decisions now live in `development/LOI_SPEC.md` section 5 and are not duplicated here. Flow Syntax (5.10) is canonical-only and the cleanup is complete.
- The bracket-literal heuristic treats `Name[ ... ]` as a shape literal only when a top-level `:` appears before the matching `]` (or the brackets are empty). This keeps indexing unambiguous; a malformed shape literal without a field colon is parsed as an index and reported there.

## Next Steps (recommended)
1. Enforce cross-file visibility and replace textual imports with a real module/namespace design, or record the final decision in `LOI_SPEC.md`.
2. Add a runtime constructor for composite globals (or define globals as compile-time-init only), and extend ownership analysis to block-local slice-literal escapes.
3. Finish IR/ASM support for strings, slice literals, globals and the new intrinsics, or mark those paths explicitly unsupported in the CLI.
4. Implement `comptime` per 5.1: parser support, constant evaluator with a step limit, and constant contexts (globals, defaults, extents).
5. Start the linear-resource design for `guard`/`<Shape>_release` (5.6) before adding file or socket APIs.
6. Add examples that need richer I/O (the current tic-tac-toe example is deterministic because Loi has no stdin/file API).
7. Harden the playground if it ever leaves localhost: real sandboxing, resource limits beyond time/size, and a browser-based regression suite. Keep `website/examples/` mirrored from `examples/` and the docs in sync with `LOI_SPEC.md`.
