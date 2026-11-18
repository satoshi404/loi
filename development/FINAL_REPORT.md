# Final standard-library report — 2026-11-18

This report covers Phase 0 through Phase 6. Every number below was measured in
this container; raw benchmark timings are in `benchmark/results.json`, and
blocked-design details are in `development/FRICTION.md`.

## Environment

| Item | Measured value |
|---|---|
| `loi` | `loi 0.1.0` |
| `rustc` | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` |
| `cargo` | `cargo 1.99.1 (5f94df478 2026-08-27)` |
| C compiler | `gcc (Ubuntu 15.2.0-16ubuntu1) 15.2.0` |
| Architecture | `aarch64`, 8 vCPUs |
| CPU models | six `Cortex-A55` cores and two `Cortex-A75` cores (`lscpu`) |
| Benchmark pin | `taskset -c 6` for every timed run |
| Compiler flags | Loi `--release --arena-size 16777216`; C `-std=c11 -O2` |

## Full verification gate

Exact command:

```sh
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check &&
cargo clippy --all-targets -- -D warnings &&
cargo test --all-targets &&
python3 tools/sync_examples.py --check &&
git diff --check
```

Result: every stage exited 0. `cargo test --all-targets` reported **294
passed, 0 failed, 0 ignored, 0 measured, 0 filtered out**.

| Binary | Passed | Failed |
|---|---:|---:|
| `unittests src/lib.rs` | 22 | 0 |
| `unittests src/main.rs` | 6 | 0 |
| `tests/bindgen.rs` | 1 | 0 |
| `tests/checker.rs` | 102 | 0 |
| `tests/cli.rs` | 42 | 0 |
| `tests/codegen.rs` | 43 | 0 |
| `tests/errors.rs` | 2 | 0 |
| `tests/lexer.rs` | 6 | 0 |
| `tests/lib.rs` | 32 | 0 |
| `tests/parser.rs` | 23 | 0 |
| `tests/pretty.rs` | 15 | 0 |
| **Total** | **294** | **0** |

The committed library phases were `a5c427a` (reconnaissance), `2fda361`
(`vec`/`slice`), `2aecdcf` (strings/hashing), `a8b97ba` (CRC-32), `bd349f0`
(maps), and `538792f` (random).

## Library tests and emitted-C checks

Of the 32 `tests/lib.rs` cases, 30 build and run at least one Loi program and
check its output or exact error. The other two
(`every_library_source_checks_cleanly` and
`library_shims_compile_as_strict_c11`) check library sources and compile the C
shims without running a Loi program. Success cases that call
`assert_ok_with_c_variants` emit C and check it in three modes: strict C11 uses
`-std=c11 -pedantic-errors -Wall -Wextra` and requires empty compiler stderr;
UBSan uses `-fsanitize=undefined -fno-omit-frame-pointer -g` and the binary is
run; ASan+UBSan uses `-fsanitize=address,undefined -fno-omit-frame-pointer -g`
and the binary is compiled but not run in this container. The five remaining
success cases (`math`, `ctype`, `os`, `net` and `all-libraries`) use `assert_ok`,
which checks, builds and runs the Loi program but does not emit C variants.
Runtime-error cases compile the same three C variants without executing them,
then run the Loi binary to check the exact flow-naming message.

| Library | New runnable test cases | Strict C11 | UBSan run | ASan+UBSan build |
|---|---:|---|---|---|
| `lib/vec.loi`, `lib/slice.loi` | 8 | clean | clean, output checked | success, not run |
| `lib/string.loi`, `lib/hash.loi` | 5 | clean | clean, output checked | success, not run |
| `lib/crc32.loi` | 3 | clean | clean, output checked | success, not run |
| `lib/map.loi` | 5 | clean | clean, output checked | success, not run |
| `lib/random.loi` | 4 | clean | clean, output checked | success, not run |

Every new library is also imported twice in one program and imported from two
separate library files. Seeded property tests compare sort/find against a naive
list and C `qsort`, hashes and CRC-32 against independent Rust/C references,
maps against Rust `HashMap`, and random draws/shuffles against a Rust xorshift
reference.

ASan execution is unverified because this PRoot container cannot map the
required shadow memory:

```text
==28452==Error: heap size 40000002000 exceeds max user virtual address 7fffffffff
```

The benchmark programs' emitted C was additionally checked separately: all six
compile with strict C11 and empty stderr, all six UBSan binaries run with the
same output as the release binaries, and all six ASan+UBSan builds succeed but
remain unexecuted for the same shadow-memory reason.

## Benchmarks

Each benchmark compares a Loi library workload with a semantically identical C
loop. Loi and C programs produce the same checksum; `benchmark/run.py` rejects
the measurement on any mismatch. Each executable is built once, correctness is
checked once, and then nine timed runs are made. The table reports the minimum
of those nine runs; `benchmark/results.json` retains all 18 timings per row.

Reproduce with:

```sh
export CC=gcc
python3 benchmark/run.py --output benchmark/results.json
```

| Workload | Operation count | Loi min (s) | C min (s) | Loi / C |
|---|---|---:|---:|---:|
| `hash` | 2048 × 64 KiB FNV-1a bytes (134,217,728 bytes) | 0.612530 | 0.611861 | 1.001 |
| `crc32` | 1024 × 64 KiB reflected-nibble CRC-32 | 0.815954 | 0.736577 | 1.108 |
| `sort` | 16 heapsorts of 65,536 fresh `i32` values | 0.251604 | 0.179833 | 1.399 |
| `int_map` | 128 keys, 200,000 get/set pairs | 0.073027 | 0.061657 | 1.184 |
| `str_map` | 8 byte-string keys, 100,000 get/set pairs | 0.031253 | 0.037120 | 0.842 |
| `random` | 100,000,000 xorshift64* draws | 0.503028 | 0.495317 | 1.016 |

The largest measured slowdown is `sort` at **1.399×**, below the 1.5×
threshold, so no ranked generated-C cause analysis was triggered. The `str_map`
Loi workload was faster than its C reference in this measurement; no claim is
made that this generalizes to other key sets or machines.

## Blocked designs

This list mirrors `development/FRICTION.md`. Each item records the attempted
design, the measured diagnostic where one was produced, the language
requirement, and the workaround that was actually implemented.

### Phase 1 — `lib/vec.loi`

1. **Slice-containing element types cannot be moved or cloned through a
   vector.** An attempt to support `Vec<[u8]>` used `give(...)` while growing
   and pushing, which produced `cannot assign to immutable variable 'first'`
   and two `use of moved value 'xs'` errors. Making callers mutable still left
   the ownership problem: moving an element out of `v.data` consumes the whole
   vector. The clone fallback works for scalars and slice-free composite
   shapes, but rejects a slice-containing composite with `cannot copy [u64]
   implicitly; use give(...) or clone(...)`. Requirement: move one element out
   of an inout composite without consuming the owner, or recursively clone
   slice fields. Workaround: document cloneable `T` only.

2. **Borrowed slice views cannot escape a flow.** Returning `v.data` directly
   produced `cannot return a value borrowed from '<-' parameter 'v'; use
   clone(...)`. Requirement: slice borrow lifetimes or a borrow-safe view type.
   Workaround: `vec_as_slice<T>` allocates a new arena slice and copies the
   live prefix.

3. **A generic slice literal cannot escape into an out-parameter.** `v.data =
   [T:]` inside `vec_new<T>` produced `slice literal storage escapes the block
   that created it; keep it in a variable declared in the same block`. Passing
   a caller-owned literal then failed with `lvalue required as left operand of
   assignment` when the callee tried to write the slice length. Requirement:
   escaping slice literals with an explicit arena/lifetime, or writable slice
   length. Workaround: `vec_new<T>` uses `alloc(..., 1)`, so an empty vector
   has capacity one.

4. **Generic comparisons fail for composite `T`.** Ordering helpers over
   arbitrary `T` produced `operator requires numeric operands, found Bad and
   Bad`. Requirement: trait constraints or `Ord`-style bounds. Workaround:
   document ordering/search/sort flows as numeric-only.

### Phase 2 — `lib/string.loi`, `lib/hash.loi`

5. **Character literals do not exist.** `c: u8 <- 'n'` produced `error:
   unexpected character '''`. Requirement: a `u8` character-literal form with
   escapes. Workaround: byte queries take numeric `u8` values; substring
   queries take one-byte string slices. No library flow was blocked.
   `str_repeat` checks overflow before allocating and aborts with a
   flow-naming message; `hash_*` flows are pure.

### Phase 3 — `lib/crc32.loi`

6. **Comptime flows cannot assign to shape fields.** An uninitialized local
   shape produced `comptime flow bindings require an initializer`; giving it a
   shape literal and assigning fields produced `comptime flows can only assign
   to local bindings`. Requirement: field assignment on local composite
   bindings inside a `comptime flow`, or a compile-time builder form.
   Workaround: compute each packed word in a scalar `comptime flow` and build
   the table in a global slice literal.

7. **A 256-entry table cannot be packed into `u64` words.** Packing eight
   `u32` entries with shifts up to 224 bits produced `shift count is out of
   range`. Requirement: a wider integer type or compile-time byte arrays.
   Workaround: a 16-entry reflected nibble table with two `u32` entries per
   `u64`, generated entirely by `comptime` flows and verified against a
   bitwise reference.

### Phase 4 — `lib/map.loi`

8. **Slice keys are stored in a contiguous buffer.** The attempted
   `StrMap<V>` shape used `keys: [[u8]]`; this is the Phase 1
   slice-containing-composite limitation, so no new diagnostic was needed.
   Requirement: recursive clone for slice fields, or moving one element out of
   an inout composite without consuming the owner. Workaround: one flat
   `keys: [u8]` arena buffer plus `starts` and `lengths` arrays. No other
   Phase 4 design was blocked; maps are generic only over values.

### Phase 5 — `lib/random.loi`

9. **No design was blocked.** `Rng` is one `u64` inout state; xorshift64*,
   bounded draws, picking and Fisher–Yates shuffling fit in pure Loi without
   allocation. Two test mistakes were ordinary language rules, not blocked
   designs: an inout argument must be declared `mut`, and two inout calls
   cannot be sequenced inside one expression.

## Unverified items

- ASan+UBSan execution is not verified. The binaries compile successfully but
  cannot run in this PRoot container because ASan's shadow-memory mapping
  exceeds the available user virtual address space.
- UBSan evidence is from executing the instrumented binaries and checking
  their output and stderr; it is not a proof of absence for all inputs.
- Benchmarks measure only this aarch64 container, `gcc 15.2.0`, `loi 0.1.0`,
  `--release`, one pinned core, and the six fixed workloads listed above. No
  other CPU, compiler, optimization level, core count, input distribution, or
  warm/cold cache state was measured.
- Property tests use fixed seeds and finite sample sizes; they are regression
  evidence, not exhaustive proofs.
- `lib/random.loi` was not evaluated for cryptographic security; it is
  explicitly documented as unsuitable for that purpose.
- No generated-C cause ranking was produced because no measured Loi/C ratio
  exceeded 1.5×.
