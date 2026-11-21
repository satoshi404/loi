# Loi Compiler — Implementation Progress

## Current Status

- **Phase:** 4 — the C11 backend is the production path. `loi gen` writes C,
  `loi build` compiles and links executables, objects and static libraries.
- **Current task (2026-11-21, Block L5 — process spawning with captured output):** `lib/process.loi` and `lib/process_shim.c` add a real process API without touching the closed intrinsic set or the C11 backend. The C shim forks, runs `/bin/sh -c`, redirects stdout and stderr to temporary files before `exec` (no pipe deadlock), and reaps the child with `waitpid`; it stores the captured buffers in a fixed table and returns a positive integer handle. The Loi side wraps that handle in a linear `Process` shape with `process_release`, so `guard` frees the buffers on every exit path. `process_spawn_args(program, args)` shell-quotes an explicit program and argument list, escaping an embedded `'`; `process_stdout` / `process_stderr` allocate in the caller's `protect` region, `process_read_stdout` / `process_read_stderr` copy into caller-owned buffers, and `process_exit_code` reports the status (128 + signal, or -1). `os_env(name: [u8]) => [u8]` in `lib/os.loi` reads runtime environment strings through a new `loi_os_getenv` binding in `lib/os_shim.c`. `examples/lsp_server.loi` now runs `loi check <path> --error-format json` itself (compiler chosen by `LOI_COMPILER`, default `loi`), publishes its stderr as diagnostics, and no longer uses `diagnosticsPath` or `read_file`. `examples/process_run.loi` is the standalone runner. Rules are in `LOI_SPEC.md` §5.9, canonical surface in `SINTAXE.md` §10, friction in `FRICTION.md`, rationale in `DECISIONS.md` and public docs in `website/docs.html`.
- **Verification for Block L5:** the six tests in `tests/process.rs` were written first and failed with `No such file or directory` for `lib/process.loi` and `lib/process_shim.c`. They now pass: stdout, stderr and exit code 7 are captured exactly; a quoted program path containing spaces and an argument containing `'` round-trip through `process_spawn_args`; 200 KB of child output is captured without deadlock; a missing program reports shell status 127; `os_env` reads a runtime byte string and returns an empty slice for an unset name; and `guard child <- process_spawn(...)` emits `loi_fn_process_release(&(loi_v_child))` in the generated C. The four LSP tests and five JSON-RPC tests still pass with the file bridge removed; `process.loi` was added to the general library checks and both new shims to the strict C11 and build lists in `tests/lib.rs`. The example was mirrored with `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **520 passed, 0 failed** across 28 suites (was 514; six new process/environment tests). No compiler, generated C, header or runtime helper ABI changed, so `benchmark/run.py` is not applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-21, Block L4 — LSP diagnostics from the compiler, superseded by L5):** `lib/lsp.loi` converts the compiler's structured diagnostics into LSP `textDocument/publishDiagnostics` notifications. Loi has no process-spawn API, so a client writes `loi check FILE --error-format json` to a file named by `params.diagnosticsPath` in a `didOpen` / `didSave` notification; `examples/lsp_server.loi` reads it with `read_file` and calls `lsp_publish_diagnostics(uri, json)`. Compiler spans are 1-based and LSP ranges 0-based, so both endpoints are decremented; severity strings map `error`/`warning`/`note` to 1/2/3, messages and URIs are escaped, and empty or malformed input publishes an empty array to clear stale markers. `jsonrpc_parse_request` now sets `has_id: bool` on `JsonRpcRequest` (optional `id`, zero for notifications) and `lib/jsonrpc.loi` adds `jsonrpc_send_notification`. Rules are in `LOI_SPEC.md` §5.9, canonical surface in `SINTAXE.md` §10, friction in `FRICTION.md`, rationale in `DECISIONS.md` and public docs in `website/docs.html`. The spawn gap is an Open Design Question below.
- **Verification for Block L4:** the four tests in `tests/lsp.rs` were written first and failed because `lib/lsp.loi` did not exist, because notification messages without `id` were rejected with `-32700`, and because the server ignored `didOpen`. They now pass: a real `unknown variable 'missing'` diagnostic from `loi check --error-format json` is published with the exact 0-based range and severity 1; a clean check (zero bytes of JSON) publishes `diagnostics: []`; `initialized` plus `shutdown` with no `id` produces only the shutdown result; and `lsp_publish_diagnostics` outside `protect` is rejected at check time. `lsp.loi` was added to the general library checks, the existing five L3 tests still pass, and the example was mirrored with `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **514 passed, 0 failed** across 27 suites (was 510; four new LSP tests). No compiler, generated C, header or runtime helper ABI changed, so `benchmark/run.py` is not applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-21, Block L3 — JSON-RPC over stdio):** `lib/jsonrpc.loi` composes the two previous blocks into a JSON-RPC 2.0 transport. `jsonrpc_read_frame(-> body) => bool` reads one `Content-Length` frame from stdin (EOF, a malformed header or a zero-length frame ends the exchange); `jsonrpc_parse_request(<- body, -> request) => bool` fills `JsonRpcRequest` (`doc: JsonDoc`, `id: i64`, `method: [u8]`) after checking the `2.0` version and the `id`/`method` types; `jsonrpc_send_result` and `jsonrpc_send_error` write framed, flushed replies. Replies are raw JSON bytes appended to a `Vec<u8>` because the JSON library can parse but not construct documents; `result` must be a raw JSON literal and `message` must not need escaping. All allocating flows require `protect`, and the shim from L1 is still linked at build time. `examples/lsp_server.loi` answers `initialize` with `{"capabilities":{}}`, `shutdown` with `null`, unknown methods with `-32601`, and malformed JSON with `-32700`. Rules are in `LOI_SPEC.md` §5.6/§5.9, canonical surface in `SINTAXE.md` §10, friction in `FRICTION.md`, rationale in `DECISIONS.md` and public docs in `website/docs.html`.
- **Verification for Block L3:** the five tests in `tests/jsonrpc.rs` were written first and failed with `No such file or directory` for `lib/jsonrpc.loi` and `examples/lsp_server.loi`. They now pass: an initialize + shutdown exchange returns the two exact framed results, an unknown method returns `-32601`, malformed JSON returns `-32700` with id 0, a header without `Content-Length` yields no frame, and `jsonrpc_parse_request` outside `protect` is rejected at check time. Each server test closes stdin so a transport that still waited for a live client would time out after 10 seconds. `jsonrpc.loi` was added to the general library checks; the example was mirrored with `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **510 passed, 0 failed** across 26 suites (was 505; five new JSON-RPC tests). No compiler, generated C, header or runtime helper ABI changed, so `benchmark/run.py` is not applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-21, Block L2 — JSON in pure Loi):** `lib/json.loi` parses and serializes JSON with no shim. `json_parse(<- text, -> doc) => bool` fills a `JsonDoc` backed by a node table, a decoded-string arena and a separate child-handle arena; `json_root`, `json_kind`, `json_bool`, `json_string`, `json_number_text`, `json_number_i64`, `json_array_len`, `json_array_get`, `json_object_get` and `json_stringify` inspect or emit a document. Strings decode the standard escapes (including `\uXXXX` surrogate pairs) to UTF-8, numbers keep their raw lexeme, object order is preserved and malformed input or trailing bytes return `false`. Parsing and stringifying allocate, so callers need `protect`. `examples/json_rpc.loi` is a realistic LSP `initialize` request decoder. Rules are in `LOI_SPEC.md` §5.9, canonical surface in `SINTAXE.md` §10, friction in `FRICTION.md`, rationale in `DECISIONS.md` and public docs in `website/docs.html`.
- **Verification for Block L2:** the six tests in `tests/json.rs` were written first and failed before `lib/json.loi` existed (missing file, then the accessor and `protect` contracts). They now pass: an LSP-shaped object yields `method=initialize / id=1 / caps=object`; an array with escapes yields `7 / 3.5 / 6 / a\n"b`; a round-trip yields `{"b":[1,2,{"x":null}],"a":"x/y"}`; three malformed documents are rejected with `bad1=false bad2=false bad3=false`; invalid handles, missing keys and wrong-kind nodes return `-1`; and `json_parse` outside `protect` is rejected at check time. `json.loi` was added to the general library checks; the example was checked, built and run, and mirrored with `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **505 passed, 0 failed** across 25 suites (was 499; six new JSON tests). No compiler, generated C, header or runtime helper ABI changed, so `benchmark/run.py` is not applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-21, LLVM slices and regions):** the opt-in LLVM backend now covers scalar-slice programs in addition to scalars. Slices lower to `%LoiSlice = type { ptr, i64 }`, matching C's `LoiSlice` ABI, with IR helpers for a bump arena, `loi_alloc_slice`, checked `loi_index`, byte-wise `loi_slice_eq`, `loi_view` and C-style runtime diagnostics. Supported slice forms: literals, `<- [T]` parameters, `len(x)` / `x.len`, indexing and index assignment, `alloc(-> x, T, n)`, `protect`, byte-slice `print` / `eprint` / `panic`, `view` and `==` / `!=`. Element types are restricted to statically sized scalars; composites, nested slices, callbacks, `guard` and `comptime` remain explicitly rejected. `print` now uses `fwrite` with the slice length instead of `printf("%s")`, and `stdout`/`stderr` are loaded from their C globals before use. `examples/llvm_slices.loi` builds with both backends and prints identical output. Rules are in `LOI_SPEC.md` §6.2, canonical CLI/spelling in `SINTAXE.md` §14, rationale in `DECISIONS.md`, resolved friction in `FRICTION.md`, public docs in `website/docs.html`.
- **Verification for LLVM slices:** the two old rejection tests (`llvm_backend_rejects_protect_blocks`, `llvm_backend_rejects_slice_parameters`) were replaced with parity tests and failed before the implementation with `LLVM backend does not support slice types`; a new negative test still rejects shape slice elements. All nine tests in `tests/llvm.rs` pass now: `clang -c` validates the IR, C11 and LLVM agree on `sum=10 len=4 / arena=60 / Hi`, the runtime helpers are present, shape elements are rejected, and all CLI restrictions still hold. Outside the suite, probes confirmed equality, `view`, `eprint` and `panic` parity, including the exact `loi: slice index out of bounds at 5:15` diagnostic and exit status 1 from both backends. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **496 passed, 0 failed** across 23 suites (was 495; the two old rejection tests became three tests, a net gain of one). The C11 backend and its ABI are unchanged. ASan was used only during debugging of the `FILE*` global load, not as a claimed gate.
- **Previous task (2026-11-21, scalar LLVM backend):** `loi gen FILE --emit-llvm-ir FILE.ll` emits real LLVM textual IR for the scalar subset and `loi build FILE --backend llvm -o program` compiles it with `clang`; C11 remains the default production backend. The emitter lives in a new `src/llvm.rs`, works from the checked/monomorphized AST and does not touch the `src/ir.rs` draft or `--emit-ir`. It uses opaque pointers, `ptr` GEPs, the same `loi_fn_*` C symbols, `printf("%s")`/`printf("%lld")` and a `snprintf`/`strtod`/`fputs` helper that matches C11 `print_float`. Supported: integer/float/bool scalars, calls and recursion, scalar `extern flow`, `if`/`while`/`for`, `break`/`continue`, compound assignment, short-circuit logic, print intrinsics and the math intrinsics. Other constructs fail with an `LLVM backend does not support ...` diagnostic. `build --backend llvm` is single-file, executable-only, rejects C-only options with exit 2, and `--emit-llvm-ir` is gen-only while `--backend` is build-only. Rules are in `LOI_SPEC.md` §6.2, canonical CLI/spelling in `SINTAXE.md` §14, rationale in `DECISIONS.md`, resolved friction in `FRICTION.md`, public docs in `website/docs.html`. `examples/llvm_scalars.loi` is the realistic program that builds with both backends.
- **Verification for the scalar LLVM backend:** the seven tests in `tests/llvm.rs` failed 7/7 before the implementation with `unknown option '--emit-llvm-ir'` / `unknown option '--backend'`; an eighth parity test was added for the example. All eight pass now: textual IR validity through `clang -c`, C11/LLVM runtime parity (`10 55 2`), all five required rejection diagnostics, the CLI mode restrictions, and `factorial=120 squares=55 mean=2` from both backends for the example. A richer probe (recursion, `for`, compound assignment, floats, casts, short-circuit logic and scalar `extern flow` with a C shim) also printed identical output in C11 and LLVM. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **495 passed, 0 failed** across 23 suites (was 487; eight new LLVM tests). The C11 backend, its ABI and `--emit-ir` are unchanged, so `benchmark/run.py` remains not applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-20, typed C callbacks):** a callback type now carries
  its C signature (`callback(T, ...) => R`), and `callback(flow)` infers it from
  a non-`extern` Loi flow whose parameters all use `<-`. The inferred signature
  is checked against the declared callback type at every use, so a shim cannot
  silently receive a pointer with the wrong C type. Plain `callback` stays
  exactly `callback(i32, i32) => i32` for source and ABI compatibility; `void`
  is allowed only as a result. The C backend emits one guarded, structurally
  named typedef per signature (`LoiCallback_1_i32_i32`,
  `LoiCallback_1_i32_void`, `LoiCallback_0_i32`) shared by generated C and
  emitted headers, and still passes the flow symbol directly with no thunk or
  userdata. Rules are in `LOI_SPEC.md` §4.9, canonical spelling in `SINTAXE.md`
  §2/§15.6, rationale in `DECISIONS.md`, resolved friction in `FRICTION.md`,
  public docs in `website/docs.html`. `examples/callback_signatures.loi` with
  `examples/callback_signatures_shim.c` is the complete buildable program.
- **Verification for typed callbacks:** the nine new tests in
  `tests/callback_signatures.rs` failed 9/9 before the implementation (the
  `callback(T) => R` type spelling did not parse) and pass 9/9 now; they cover
  positive inference, fixed-alias identity, mismatched signatures and results,
  `void` parameters, non-input flow parameters, generated C typedefs, a strict
  C11 harness that runs three callback shapes, emitted headers, and copyability
  without comparability. One legacy checker assertion was updated to the new
  use-site diagnostic, and the example was added to the checker, codegen and CLI
  suites. The example was checked, built and run (`30`, `0`, `1`, `2`, exit 0)
  and mirrored with `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **487 passed,
  0 failed** across 22 suites (was 475; twelve new tests: 9
  `callback_signatures`, 1 checker, 1 codegen, 1 CLI). The C backend ABI changed
  only by adding guarded typedefs for non-fixed signatures; plain `callback`
  programs keep their symbols, so `benchmark/run.py` is not applicable.
  ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-20, module-qualified mangling):** two modules may now
  declare the same item name. The driver computes an internal `<module>::<name>`
  spelling for names declared in several modules and rewrites references before
  monomorphization; unique names are untouched, so single-module programs keep
  their ABI. Unqualified names bind to the current module first, then to a
  unique declaration elsewhere; otherwise the resolver reports an ambiguity
  with qualified candidates. The C backend sanitizes `::` to `__`, checks for
  symbol collisions, and uses the same names in headers and multi-file objects.
  `main`, intrinsics and same-module duplicates are excluded; `extern flow ...
  as` keeps its link name. Rules are in `LOI_SPEC.md` §4.8, canonical spelling
  in `SINTAXE.md` §13, rationale in `DECISIONS.md`, resolved friction in
  `FRICTION.md`. `examples/module_mangling.loi` and its two imported modules
  are the realistic example.
- **Verification for module mangling:** the first nine tests in
  `tests/mangling.rs` failed 6/9 before the implementation (the three expected
  rejections already passed); a tenth test for shared explicit link names was
  added during validation. All ten pass now. They cover duplicate names in
  two modules, own-module unqualified binding, ambiguous cross-module calls,
  C symbols, headers, multi-file linking, all declaration kinds including
  generic instances, sanitized-symbol collisions, shared explicit link names and
  same-module duplicates.
  The example was checked, built and run (`17`, exit 0) and mirrored with
  `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **475 passed,
  0 failed** across 21 suites (was 465; ten new mangling tests). This change is
  ABI-visible only for duplicated names, so `benchmark/run.py` is not applicable
  to the unchanged single-module path.
  ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-20, structural `Eq` + `Ord`):** `==`/`!=` are now
  structural for numbers, bool, marks, `cstr`, opaque handles, shapes, choices
  and slices whose elements support equality; `<`, `<=`, `>` and `>=` follow
  the same lexicographic rule except for opaque handles and callbacks, which
  have no order. A fourth bound, `Ord`, joins `Clone`, `Copy` and `Eq`. The
  checker, monomorphizer and C backend share one rule via
  `types::ComparisonCaps`/`ComparisonEnv`; the backend emits on-demand
  `loi_eq_*`/`loi_ord_*` helpers while `[u8]` keeps `loi_slice_eq`.
  `vec_find<T>`/`slice_find<T>` use structural `Eq` and
  `vec_sort<T>`/`vec_binary_search<T>` declare `T: Clone + Ord`.
  `examples/ordering.loi` sorts and searches `Vec<Task>` by priority, status,
  id and label. Rules are in `LOI_SPEC.md` §4.8/§5.1a and `SINTAXE.md` §3.2,
  §5.4, §9/§10; rationale in `DECISIONS.md`; limits in `FRICTION.md`.
- **Verification for structural `Eq`/`Ord`:** the seven new tests in
  `tests/comparison.rs` failed 7/7 before the implementation and pass 7/7 now;
  they cover positive and negative checker cases, `Ord` bounds, and a
  strict-C11 harness that runs generated helpers. Older narrow assertions were
  updated, and library tests now sort and search `Vec<Point>` with whole-shape
  comparisons. The example was checked, built and run (`ordering ok`, exit 0)
  and mirrored with `python3 tools/sync_examples.py`. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **465 passed,
  0 failed** across 20 suites (was 455; ten new tests: 7 comparison, 2 library,
  1 CLI). Existing generated names, headers and ABI are unchanged; the C
  backend only adds helpers used by structural comparisons, so
  `benchmark/run.py` is not applicable. ASan execution is still not claimed in
  this PRoot container.
- **Previous task (2026-11-20, Block B6 — C callbacks):** Loi now has a fixed-ABI
  FFI type `callback` (`int32_t (*)(int32_t, int32_t)`) and a closed
  `callback(flow)` intrinsic. Only a non-extern flow with exactly
  `<- a: i32, <- b: i32 => i32` is accepted; the generated C passes that flow's
  symbol directly, so no thunk, trampoline or userdata is needed. Callback
  values are copyable and can be stored in locals, shape fields and slices, but
  cannot be called, compared or dereferenced from Loi. A `callback` parameter
  must use `<-`, and an `extern flow` cannot return `callback`; ordinary Loi
  flows may return callback values. The generator still
  skips C function pointers; declare those `extern flow`s by hand. Rules are in
  `SINTAXE.md` §2 and §15.6, `LOI_SPEC.md` §4.9, `DECISIONS.md`, `FRICTION.md`
  and `website/docs.html`. `examples/callbacks.loi` with
  `examples/callbacks_shim.c` is the complete buildable program.
- **Verification for Block B6:** the first end-to-end probe
  (`apply_cb(9, 4, callback(add))` with a C harness) compiled and returned 0,
  and the negative probes produced the intended diagnostics, including bad
  directions and `extern flow` callback results. The stored
  probe (`pos_store.loi`) initially generated `void loi_v_cb = loi_fn_add;`
  because the codegen intrinsic-return map lacked `callback`; after adding it,
  locals, flow returns, shape fields and callback slices all emit
  `LoiCallback`. Added three checker tests, three codegen tests (generated C,
  strict C11, sanitizer compile, C harness run, emitted header) and one CLI test
  that runs the complete `examples/callbacks.loi` + shim build. Final gate
  command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **455 passed,
  0 failed** across 19 suites (was 448; seven new tests). The C backend ABI
  changed only by adding a guarded typedef and a new parameter type, so
  `benchmark/run.py` is not applicable. ASan execution is still not claimed in
  this PRoot container.
- **Previous task (2026-11-20, Block B5 — binding generator writable buffers):**
  `tools/bindgen/bindgen.py` maps a C mutable `char *` parameter to a writable
  Loi `<-> [u8]` slice. The compiler passes `<->` slices as a pointer to the
  slice struct, so the generated shim takes `LoiSlice *` and forwards
  `buf->data`; the C length parameter remains explicit, so a call is
  `sample_fill(<-> buffer, count)`. `const char *` stays `cstr`, and pointer
  results stay skipped because ownership of the returned storage is
  unspecified. Rules are in `SINTAXE.md` §15.5, `LOI_SPEC.md` §4.9,
  `DECISIONS.md`, `FRICTION.md` and `website/docs.html`.
- **Verification for Block B5:** before implementation, the extended
  `tests/bindgen.rs` failed with
  `a mutable buffer must reach C through the shim's data pointer`; after the
  first mapping attempt, `loi build` rejected the probe because `sample_fill`
  was called with `<-` instead of `<->`, confirming the generated direction.
  The test now asserts the `<-> [u8]` binding, the `LoiSlice *` shim parameter,
  the `buf->data` cast, the filled byte and a skipped mutable pointer result.
  The generated program is built with the shim under strict C11 flags and run.
  Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **448 passed,
  0 failed** across 19 suites (unchanged count: the existing bindgen test was
  extended, not split). Only `bindgen.py`, its fixture and its test changed; no
  compiler, ABI or generated header changed, so `benchmark/run.py` is not
  applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-20, Block B4 — binding generator input buffers):**
  `tools/bindgen/bindgen.py` maps a C `const T *` parameter to a borrowed
  Loi `[T]` slice. The generated shim takes the shared `LoiSlice` layout and
  passes `slice.data` to C; a companion length parameter remains explicit, so a
  call is `sample_checksum(bytes, bytes.len)`. `const char *` still binds as
  `cstr`, and `const T *` results remain skipped because ownership of the buffer
  is unspecified. Rules are in `SINTAXE.md` §15.5,
  `LOI_SPEC.md` §4.9, `DECISIONS.md`, `FRICTION.md` and `website/docs.html`.
- **Verification for Block B4:** before implementation, the extended
  `tests/bindgen.rs` failed on the new positive assertions with
  `sample_checksum: parameter 'bytes' is a const pointer to a scalar` in
  `skipped.txt`. Added `const unsigned char *` and `const int *` input-buffer
  functions to `tools/bindgen/tests/sample.h`/`.c`, asserted the generated
  `[u8]`/`[i32]` flows, the guarded `LoiSlice` typedef and the shim cast, and
  kept negative checks for const pointer returns, pointer-to-pointer, mutable
  `char *` and variadics. The generated program is built with the shim under
  strict C11 flags and run. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **448 passed,
  0 failed** across 19 suites (unchanged count: the existing bindgen test was
  extended, not split). Only `bindgen.py`, its fixture and its test changed; no
  compiler, ABI or generated header changed, so `benchmark/run.py` is not
  applicable. ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-20, Block A6 — official `Option<T>` and `Result<T, E>`):**
  `lib/option.loi` and `lib/result.loi` provide optional and error-carrying values
  as generic shapes with an explicit `bool` tag: `Option<T>` has `present` and
  `value`, `Result<T, E>` has `ok`, `value` and `error`. They are shapes, not
  generic `choice` declarations, because choices are not generic and forbid
  slice payloads. The unselected field is zero-initialized by the `->` out-param
  and is only read through `option_get_or`, `option_unwrap`, `result_unwrap_or`,
  `result_unwrap` or `result_unwrap_err`; the three `unwrap` flows abort with a
  flow-naming message. Constructors and readers clone their `<-` inputs, so a
  caller needs `protect` when `T` or `E` contains slices. No compiler, parser or
  C-backend change was needed. Rules are in `LOI_SPEC.md` §4.8, canonical names
  in `SINTAXE.md` §14, rationale in `DECISIONS.md` and `FRICTION.md`, public docs
  in `website/docs.html`; runnable example: `examples/options_results.loi`
  (`workers=8`, `not a number`).
- **Verification for Block A6:** before implementation, the 6 new tests in
  `tests/lib.rs` failed with
  `loi: /root/loi/lib/option.loi: No such file or directory (os error 2)`.
  Added 5 runtime tests (presence/fallback, independent slice payloads,
  `option_unwrap` on an absent value, ok/error reporting, `result_unwrap` on an
  error), 1 checker test for a slice payload outside `protect`, and extended
  `every_library_source_checks_cleanly` and `all_libraries_can_be_imported_together`
  to include both modules. Both libraries were checked directly with
  `loi check`; the example was checked, built and run. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **448 passed,
  0 failed** across 19 suites (the A3 total of 442 plus 6 new tests). No C code,
  ABI or generated header changed, so `benchmark/run.py` is not applicable.
  ASan execution is still not claimed in this PRoot container.
- **Previous task (2026-11-20, Block A3 — real views):** `view(xs, start, end)` is
  a zero-copy subslice intrinsic. The checker computes per-flow view summaries
  to a fixpoint: a borrowed slice may escape through `=>`, a composite result
  containing a slice, or a `->` parameter whose type contains a slice. Loans
  carry both the source place and the holder slot storing the view, so they
  survive sibling writes and sub-place reads, and are released only when that
  slot is overwritten or its scope ends. Writing, moving or mutably borrowing
  an overlapping source path is rejected while a view is live; the existing
  region taints still stop block/`protect` storage from escaping. `lib/slice.loi`
  adds `slice_view<T>` and `lib/vec.loi` adds `vec_view<T>`;
  `vec_as_slice<T>` keeps its detached-copy behaviour. Rules are in `SINTAXE.md`
  §6 and §14, `LOI_SPEC.md` §4.4a and §5.9, `DECISIONS.md`, `FRICTION.md` and
  `website/docs.html`; runnable example: `examples/views.loi` (`head=10 body=7`).
- **Verification for Block A3:** before implementation, 11 of 12 new tests in
  `tests/views.rs` failed with `unknown function 'view'`. Added 12 checker/codegen
  tests (borrowed returns, forwarding, view-of-view, region escape, write/move
  conflicts, scope end, clone, argument checks, helper lowering) plus 2 runtime
  tests in `tests/lib.rs` (real aliasing through `view`, `slice_view<T>` and
  `vec_view<T>`, and the runtime bounds trap). An independent review found three
  soundness holes in the first loan model (sibling-field overwrite, composite
  and `->` escapes, incompatible source/slot path comparisons); five failing
  regression tests now pin the fixes. The example was checked, built and run.
  Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **442 passed,
  0 failed** (the B3 total of 423 plus 17 view tests and 2 runtime tests). The
  C11 backend gained one additive static helper, `loi_view`, and one intrinsic
  lowering; existing helpers, headers and the ABI of existing programs are
  unchanged, so `benchmark/run.py` is not applicable. ASan execution is still
  not claimed in this PRoot container.
- **Previous task (2026-11-20, Block B3):** standard-library linear handles built
  on `guard`. `lib/os.loi` adds `File` with `file_open_read`, `file_open_write`,
  `file_read`, `file_write` and the convention-named `file_release`;
  `lib/net.loi` adds `TcpStream` with `tcp_stream_connect`, `tcp_stream_send`,
  `tcp_stream_recv` and `tcp_stream_release`. Both release flows are `open`, so
  a `guard` in an importing module can call them. The POSIX details stay in
  `lib/os_shim.c` and `lib/net_shim.c`; descriptors are `i32`, so no foreign
  pointer enters Loi. The module resolver now checks the synthesized release
  name against `open`/`sealed`, closing the B1 gap for compiler-inserted calls.
  Rules are in `SINTAXE.md` §14, `LOI_SPEC.md` §5.6, `DECISIONS.md`,
  `FRICTION.md` and `website/docs.html`; runnable example:
  `examples/handles.loi`.
- **Verification for Block B3:** 5 tests in `tests/handles.rs` cover file write
  plus release, file read inside `protect`, stream release on return, rejection
  of a raw descriptor and rejection of an explicit release; 1 new test in
  `tests/modules.rs` reject sealed release flows from another module for call
  and shape-literal initializers. Before implementation 4 handle tests failed
  with unknown functions and the sealed release tests were accepted. The example
  was checked, built with
  `--c-source lib/os_shim.c` and run, printing `read 23 bytes`. Final gate
  command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **423 passed,
  0 failed** (the B2 total of 416 plus 5 handle tests and 2 module tests). New C
  symbols live only in `lib/os_shim.c`; generated C, headers, runtime helpers
  and the ABI of existing programs are unchanged, so `benchmark/run.py` is not
  applicable. No ASan execution is claimed in this PRoot container.
- **Previous task (2026-11-20, Block B2):** linear non-memory resources with
  `guard`: convention-named `<shape>_release(<-> value: Shape)` flows called on
  normal exit, `=>`, `break` and `continue` with LIFO unwinding and before
  `protect` restore; explicit release, moves and returns rejected. 12 tests in
  `tests/guard.rs`, gate exited 0 with **416 passed, 0 failed**; see git history
  for the full breakdown.
- **Previous task (2026-11-20, Block B1):** real modules/namespaces. Each source
  file is a module; `import "..."` binds its file stem or an explicit `as` alias,
  and `module::item` works in value, type and constructor positions. A new driver
  loader (`src/main.rs`) tracks modules by canonical path, detects cycles,
  deduplicates diamonds and resolves references through `src/modules.rs` before
  monomorphization. At the time, qualified names were rewritten to plain names;
  module-qualified mangling for duplicated names was added later. `open`/`sealed` are now enforced on
  cross-module references, qualified or not; a module may always use its own
  sealed items. The stdlib files that expose public flows without `open`
  (`ctype`, `math`, `net`, `os`, `random`) and generated bindings now declare
  them `open`; cross-module test wrappers were updated too. Declaration names
  were unique per compilation unit at this point; the limitation was removed by
  module-qualified mangling. Rules are in
  `SINTAXE.md` §13, `LOI_SPEC.md` §4.8, `DECISIONS.md`, `FRICTION.md` and
  `website/docs.html`.
- **Verification for Block B1:** 12 new integration tests in `tests/modules.rs`
  cover qualified/unqualified open and sealed access, aliases, file-stem names,
  qualified types and constructors, unknown modules/items, duplicate aliases,
  cycles, diamond dedup and a two-file build that links and returns 42. Three
  frontend tests cover `::` lexing, alias parsing and formatting; one bindgen
  test now requires `open` bindings. Before implementation the parser/tests did
  not compile; during implementation sealed cross-module stdlib and fixture
  calls were rejected until their public items became `open`. Final gate
  command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **404 passed,
  0 failed** (lib 22 + main 6 + bindgen 1 + checker 147 + cli 42 + codegen 61 +
  const_values 5 + errors 3 + header 4 + lexer 9 + lib-integration 37 + modules 12
  + msrv 1 + order 6 + parser 29 + pretty 19). No generated C, header, runtime
  helper or ABI changed; qualified names are resolved before the monomorphizer,
  so `benchmark/run.py` is not applicable.
- **Previous task (2026-11-19, Block A5):** generic type parameters can declare
  `T: Clone + Copy + Eq` bounds on shapes and flows. `ast::TypeParam` carries a
  list of `TypeBound`; the parser accepts the three constraint names and rejects
  unknown ones, and the pretty-printer round-trips them. Bounds are validated in
  the monomorphizer when concrete arguments are known, pointing at the call site
  or type use with source spelling (`Vec<i32>`), while collision diagnostics keep
  mangled names. `Clone` is satisfied by every type, `Copy` by numbers/bool/
  marks/opaques/`cstr`, and `Eq` by numbers/bool/marks/`[u8]`; no `Ord` was added.
  Library flows now state their real contracts: `vec_find`/`slice_find` use
  `T: Eq`, copying flows use `T: Clone`, math helpers use `T: Copy`. Rules are in
  `SINTAXE.md` §5.4, `LOI_SPEC.md` §5.1a, `FRICTION.md`, `DECISIONS.md` and
  `website/docs.html`.
- **Verification for Block A5:** 14 new tests cover parsing (positive and unknown
  bound), formatting, checker acceptance/rejection at call sites and type uses,
  preserved mangling plus strict C11/sanitizer compile-only execution, and the
  `vec_find` library contract; before implementation the parser and checker tests
  failed to compile or reported unbounded comparisons. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **389 passed,
  0 failed** (lib 22 + main 6 + bindgen 1 + checker 147 + cli 42 + codegen 61 +
  const_values 5 + errors 3 + header 4 + lexer 8 + lib-integration 37 + msrv 1 +
  order 6 + parser 28 + pretty 18). The codegen test runs the generated program
  (stdout `1`) and only claims sanitizer compile-only in this PRoot container.
  Bounds do not change mangling, generated C, headers or the ABI: a bounded
  `find<T: Clone + Eq>` still emits `find__i32`, and headers expose only concrete
  instantiations. No runtime helper signature changed, and generated hot loops
  are unchanged, so `benchmark/run.py` is not applicable.
- **Previous task (2026-11-19, Block A4):** empty slice literals (`[T:]`) can
  escape their block because they emit `{ NULL, 0 }`; non-empty literals stay
  block-confined. Slice member writes use the real `LoiSlice.len` field, and
  `vec_new<T>` no longer allocates while `vec_push<T>` reserves at least one
  slot. Gate exited 0 with **375 passed, 0 failed**; see git history for the full
  test breakdown. Rules remain documented in `SINTAXE.md` §12, `LOI_SPEC.md` §4,
  `FRICTION.md`, `DECISIONS.md` and `website/docs.html`.
- **Previous task (2026-11-19, partial Block A3):** a partially consumed composite can
  be passed to a read-only flow when that flow only reads live fields. The
  checker records, per `<-` parameter, the root-relative paths a flow reads
  (`FunctionScan::direct_reads` plus `collect_direct_reads`), forwards whole
  `<-` arguments from the callee's paths through a fixpoint (`param_reads`), and
  treats unknown/external callees as whole readers. At a call site a place with
  consumed paths is checked against the callee's effective read paths instead
  of being rejected by the whole-place rule; overlapping paths still report a
  use of the moved value. Five checker tests (three positive, two negative)
  and one codegen test cover the feature, including the realistic
  `vec_len(v)`-after-`give(v.data[0])` case that prints `27` and exits 0. The
  read view was lexical to the call: returning or storing a `<-`-borrowed value
  stayed rejected, and returning a borrowed slice view was a separate limitation,
  now resolved by the 2026-11-20 real-view work above. Rules are documented in
  `SINTAXE.md` §11, `LOI_SPEC.md` §4.2–4.4,
  `FRICTION.md`, `DECISIONS.md` and `website/docs.html`.
- **Verification for Block A3:** the three positive checker
  tests failed with `use of moved value` before the change; the two negative
  tests and the codegen test passed only after the implementation. Final gate
  command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **366 passed,
  0 failed** (lib 22 + main 6 + bindgen 1 + checker 133 + cli 42 + codegen 58 +
  const_values 5 + errors 3 + header 4 + lexer 8 + lib-integration 34 + msrv 1 +
  order 6 + parser 26 + pretty 17). The new codegen test compiles the generated
  C under strict C11 and with sanitizers compile-only, then runs it; no ASan
  execution is claimed in this PRoot container. No generated-C ABI or runtime
  helper signature changed, and the benchmarked clone programs do not exercise
  read views, so `benchmark/run.py` is not applicable to this compiler-only
  change.
- **Previous task (2026-11-19, Block A2):** `clone` is now a recursive deep copy.
  When a value contains slices, every slice gets fresh arena storage, nested
  shapes are cloned recursively, and the source stays live and unchanged;
  slice-free types keep the bit-wise copy. Because it allocates, a deep clone
  requires an active `protect` region (directly or through a flow); clones
  outside `protect` and results escaping the region are compile errors, and a
  flow that deep-clones propagates the allocation effect to its callers via a
  `clone_allocators` fixpoint. The checker resolves place/return types in the
  effect prepass and rewrites `collect_value_regions` so clone results carry the
  active region instead of the source's taints. The C backend emits on-demand
  type-directed `static inline` clone helpers with forward declarations:
  `memcpy` for slices of scalar elements, a helper per element for slice-of-
  composite, and bit-copy for types without slices. Nine checker tests (the old
  slice-clone rejection test inverted), five codegen tests (strict C11 +
  sanitizer compile-only + run) and one library test (`Vec<[u8]>`) cover the
  feature; a `Vec<[u8]>` probe prints `9798` and exits 0. Rules are documented
  in `SINTAXE.md` §11/§14, `LOI_SPEC.md` §4.3/§5.7, `website/docs.html`,
  `FRICTION.md` (slice-containing element and map-key restrictions lifted) and
  `DECISIONS.md`.
- **Previous task (2026-11-19, Block A1):** `give` on a field or element now
  consumes only that ownership path when the sub-place is composite. Scalar
  sub-places are copied without killing the owner; whole-variable `give` keeps
  its existing transfer rule. A runtime index in a consumed path is a wildcard
  over all elements, and cross-flow effect summaries are path-sensitive, so a
  callee giving `p.left` kills only `p.left` at the call site. `checker.rs` now
  parses ownership paths into components, resolves path types from callee
  signatures and composes consumed paths through calls. Six new tests cover
  field moves, scalar fields/elements, runtime-index swaps, cross-flow paths,
  wildcard soundness and generated C11; the swap prints `321` when compiled and
  run. Rules are documented in `SINTAXE.md` §11, `LOI_SPEC.md` §4.3 and
  `website/docs.html`; design reasons are in `DECISIONS.md` and the remaining
  library friction is in `FRICTION.md`.
- **Previous task (2026-11-19, review follow-up):** addressed both reported
  review problems. `tests/order.rs` now probes clang with `has_tool --version`,
  honors `LOI_TEST_CLANG`, treats the Termux path as only one candidate, and
  prints a skip message instead of failing when no clang exists. The recursive
  `comptime` global error now points at `LIMIT` (`7:1`) with a secondary label
  on the recursive call (`4:17`), while the first diagnostic stays at `4:17`.
- **Previous task (2026-11-19, item 1):** Loi now defines strict left-to-right
  evaluation rather than rejecting ambiguous programs. The C backend sequences
  operator operands, call/intrinsic arguments, literal elements, assignment
  target before value, index base before index and range start before end with
  named temporaries joined by the comma operator; even a plain read is captured
  when a sibling operand has effects. `and`/`or` stay native C `&&`/`||` so the
  right operand remains lazy. Compound assignment snapshots the target's old
  value before an effectful right operand. `else if` now emits braces because a
  condition may declare sequencing temporaries. Plain lvalue out-arguments and
  pure range starts stay inline to avoid needless temporaries. The repro
  `xs[first()] = second()` prints `5007` under both GCC and clang; 320 generated
  differential programs match a left-to-right oracle. Rules are documented in
  `SINTAXE.md` ("Evaluation order"), `LOI_SPEC.md` §4.11 and `website/docs.html`.
- **Previous task (2026-11-19, item 4):** mark variants and choice constructors
  now work in global, shape-default, slice and `comptime` constant contexts.
  `ConstValue` carries marks and choices; the evaluator handles `is`, equality,
  payload reads and coercion, and preserves step/depth limits. Generated C uses
  static mark macros and nested choice tag/payload initializers under strict
  C11. Forbidden calls now report the exact offending subexpression span.
  New tests cover runtime/comptime agreement, generated initializers, exact
  diagnostics and two negative corpus files; the recursion case is handled by
  the review follow-up above instead of moving both errors to the same span.
- **Previous task (2026-11-19, item 2):** emitted headers were made
  self-contained and linkable with filename guards, shared guarded type
  definitions and `extern "C"` prototypes.
- **Previous task (2026-11-18, item 3):** declared and verified Rust 1.88 as the
  MSRV. Replaced the single `if let` guard with a nested check; 1.85 and 1.87
  still fail on stable let-chains, while 1.88 builds and passes the suite. Added
  a manifest regression test, `tools/check_msrv.sh`, and requirements lines in
  `README.md`, `SINTAXE.md`, `LOI_SPEC.md` and `website/docs.html`.
- **Current task (2026-11-18):** fixed the five reported library/runtime issues.
  `main` is now the only allocation entry point for the checker, so documented
  library flows and shapes are `open` and appear in generated C headers while
  internal helpers remain `sealed`; Loi call sites are still checked. Random
  zero seeds use the named nonzero `RNG_FALLBACK_SEED`. `crc32_extend` chains
  finalized CRC values zlib-style. `map_*_get_or` and `map_*_lookup` return a
  default or a `found` status in one scan. `vec_reserve` is exported and the
  docs explain pre-sizing because the arena retains older growth blocks.
- **Verification for Block A2:** the nine new checker tests
  and five new codegen tests were written first; the inverted
  `clone_of_slice_containing_shape_is_allowed` failed against the old rejection
  and the rest failed at check time. After the implementation all new tests
  pass, including the `Vec<[u8]>` library test. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **360 passed,
  0 failed** (lib 22 + main 6 + bindgen 1 + checker 128 + cli 42 + codegen 57 +
  const_values 5 + errors 3 + header 4 + lexer 8 + lib-integration 34 + msrv 1 +
  order 6 + parser 26 + pretty 17). The clone tests compile the generated C
  under strict C11 and with sanitizers compile-only, and run it; no ASan
  execution is claimed in this PRoot container. Generated C changes only for
  clone of slice-containing values: new `static inline loi_clone_<type>` helpers
  call `loi_alloc_slice` in the active arena; no existing ABI or runtime helper
  signature changed. `benchmark/run.py` (min of 9, pinned CPU 6, GCC 15.2.0)
  before/after Loi/C ratios: hash 1.009→0.995, crc32 1.115→1.114,
  sort 1.325→1.335, int_map 1.234→1.182, str_map 0.798→0.839,
  random 0.990→1.002. The benchmarked programs clone scalar values only, so
  their emitted C is unchanged and the differences are noise.
- **Verification for Block A1:** the five A1 checker tests and the
  codegen swap test were written first and failed with the original whole-root
  consumption (`3 failed` in `checker`; codegen blocked at check time). After
  the implementation all new tests pass. Final gate command:
  `export PATH="/root/.cargo/bin:$PATH"; export RUSTFLAGS="-C linker=/usr/bin/gcc"; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check`.
  All five checks exited 0; `cargo test --all-targets` reported **346 passed,
  0 failed** (lib 22 + main 6 + bindgen 1 + checker 120 + cli 42 + codegen 52 +
  const_values 5 + errors 3 + header 4 + lexer 8 + lib-integration 33 + msrv 1 +
  order 6 + parser 26 + pretty 17). The swap test also compiles the generated C
  under strict C11 and with sanitizers compile-only; no ASan execution is
  claimed in this PRoot container. There is no generated-C ABI or runtime
  change. `benchmark/run.py` (min of 9, pinned CPU 6) before/after ratios:
  hash 1.025→1.009, crc32 1.113→1.115, sort 1.379→1.325, int_map 1.172→1.234,
  str_map 0.808→0.798, random 1.013→0.990. The compiler-only change has no
  expected runtime effect; the differences are noise.
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
- **Not done:** growable maps, the IR/ASM backends and browser integration tests.

## Component Status

| Component | Status | Notes |
|---|---|---|
| Lexer | Working | Canonical Flow Syntax only; strings with decoded escapes; decimal/hex/binary integers; floats; `<-`/`->`/`<->`/`=>` glyphs; bitwise/shift/compound tokens; `guard` keyword; `//` comments collected for `fmt` |
| AST / parser | Working | Shapes and generic shapes, flows and generic flows, `extern flow ... as`, `opaque`, imports, globals, slice/shape literals, `give`/`clone`/`alloc`, `protect`, `guard`, loops, bitwise/shift precedence, `comptime flow`/`comptime expr`, explicit type arguments; module resolution rewrites shared names before monomorphization |
| Pretty-printer | Working | Emits canonical Flow Syntax including generics and `comptime`; idempotent on all canonical files; `loi fmt` preserves `//` comments (`format_program` remains comment-free) |
| Type checker | Working | Types, literals, directions, calls, globals, intrinsics, `opaque`/`cstr`, constant folding, shape-cycle detection, bitwise/shift rules, structural `Eq`/`Ord` comparisons, I/O intrinsic signatures, allocation effects (including deep-clone allocators), `<-` read-view summaries and linear-resource `guard` validation (release-flow signature, no explicit release/move/return); monomorphization feeds concrete declarations into the checker and diagnostics show source generic spellings; module-resolved names keep same-module duplicates as errors and reject ambiguous unqualified references |
| Compile-time evaluation | Working | `src/const_eval.rs`: pure `comptime flow` interpreter with step/depth limits, constant globals and shape defaults, `comptime expr`, purity diagnostics |
| Diagnostics | Working | `src/diagnostic.rs`: human output with full source underlines (multi-line spans get `...`), secondary labels and an error summary; `short` keeps `path:line:column: message`; `json` emits structured objects with `end_line`/`end_column`; ANSI colors honor `NO_COLOR`/`CLICOLOR_FORCE`; unknown names get namespace-aware edit-distance suggestions; parser spans carry exact token ends and multi-line end lines with a line-scan fallback |
| Ownership | Working | Give liveness, path-sensitive partial moves, field-sensitive lexical read views, definite assignment/return, exclusive `->`/`<->`, cross-flow consumption, `protect` taints and lexical-block escape analysis for non-empty slice literals and deep-clone results (empty literals are untainted), writable slice length, `read_file`/`read_stdin`/`read_line`, linear-resource `guard` scoping; gives from globals are rejected and move state is per function body |
| C backend | Working | One C11 translation unit, ordered shape layouts, checked arithmetic, bitwise/shift helpers, growable arena, recursive on-demand deep-clone helpers, on-demand structural `loi_eq_*`/`loi_ord_*` helpers, module-qualified symbols for duplicated names with collision validation, bounds checks, defaults, headers, source maps, multi-file objects, statics, file/stdin I/O, numeric printing and implicit `guard` releases on normal/return/`break`/`continue` paths with LIFO unwinding |
| C interop | Working | `opaque` → `void *`, `cstr` → `const char *`, fixed-ABI `callback` → `LoiCallback` from `callback(flow)`, bodyless `extern flow`, `--c-source`, `--cc-flag`; string literals coerce to `cstr` |
| Library | Working | `lib/std.loi` (scalar/string helpers), `lib/math.loi` (generic and `comptime`), `lib/ctype.loi` (pure ASCII helpers), `lib/vec.loi` (cloneable vectors with `Eq` search and `Ord` sort), `lib/slice.loi` (generic slice helpers), `lib/option.loi`/`lib/result.loi`, `lib/os.loi`/`lib/net.loi` (thin POSIX bindings with C shims and linear `File`/`TcpStream` handles), `lib/stdio.loi` (deterministic byte I/O with a C shim); prefixed names, explicitly imported |
| Binding generator | Working | `tools/bindgen/bindgen.py` emits `opaque` handles, safe `shape`s, typed constants, `extern flow`s, a `memcpy` shim with `_Static_assert` checks and a skip report; C `const T *` parameters bind as borrowed `[T]` slices and mutable `char *` parameters as `<-> [u8]`, with a shared-layout `LoiSlice` shim |
| Vendored bindings | Working (syntax-verified) | `vendor/sdl2/` (SDL2 2.32.10) and `vendor/raylib/` (raylib 5.5), each with generated `.loi`, shim, skip report and README |
| IR | Draft | `src/ir.rs`; `--emit-ir` writes text. Globals and some literals are not lowered |
| Assembly | Disabled draft | `src/asm.rs` cannot generate runnable code; the CLI rejects `--emit-asm` with exit 2 |
| CLI | Working | `lex`/`parse`/`fmt`/`check`/`gen`/`build`/`clean`, incremental stamps, profiles, source maps, `--arena-size`, `--c-source`, `--error-format`, `--color` |
| Website / playground | Working (local only) | Static site plus `website/server.mjs` driving the real compiler with `--error-format=short --color=never`; `127.0.0.1` by default with source/output/time limits |

## Verification

The most recent gate result is recorded in **Current Status** above; the
entries below are earlier gates kept for reference.

Last verified 2026-11-20 for Block B3 with:

```sh
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
python3 tools/sync_examples.py --check
git diff --check
```

- All five checks exited 0. `cargo test --all-targets` reported **423 passed,
  0 failed** (the B2 total of 416 plus 5 handle tests and 2 module tests). New C
  symbols live only in `lib/os_shim.c`; generated C, headers, runtime helpers
  and the ABI of existing programs are unchanged, so `benchmark/run.py` is not
  applicable. No ASan execution is claimed in this PRoot container.
- `examples/handles.loi` was checked, built with `--c-source lib/os_shim.c` and
  run (`read 23 bytes`), and mirrored into `website/examples/` with
  `python3 tools/sync_examples.py`.

Previously verified 2026-11-20 for Block B2 with the same gate command:

- All five checks exited 0. `cargo test --all-targets` reported **416 passed,
  0 failed** (the B1 total of 404 plus 12 guard tests: lexer 1, checker 10 and a
  compiled C harness). B2 adds one implicit release call per guarded exit path;
  all pre-existing generated C, headers, runtime helpers and ABI are unchanged,
  so `benchmark/run.py` is not applicable.
- `examples/guard_resources.loi` was checked, built and run (`connection 7
  exchanged`, then `1`), and mirrored into `website/examples/`.

Previously verified 2026-11-20 for Block B1 with the same gate command:

- All five checks exited 0. `cargo test --all-targets` reported **404 passed,
  0 failed** (lib 22, main 6, bindgen 1, checker 147, cli 42, codegen 61,
  const_values 5, errors 3, header 4, lexer 9, lib-integration 37, modules 12,
  msrv 1, order 6, parser 29, pretty 19). B1 changes no generated C, header,
  runtime helper or ABI: qualified names are resolved before monomorphization,
  so `benchmark/run.py` is not applicable.
- `LOI_TEST_CLANG=/definitely/not/clang cargo test --test order -- --nocapture`
  ran all 6 order tests with no failure.

Previously verified 2026-11-19 for Block A5 with the same gate command:

- All five checks exited 0. `cargo test --all-targets` reported **389 passed,
  0 failed** (lib 22, main 6, bindgen 1, checker 147, cli 42, codegen 61,
  const_values 5, errors 3, header 4, lexer 8, lib-integration 37, msrv 1,
  order 6, parser 28, pretty 18). A5 adds no benchmark: bounds are static checks
  and emit the same C as before.
- `LOI_TEST_CLANG=/definitely/not/clang cargo test --test order -- --nocapture`
  ran all 6 order tests and printed one skip message per test with no failure.
  With clang present, all 6 ran differentially against GCC 15.2.0 (`cc`) and
  clang 21.1.8.
- `benchmark/run.py --output /tmp/opencode/bench_{a1_after,a2_after}.json`
  (minimum of 9 runs, pinned CPU 6, GCC 15.2.0, identical checksums) measured
  these Loi/C ratios:

  | benchmark | before | after |
  |---|---:|---:|
  | hash | 1.009 | 0.995 |
  | crc32 | 1.115 | 1.114 |
  | sort | 1.325 | 1.335 |
  | int_map | 1.234 | 1.182 |
  | str_map | 0.798 | 0.839 |
  | random | 0.990 | 1.002 |

  The benchmarks clone scalar values only, so their generated C is unchanged;
  the ratios are recorded as measured rather than attributed to the patch.

Previously verified 2026-11-18 after the canonical example expansion with:

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
- Cross-flow consumption is path-sensitive, and read-view summaries now let a
  partially consumed composite be passed to a `<-` flow whose read paths avoid
  the consumed ones. A flow whose reads cannot be summarized (an unknown or
  external callee) is treated as reading the whole value, so it still rejects
  partial moves. Returning a slice view that points into a `<-` parameter is not
  supported: that needs return-alias summaries and borrow-region tracking.
- `protect` taints follow locals, fields, ownership transfers, shape literals,
  `->` arguments and returns, and are conservatively forwarded from consumed
  `<->` arguments to every slice-containing output or composite result. They
  are not tracked through callees that only mutate `<->` storage in place.
- The entry-point heuristic treats `main` and every uncalled non-`sealed`
  flow as an entry for the allocation check; an uncalled `sealed` helper may
  rely on a caller's `protect`.
- Non-empty slice literals are rejected when they escape their block through a
  return, a variable declared outside the block or a shape that escapes. Empty
  literals (`[T:]`) have no backing storage and may escape. Writing `xs.len` is
  unchecked against capacity: the checker does not track slice capacity, so a
  length larger than the allocated buffer is the programmer's responsibility.
  This is lexical escape analysis, not a full interprocedural lifetime system.
- `clone` of a slice-containing value is a deep copy and therefore allocates;
  it is rejected outside every `protect` region, and its result may not escape
  that region. Slice-free composites still copy bit-wise without allocation.
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
- A `mark` variant or `choice` constructor passed directly as a call argument
  (for example `same<Color>(Color.Red, Color.Red)`) is misreported as
  `unknown variable 'Color'`: `<-` argument checking treats place-like
  expressions as lvalues. Binding it first (`red <- Color.Red`) works. This is
  a pre-existing checker bug, not part of Block A5; A5 tests use local binds.

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
- `callback` is an FFI-only function pointer with one fixed signature, built by
  `callback(flow)` from an exact-signature Loi flow. It is copyable and storable
  but not callable, comparable or dereferenceable from Loi; C uses the flow
  symbol directly. C function pointers with other signatures remain unsupported.
- Globals may use any compile-time constant: scalars, strings, slices and
  nested shapes. The backend folds them once and emits strict C11 static
  initializers: brace/designated initializers for shapes and named `static`
  arrays for slice data, with no compound literals.
- `==`/`!=` and the ordering operators are structural for the types listed in
  `LOI_SPEC.md` §4.8; `Ord` is a fourth generic bound.
- Imports bind modules with `module::item` qualified references, and `open`/
  `sealed` is enforced across module boundaries (§13). Names repeated in several
  modules are mangled into distinct C symbols; unique names keep their ABI.
- I/O intrinsics are limited to `print`, `eprint`, `panic`, numeric/`printf`
  formatting and whole-file / whole-stdin reads and writes. `printf` needs a
  literal format string and rejects dynamic `*` width/precision; user-declared
  variadic flows are not supported. `read_file`, `read_stdin` and `read_line`
  allocate in the current `protect` region and cannot escape it. There is no
  directory or seek API. `lib/os.loi` and `lib/net.loi` wrap POSIX calls through
  shims (process/environment queries, file handles and a minimal TCP client).
  The shims are POSIX-only and expose descriptors as `i32`; `File` and
  `TcpStream` are linear resources whose `guard` release flows close them.
  `lib/stdio.loi` reads exact byte counts and flushes stdout through C stdio;
  it must not be mixed with the shims' raw descriptors on the same stream.
  User-declared resources remain supported (`examples/guard_resources.loi`),
  and the stdlib handle example is `examples/handles.loi`.
- Library limits: `lib/ctype.loi` classifies ASCII bytes only (Loi strings are
  UTF-8 bytes), and `math_factorial`/`math_pow_u64` follow unsigned Loi
  arithmetic, so large results wrap at 64 bits.
- `comptime` is implemented for pure flows, expressions, globals and shape
  defaults. Freshness, spatial tiers and ordering blocks are not implemented.
  `compare_values` still requires numeric `ConstValue`s, so ordering a shape,
  mark, `cstr`, opaque handle or slice inside a `comptime` expression or global
  initializer reports `comparison requires numeric compile-time values`;
  runtime comparisons are structural.
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
- The binding generator is conservative. C function pointers, variadics, unions,
  bitfields, inline arrays, pointer-to-pointer, wide strings, pointer results and
  pointers to unsafe records are skipped and reported. Read-only `const T *` and
  mutable `char *` parameters are supported as slices; other callback signatures
  than the fixed `callback` ABI need a hand-written `extern flow`.
- Full SDL2/raylib link-and-run needs their runtime dependency closure, which
  this development environment does not provide.
- `guard` derives the release flow name from the shape (`Connection` →
  `connection_release`), so two shapes with the same snake-case name cannot both
  be resources in one compilation unit. A release flow called by a `guard` in
  another module must be `open`; the module resolver rejects a sealed one.
- The playground executes arbitrary Loi with the server process's privileges.
  It is a local development tool, not a sandbox, and must not be exposed
  publicly without real isolation.

## Open Design Questions

Language-level questions are resolved in `development/LOI_SPEC.md` section 5.
Remaining implementation or secondary choices:

- Decide whether resources need an explicit `resource` declaration instead of
  the `<shape>_release` naming convention. B2/B3 keep the convention to avoid a
  second declaration form; the stdlib `File` and `TcpStream` handles use it.
- User-declared variadics, arbitrary C unions, fixed arrays and C function
  pointers whose target is not an all-`<-` Loi flow remain shim-only. Typed
  `callback(T, ...) => R` now covers Loi flows; `printf` is a variadic intrinsic
  and Loi `choice` covers tagged unions with safe payloads.
- Define the byte-slice/`cstr` conversion surface beyond string literals.
- Extend escape analysis beyond lexical blocks if programs need block-local
  slices to outlive their block without `alloc`.
- Revisit `<-` composite passing if concurrency or observable aliasing is ever
  introduced; today the C backend passes by value and the checker enforces
  read-only use.
- Finish or remove the IR/ASM drafts. Until then `--emit-ir` stays diagnostic
  and `--emit-asm` stays rejected.
- Add an FFI-safe string-array type so `process_spawn_args` can call `execvp`
  directly instead of building a `/bin/sh -c` command; the current shell path
  adds one process per spawn and requires careful quoting in pure Loi.
- `--error-format json` still emits no `[]` for a clean check, so
  `lib/lsp.loi` treats an empty snapshot as cleared markers.
- Define an LLVM ABI for composite and nested-slice elements, callback values
  and `comptime` before extending the opt-in backend further. Scalar slices
  and `protect` regions now have a C-compatible ABI; C11 remains the permanent
  semantic oracle and no language syntax is involved.

## Next Steps

1. Block A2: done — recursive deep `clone` with `protect`-scoped allocation;
   `Vec<[u8]>` works and the library docs no longer require slice-free `T`.
2. Block A3: done — read-path summaries let a partially consumed composite be
   passed to a read-only flow that only touches live fields, and the real-view
   work adds zero-copy borrowed slice views with lexical loans and safe returns.
3. Block A4: done — empty slice literals escape freely, `.len` is writable, and
   `vec_new<T>` no longer needs `protect`. Non-empty literals stay block-local.
4. Block A5: done — `Clone`, `Copy` and `Eq` bounds let library flows state
   their element requirements; equality search is no longer numeric-only.
   `Ord` and structural comparisons are now implemented as well; see item 10.
5. Blocks A, B1, B2 and B3 are done: generics/ownership, real modules, `guard`
   resources and stdlib `File`/`TcpStream` handles.
6. Block A6: done — official `Option<T>` and `Result<T, E>` are pure-Loi generic
   shapes with guarded readers; generic choices remain out of scope.
7. Block B4: done — C `const T *` parameters bind as borrowed slices through the
   generated shim.
8. Block B5: done — C mutable `char *` parameters bind as `<-> [u8]` slices, with
   the shim matching the compiler's `LoiSlice *` ABI.
9. Block B6: done — fixed-ABI `callback` values pass exact-signature Loi flows
   to C directly.
10. Structural `Eq`/`Ord`: done — one shared structural rule for `==`, `!=`,
    `<`, `<=`, `>` and `>=`, an `Ord` bound, and on-demand C helpers.
11. Module-qualified mangling: done — duplicated declaration names get distinct
    C symbols; unique names keep their ABI.
12. Typed callbacks: done — `callback(T, ...) => R` infers a signature from an
    all-`<-` Loi flow and passes the symbol directly to C.
13. Opt-in LLVM backend: done for scalars and scalar slices with `clang`
    compiled IR, C11 output/diagnostic parity and a C-compatible region ABI.
    Later: the IR/ASM drafts and LLVM ABIs for composites, callbacks and
    `comptime`; keep the C11 backend as the production oracle.
14. Keep the docs in sync: `SINTAXE.md` is canonical, `website/docs.html`
    mirrors it, and `python3 tools/sync_examples.py --check` guards the example
    mirror.
15. Block L1: done — deterministic stdio byte I/O, a framed LSP-style example
    and three failing-first tests.
16. Block L2: done — a pure-Loi JSON parser/serializer with a node table, a
    decoded-string arena and six failing-first tests.
17. Block L3: done — JSON-RPC 2.0 over stdio and a working initialize/shutdown
    server, with five failing-first tests.
18. Block L4: done — real compiler diagnostics published as LSP notifications,
    with four failing-first tests. The temporary `diagnosticsPath` bridge was
    removed in L5.
19. Block L5: done — process spawning with captured stdout/stderr and exit
    status, `os_env` for runtime byte strings, an LSP server that checks the
    document itself, and six failing-first tests. Next: an FFI-safe argv type
    would let `process_spawn_args` avoid the extra shell process.
