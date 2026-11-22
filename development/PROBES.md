# Phase 0 — Reconnaissance probes (2026-11-18)

All answers below come from tiny programs executed with the freshly built
`./target/debug/loi` (`loi 0.1.0`). The probe sources live in
`/tmp/opencode/loi-phase0/`; **no library or compiler code was added in this
phase**. Commands use `LOI=/root/loi/target/debug/loi`.

Environment measured with `loi --version`, `rustc --version`, `cargo --version`,
`cc --version`, `uname -m` and `lscpu`:

| Item | Measured value |
|---|---|
| `loi` | `loi 0.1.0` |
| `rustc` | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` |
| `cargo` | `cargo 1.99.1 (5f94df478 2026-08-27)` |
| `cc` | `cc (Ubuntu 15.2.0-16ubuntu1) 15.2.0` |
| Machine | `aarch64`, 8 vCPUs, `Cortex-A55` and `Cortex-A75` model names |
| Date | 2026-11-18 |

`a)` is **not missing**: `& | ^ ~ << >>` all exist for the integer widths, so the
whole-task stop condition does not apply. No item was stopped by a missing
language feature in Phase 0.

---

## a) Bitwise operators and widths

Probe: `a_bitwise.loi` (checks, builds, runs), plus `a_bool.loi`,
`a_float.loi`, `a_shift8.loi`, `a_shift64.loi`, `a_shift_runtime.loi` and
`a_compound.loi` for the rejection/boundary cases.

```sh
$LOI check /tmp/opencode/loi-phase0/a_bitwise.loi
$LOI build /tmp/opencode/loi-phase0/a_bitwise.loi -o /tmp/opencode/loi-phase0/a_bitwise --rebuild
/tmp/opencode/loi-phase0/a_bitwise
```

Measured output:

```text
not u8=15 i8=-113 u16=3855 i16=-28785 u32=252645135 i32=-1886417009 u64=1085102592571150095 i64=-8102099357864587377
and 48 48 12336 12336 808464432 808464432 3472328296227680304 3472328296227680304
or 243 115 62451 29555 4092851187 1936946035 17578661999652631539 8319119876378817395
xor 15 15 3855 3983 252645135 261066639 1085102592571150095 1121272678990188431
shl 224 -32 57824 -7968 3789677024 -522133280 16276538888567251424 -2242545357980376864
shr 120 56 30840 14392 2021161080 943208504 8680820740569200760 4051049678932293688
```

Answer:

- `&`, `|`, `^`, `~`, `<<` and `>>` are accepted on all eight tested integer
  widths: `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`.
- `>>` behaved logically for unsigned values and arithmetically for signed
  values in the measured output (`u8 0xF0 >> 1 = 120`; `i8 0x70 << 1` wrapped to
  `-32`, then `>> 1` gave `-16` as part of the signed output sequence).
- Compound forms tested on `u8`: `&=`, `|=`, `^=`, `<<=`, `>>=`.
  `a_compound.loi` printed `compound=76`.
- `bool` and `f64` are rejected:

```text
error: bitwise operator requires integer operands, found bool and bool
error: bitwise operator requires integer operands, found f64 and f64
```

- Constant out-of-range shift counts are rejected at check time:

```text
error: shift count 8 is out of range for u8 (0..8)
error: shift count 64 is out of range for u64 (0..64)
```

- A non-constant out-of-range shift compiles and traps at runtime. Running
  `a_shift_runtime.loi` printed:

```text
loi: shift count out of range at 6:19
```

No arithmetic emulation is needed.

---

## b) `comptime`: loops, recursion, locals, limits and return shapes

Probe `b_comptime.loi` checked, built and ran:

```text
while=45 for=45 fact=3628800 pair=4,5
```

So a `comptime flow` can use `while`, `for` ranges, local `mut` bindings,
`if`, recursion into another `comptime flow`, and can return scalars and a
shape with scalar fields.

### Slice returns

`b_slice.loi` tried a local bound to a slice literal:

```loi
comptime flow make_slice
    <- x: u64
    => [u64]
{
    values <- [u64: x, x + 1]
    => values
}
```

Both the direct `=> [u64: ...]` form and this local form were rejected with the
same text:

```text
error: cannot return a slice literal: its storage ends with the current block; bind it to a local slice first
```

`b_shape_slice.loi` tried a shape containing a slice and got two errors:

```text
error: cannot return a slice literal: its storage ends with the current block; bind it to a local slice first
error: cannot copy [u64] implicitly; use give(...) or clone(...)
```

Answer: a `comptime flow` can return a scalar or a slice-free shape, but not
`[T]` and not a shape containing `[T]`.

This matters for Phase 3: the CRC32 table cannot be *returned* from a comptime
flow. A global slice literal **can** be read from a comptime flow;
`b_crc_table.loi` checked, built and printed:

```text
crc-step=16777214
```

Its comptime step was
`((crc ^ (byte as u32)) & 0x3 as u32) as u64` indexing a global
`TABLE: [u32] <- [u32: 0, 1, 2, 3]`. That gives the Phase 3 fallback
(`tools/gen_tables.py` plus compile-time checks of selected values) a tested
path.

### Limits

The evaluator source defines a step budget of `1_000_000` and call depth `64`;
both were confirmed with boundary probes.

- `b_step_limit.loi` with a 2,000,000-iteration `while` failed:

```text
error: initializer for global 'N' must be a compile-time constant: comptime evaluation exceeded the step limit
```

  Binary search over the same probe found the largest passing loop bound
  `166664` and the first failing bound `166665`.

- `b_depth_limit.loi` with `comptime recurse(100)` failed:

```text
error: initializer for global 'N' must be a compile-time constant: comptime call depth exceeded; is the flow recursive?
```

  Binary search found the largest passing recursion argument `63` and the
  first failing argument `64`.

---

## c) Ownership into a flow, including generic flows

Probes: `c_ownership.loi`, `c_generic_give.loi`, `c_in_read.loi` and
`c_give_kills.loi`.

`give` is an **expression at the call site**, not a parameter direction. Exact
syntax that checked, built and ran:

```loi
first <- Item { value: 7 }
a <- take_item(give(first))          // <- item: Item

second <- Item { value: 9 }
consume<Item>(give(second))          // generic: flow consume<T> <- item: T
```

`c_ownership.loi` printed `ownership 7 1 11`; `c_generic_give.loi` printed
`generic give ok`. A generic flow therefore accepts `give` semantics with
explicit type arguments.

A composite passed to a `<-` parameter **without** `give` is a read-only
borrow; the source stays live. `c_in_read.loi` printed:

```text
a=7 still=7
```

After `give`, the source is dead. `c_give_kills.loi` failed with:

```text
error: use of moved value 'first'
  --> /tmp/opencode/loi-phase0/c_give_kills.loi:10:50
   |
 9 |     second <- give(first)
   |                    ----- moved here
10 |     printf("second=%d first=%d\n", second.value, first.value)
   |                                                  ^^^^^^^^^^^ used after move
```

Answer for composite element `T`: use `f(give(x))` / `f<T>(give(x))` to move,
or `f(x)` for a read-only borrow; `->` out-parameters are initialized with
`f(-> x)`.

---

## d) Comparing generic values

Probe `d_generic_cmp.loi`:

```loi
flow less<T>
    <- a: T
    <- b: T
    => bool
{
    => a < b
}
```

The declaration and `less<i32>(2, 3)` check cleanly. Instantiating with a
composite shape fails at the instantiation with this exact diagnostic:

```text
error: operator requires numeric operands, found Bad and Bad
  --> /tmp/opencode/loi-phase0/d_generic_cmp.loi:10:8
   |
10 |     => a < b
   |        ^^^^^
```

Answer: a generic flow may write `a < b`, but it only succeeds for numeric
instantiations; a shape instantiation fails because there are no trait
constraints. This is why later generic sort/search flows must be documented as
scalar-only rather than pretending to support arbitrary `T`.

---

## e) `alloc` in helpers and slices returned with `give`

Probe `e_arena.loi` checked, built and ran inside one `protect` block:

```text
helper 9 9
give 41 44
return 7
```

Exact forms that work:

```loi
flow fill_bytes
    <- tag: u8
    -> data: [u8]
{
    alloc(-> data, u8, 8)
    ...
}

protect {
    mut filled: [u8]
    fill_bytes(9, -> filled)          // still valid after the helper returns
}
```

```loi
flow make_bytes
    -> data: [u8]
{
    alloc(-> data, u8, 4)
    ...
}

protect {
    mut made: [u8]
    make_bytes(-> made)
    owned <- give(made)               // explicit transfer of the allocated slice
}
```

```loi
flow return_given
    => [u8]
{
    mut local: [u8]
    alloc(-> local, u8, 3)
    local[0] = 7
    => give(local)
}
```

Answer: allocation performed inside a helper called from `protect` remains
valid for the remainder of that enclosing `protect`; it does **not** die at the
helper call. `give` transfers the slice handle, and both `->` and `=> give(...)`
forms are accepted within the protected region.

Escaping the region is rejected. `e_escape.loi` put `alloc(-> outside, ...)` in
a `protect` where `outside` was declared before it:

```text
error: value allocated inside 'protect' escapes the block
 --> /tmp/opencode/loi-phase0/e_escape.loi:7:18
  |
6 |     protect {
  |     ------- allocated in this 'protect' block
7 |         alloc(-> outside, u8, 4)
  |                  ^^^^^^^ escapes here
```

---

## f) Arena growth cost

Probe: `f_arena_growth.loi` with a temporary C shim `f_clock_shim.c`
(`probe_seconds()` wrapping `clock()`), linked with `--c-source`. The program
performs 100,000 `alloc(-> chunk, u8, 1024)` calls, writes the first and last
byte of each chunk, and prints `arena bytes=102400000 last=...`. The timer
starts just before `protect` and ends just after it, so it includes the one
`printf` inside the block but excludes process startup.

Nine runs each on the machine listed above:

| Configuration | Nine measured seconds | Min of 9 |
|---|---|---|
| default first chunk (64 MiB) | `0.060311 0.054048 0.054717 0.059516 0.055152 0.060751 0.056604 0.054939 0.054488` | **0.054048 s** |
| `--arena-size 1024` | `0.059511 0.055957 0.060389 0.053278 0.056671 0.055929 0.061331 0.057753 0.061461` | **0.053278 s** |

Both configurations allocated 102,400,000 bytes in 1 KiB pieces. With
`--arena-size 1024`, the first chunk is 1 KiB and later chunks grow
geometrically; no chunk is moved, so previously returned slices stay valid.
The measured numbers are end-to-end for this probe, not a per-`alloc` cost
claim.

---

## g) Instantiation depth and generated C size

Probe `g_depth.loi` recursively instantiates `nest<Box<T>>`:

```loi
flow nest<T>
    => i32
{
    => nest<Box<T>>()
}
```

`loi check` stopped with:

```text
error: generic instantiation exceeds the depth limit of 64; recursive instantiations such as 'f<T>' calling 'f<Vec<T>>' are not allowed
 --> /tmp/opencode/loi-phase0/g_depth.loi:8:13
  |
8 |     => nest<Box<T>>()
  |             ^^^^^
```

The measured maximum is therefore the **64** reported by the compiler.

Generated C size, measured with `loi gen FILE -o FILE.c` and `wc -c`:

| Probe | Instantiations | Generated C bytes | Unique `bump__*` symbols |
|---|---:|---:|---:|
| `g1.loi` | 1 (`Box<i32>` + `bump<i32>`) | 15,642 | 1 |
| `g20.loi` | 20 (`Box<T>` + `bump<T>` for 10 scalar and 10 nested `Box<...>` `T`s) | 26,994 | 20 |

Both programs checked, built and ran (`g1` printed `total=1`, `g20` printed
`total=20`).

---

## Extra import probes

The Phase 0 probes also checked the textual-import rules that later library
phases depend on:

- `imports_twice.loi` imports `/root/loi/lib/math.loi` twice; it checks, builds
  and prints `min=3`.
- `imports_lib_a.loi` and `imports_lib_b.loi` each import `math.loi`;
  `imports_two.loi` imports both libraries and prints `a=3 b=5`.

Duplicate imports are deduplicated; importing the same library from two
libraries did not produce a redefinition error.

---

## C compilation verification and its limit

For each runnable probe, the emitted C was generated with `loi gen` and
compiled with:

```sh
cc -std=c11 -pedantic-errors -Wall -Wextra "$c" -o "$exe"
```

Result: **9/9 probe programs passed** (`a_bitwise`, `b_comptime`, `c_ownership`,
`e_arena`, `f_arena_growth`, `g1`, `g20`, `imports_twice`, `imports_two`).

`-fsanitize=address,undefined` builds succeeded, but **none of the sanitizer
binaries could run in this PRoot/Termux container**. The first failure was:

```text
Error: heap size 40000002000 exceeds max user virtual address 7fffffffff
Try using a kernel that allows a larger virtual address space
ERROR: AddressSanitizer: out of memory: failed to allocate 0x2000 (8192) bytes of memory at address 0x040000000000 (error code: 12)
AddressSanitizer: CHECK failed: sanitizer_common.cpp:61 "((0 && "unable to mmap")) != (0)" (0x0, 0x0)
```

Retrying with `ASAN_OPTIONS=allocator_may_return_null=1:detect_leaks=0` did not
change it. This is an environment/ASan shadow-memory mapping failure, not a
report from the probe programs. UBSan-only builds and runs were clean for all
runnable probes; `c_generic_give` emitted a compiler warning
(`-Wformat-security`) because its probe used `printf` with no conversions, not
a UBSan report. Library tests in later phases must avoid that printf shape.

## Gate

Phase 0 changed only documentation. With
`PATH=/root/.cargo/bin:$PATH` and `RUSTFLAGS="-C linker=/usr/bin/gcc"`:

- `cargo fmt --check` — exit 0
- `cargo clippy --all-targets -- -D warnings` — exit 0
- `cargo test --all-targets` — **269 passed, 0 failed** (per binary: `src/lib.rs` 22, `src/main.rs` 6, `tests/bindgen.rs` 1, `tests/checker.rs` 102, `tests/cli.rs` 42, `tests/codegen.rs` 43, `tests/errors.rs` 2, `tests/lexer.rs` 6, `tests/lib.rs` 7, `tests/parser.rs` 23, `tests/pretty.rs` 15)
- `python3 tools/sync_examples.py --check` — `website/examples is in sync`, exit 0
- `git diff --check` — exit 0
