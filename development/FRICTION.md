# Standard-library language friction

This file records library designs that were tried and are blocked by the
current Loi language/checker. It is updated in the phase where the friction is
encountered. Each entry states what was attempted, the exact diagnostic, and
the language requirement needed to implement the design without a workaround.

## Phase 1 — `lib/vec.loi`

### Slice-containing element types cannot be moved or cloned through a vector

Attempted design: a generic `Vec<T>` whose `T` may itself be a slice, such as
`Vec<[u8]>`. The move-oriented push copied existing elements with
`grown[i] = give(v.data[i])` while growing and stored the argument with
`v.data[i] = give(value)`. The source was `/tmp/opencode/loi-phase0/vec_move_proto.loi`.

Resulting checker diagnostics included:

```text
error: cannot assign to immutable variable 'first'
error: use of moved value 'xs'
error: use of moved value 'xs'
```

The first diagnostic came from using `<-> first` for a value that the caller
had declared immutable. Making the callers mutable and retrying the inout path
still left the ownership problem: moving individual elements out of `v.data`
consumes the vector, so the vector cannot be used again.

The clone-based fallback prototype (`vec_proto2.loi`) uses
`grown[i] = clone(v.data[i])` and `v.data[v.len] = clone(value)`. It works for
scalars and composite shapes without slice fields, but the checker rejects
cloning a slice-containing composite:

```text
error: cannot copy [u64] implicitly; use give(...) or clone(...)
```

That diagnostic was observed while trying to return a shape containing a
slice from a `comptime flow` (`b_shape_slice.loi`), and the same limitation
applies to cloning such a value inside a generic vector operation.

Language requirement: support moving an element out of an inout composite
field without consuming the whole composite, or provide a clone operation for
composites that recursively clones slice fields. Until then `Vec<T>` is
documented as requiring cloneable `T`: scalars and shapes whose fields are
themselves cloneable without slices.

Resolved by Block A1 (giving a composite element now consumes only that element
path, with path-sensitive cross-flow summaries) and Block A2 (`clone` is now a
recursive deep copy: slice-containing `T`, including `Vec<[u8]>`, works). The
`clone`-inside-`protect` rule supplies the allocation region; the previously
blocked `vec_proto2.loi` pattern now type-checks and runs.

A follow-up attempt to replace `clone` with `give` in `vec_reserve`, `vec_pop`,
`vec_swap` and `slice_swap` exposed a second limitation: after a composite
element is moved, passing the whole vector to a `<-` flow such as `vec_len`
was rejected because a whole-place read overlapped every moved subpath.
**Resolved by Block A3:** the checker now summarizes the paths each `<-` flow
reads (forwarding them through calls) and accepts the whole-value argument when
those paths avoid the consumed ones. The `vec_len`-after-move pattern is
covered by checker and codegen tests.

### Borrowed slice views cannot escape a flow

Attempted design: `vec_as_slice<T>(<- v: Vec<T>) => [T]` returning `v.data`
directly. The source was `/tmp/opencode/loi-phase0/vec_as_slice_proto.loi`.

Exact diagnostic:

```text
error: cannot return a value borrowed from '<-' parameter 'v'; use clone(...)
  --> /tmp/opencode/loi-phase0/vec_as_slice_proto.loi:11:8
   |
11 |     => v.data
   |        ^^^^^^
```

Language requirement: slice borrow lifetimes (or a borrow-safe view type) so a
callee can return a slice that aliases storage owned by the caller for a
statically bounded region. The original library workaround was
`vec_as_slice<T>`, which allocated a new arena slice inside the caller's
`protect` and copied the live prefix into it.

**Resolved by Block A3 (real views):** the `view(xs, start, end)` intrinsic,
per-flow return-view summaries and lexical loans now allow a slice rooted in a
`<-` parameter to escape the call safely. Writes, moves and mutable borrows of
an overlapping source path are rejected while a view is live, and the source's
storage region still restricts where the result may be stored. New zero-copy
library flows are `slice_view<T>` and `vec_view<T>`; `vec_as_slice<T>` keeps
its detached-copy behaviour for callers that need an independent buffer.

### A generic slice literal cannot escape into an out-parameter

Attempted design: initialize an empty vector with `v.data = [T:]` inside
`vec_new<T>`. The source was `/tmp/opencode/loi-phase0/vec_proto.loi`.

Exact diagnostic:

```text
error: slice literal storage escapes the block that created it; keep it in a variable declared in the same block
  --> /tmp/opencode/loi-phase0/vec_proto.loi:10:14
   |
 7 | flow vec_new<T>
   | ---- allocated in this block
10 |     v.data = [T:]
   |              ^^^^ escapes here
```

Assigning a slice literal in the caller and passing it in was also tried; the
callee cannot write the slice header's `len`, and the C backend rejects that
assignment:

```text
error: lvalue required as left operand of assignment
```

Language requirement: either permit escaping slice literals into out-parameters
with an explicit lifetime/arena region, or make slice length writable so a
caller-owned buffer can be resized by the callee.

**Resolved by Block A4.** The empty literal `[T:]` emits `{ NULL, 0 }` and needs
no backing array, so the checker no longer taints it with the current lexical
block; it can be returned or assigned into an out-parameter. Non-empty literals
still carry block storage and remain confined to their block. Separately, slice
member places now use the real `LoiSlice` field name (`len`) in generated C, so
`s.len = n` through a `<->` parameter compiles and runs. `vec_new<T>` now uses
`v.data = [T:]`, `v.len = 0`, `v.cap = 0` and no longer needs a `protect`;
`vec_push<T>` reserves at least one slot on first growth. The remaining
constraint is unchanged: growing a vector still calls `alloc` and therefore
needs a caller `protect` region.

### Generic comparisons fail for composite `T` — resolved

Attempted design: generic ordering helpers (`math_min<T>`, `math_max<T>`) over
arbitrary `T`. The Phase 0 probe `d_generic_cmp.loi` confirmed:

```text
error: operator requires numeric operands, found Bad and Bad
```

Block A5 first added `T: Clone + Copy + Eq` bounds so `vec_find<T>` and
`slice_find<T>` could search numbers, bool, marks and `[u8]`. `Copy` marks flows
that pass `T` by value; `Clone` documents flows that copy elements.

**Resolved by structural `Eq` + `Ord`.** `==`/`!=` now compare numbers, bool,
marks, `cstr`, opaque handles, shapes, choices and slices structurally; the
new `Ord` bound covers `<`, `<=`, `>` and `>=` with a lexicographic rule, except
for opaque handles and callbacks, which have no order. `vec_find<T>` now works
on whole-shape needles and `vec_sort<T>`/`vec_binary_search<T>` declare
`T: Clone + Ord`; `examples/ordering.loi` sorts and searches `Vec<Task>` by
priority, status, id and label. Checker, monomorphizer and C backend share the
rule via `types::ComparisonCaps`/`ComparisonEnv`.

Still open: compile-time comparison of non-numeric values. `const_eval`'s
`compare_values` remains numeric-only, so ordering a shape, mark or slice in a
`comptime` expression or global initializer reports
`comparison requires numeric compile-time values`; runtime comparisons are
unaffected. `math_min<T>`/`math_max<T>`/`math_sum<T>` still require numeric `T`
because they perform arithmetic, not because ordering is unavailable.

### Generic sum types cannot be `choice` declarations

Attempted design: a generic `choice Option<T> [ Some: T, None ]` or
`choice Result<T, E> [ Ok: T, Err: E ]` as the official optional/error types.
The checker rejects generic choices twice: the parser has no type parameters on
`choice` declarations, and `valid_choice_payload` in `src/checker.rs` permits
only scalars, marks and slice-free shapes as payloads. A payload such as `T` or
`[u8]` is neither.

Exact diagnostic for a slice payload:

```text
error: choice 'Option' variant 'Some' cannot hold [u8]; payloads must be scalars, marks or slice-free shapes
```

**Resolved by Block A6 with a documented shape-based representation.**
`lib/option.loi` and `lib/result.loi` use generic shapes with a `bool` tag and
typed payload fields, following the same `found`-plus-default pattern as
`map_*_lookup`. The unselected field is zero-initialized and is only read by
the library's guarded flows. This keeps the ABI and the closed `choice` rules
unchanged and lets `T` contain slices, at the cost of no exhaustive pattern
match: callers test the tag explicitly. A future generic-choice design would
need parser, monomorphizer and C-union changes and is not planned here.

## Phase 2 — `lib/string.loi` and `lib/hash.loi`

### Character literals do not exist

Attempted design: write byte-oriented tests with character literals, for
example `str_contains("banana", 'n')` and `c: u8 <- 'n'`. The probe was
`/tmp/opencode/loi-phase0/char_literal.loi`.

Exact diagnostic:

```text
error: unexpected character '''
 --> /tmp/opencode/loi-phase0/char_literal.loi:4:14
  |
4 |     c: u8 <- 'n'
  |              ^

1 error
```

Language requirement: a `u8` character-literal form with escapes. Current
workaround: pass the numeric byte value (`110` for `n`) to `str_count`, or pass
a one-byte string slice (`"n"`) to `str_index_of` and `str_contains`. The
library APIs themselves are not blocked; only the test spelling changed.

### No allocation-free dynamic error formatting was needed

No Phase 2 flow was blocked. `str_repeat` checks multiplication overflow before
allocating and calls `panic` with a literal, flow-naming message; `hash_*` flows
are pure. The `if false { panic(name) }` placeholder initially written into
`lib/string.loi` was dead code and was removed after the tests passed.

## Phase 3 — `lib/crc32.loi`

### Comptime flows cannot assign to shape fields

Attempted design: build a packed shape of compile-time table words by mutating
its fields in a `comptime flow`. The probe was
`/tmp/opencode/loi-phase0/comptime_shape.loi`. An uninitialized local shape
failed first:

```text
error: initializer for global 'PACK' must be a compile-time constant: comptime flow bindings require an initializer
  --> /tmp/opencode/loi-phase0/comptime_shape.loi:22:15
```

Giving the local a shape literal and assigning fields then produced:

```text
error: initializer for global 'PACK' must be a compile-time constant: comptime flows can only assign to local bindings
  --> /tmp/opencode/loi-phase0/comptime_shape.loi:22:15
```

Language requirement: permit field assignment on local composite bindings
inside a `comptime flow`, or provide a compile-time builder form. Current
workaround: compute each packed word in a scalar `comptime flow` and construct
the table in a global slice literal whose elements are `comptime` calls; the
C backend emits it as a static named array.

### A 256-entry table cannot be packed into `u64` words

Attempted design: pack eight 32-bit entries into each `u64` with shifts up to
224 bits. The probe was `/tmp/opencode/loi-phase0/comptime_pack_shape.loi`.

Exact diagnostic:

```text
error: initializer for global 'PACK' must be a compile-time constant: shift count is out of range
  --> /tmp/opencode/loi-phase0/comptime_pack_shape.loi:45:15
```

Language requirement: a wider integer type or compile-time byte arrays so 256
32-bit entries can be stored compactly. Current workaround: use the 16-entry
reflected nibble table, pack two 32-bit entries per `u64`, and process two
nibbles per input byte. It is still generated entirely by `comptime` flows and
matches a bitwise reference on known vectors and seeded data.

## Phase 4 — `lib/map.loi`

### Slice keys are stored in a contiguous buffer

Attempted design: `StrMap<V>` with a `keys: [[u8]]` field so each entry owns its
key slice directly. This was the same slice-containing-composite limitation
recorded in Phase 1: slices could not be cloned recursively and moving one out
of an inout composite consumed the map. No new diagnostic was needed for this
phase; the Phase 1 record applied unchanged.

Block A1 (path-sensitive partial moves) and Block A2 (recursive deep `clone`)
lift the language restriction, so a `[[u8]]` value is now representable. The
current contiguous-buffer design is retained deliberately: storing all key
bytes in one `keys: [u8]` arena buffer with `starts` / `lengths` arrays avoids a
per-key allocation and keeps rehashing cheap. It is no longer a workaround for
a checker limitation.

No other Phase 4 design was blocked. `IntMap<V>` and `StrMap<V>` are generic
only over `V`, which must be cloneable (recursive deep clone now covers values
that contain slices).

## Phase 5 — `lib/random.loi`

No Phase 5 design was blocked. `Rng` is a single `u64` inout state; xorshift64*,
bounded draws, picking and Fisher–Yates shuffling all fit in pure Loi without
allocation. The only earlier test mistakes were ordinary language rules: a
binding passed with `<->` must be declared `mut`, and two inout calls cannot be
sequenced inside one expression. Neither is a blocked design.

## Block B1 — real modules and namespaces

### Public library items had to become `open`

Once `open`/`sealed` was enforced across module boundaries, calls to default-
sealed public stdlib flows were rejected:

```text
error: 'ctype_is_digit' is sealed in module 'ctype' and cannot be used from module 'main'
```

Public flows, shapes, globals, marks and choices in `ctype`, `math`, `net`, `os`
and `random` were changed to `open`, and `tools/bindgen/bindgen.py` now emits
`open` on every generated declaration. Internal helpers stay `sealed`, and a
module still accesses its own sealed items.

### Declaration names remain global per compilation unit — resolved

Module qualification initially disambiguated references, not declarations or C
symbols. Two imported modules that both declared `helper` failed in the checker
even when every call was qualified:

```text
error: duplicate function 'helper'
```

Probe: `/tmp/opencode/loi-b1-friction`. **Resolved by module-qualified
mangling:** names declared in more than one module receive an internal
`<module>::<name>` spelling and distinct C symbols. Unqualified names bind to
the current module first; a call from a third module is an explicit ambiguity
error listing the qualified candidates. Unique names keep their old symbols, so
single-module programs and the standard library remain ABI-compatible. Two
resolved names that sanitize to the same C identifier are rejected rather than
silently producing a duplicate definition.

## Block B2 — linear resources and `guard`

### The release name is the full snake-case shape name

The first realistic example declared `shape Connection` with a release flow
named `conn_release`. The checker rejected it:

```text
error: shape 'Connection' has no resource release flow 'connection_release'
```

`guard` derives the flow name from the entire shape name, not from the binding
name or an abbreviation. `Connection` needs `connection_release`; `HttpClient`
needs `http_client_release`. There is no alias or attribute to override it, so
two shapes with the same snake-case name cannot both be resources in one
compilation unit. The example was renamed to `connection_release` and now
checks, builds and runs.

### The release signature is exact

A release flow with `<-` instead of `<->`, an extra parameter, or a result is
rejected at the `guard`:

```text
error: invalid resource release flow 'file_release' for shape 'File'; expected flow file_release(<-> value: File) with no return
```

This is deliberate: the compiler synthesizes the call, so the direction,
parameter count and void result must be unambiguous. A hand-written release
that also returns a status code has to put that status in a mutable global or
field instead.

### No standard-library resource API yet (resolved by B3)

`guard` worked with any shape that had a matching release flow, but Loi had no
long-lived file or socket handle library. A real program had to declare its own
shape and release flow first; `examples/guard_resources.loi` still shows that
pattern. Block B3 added OS-backed `File` and `TcpStream` handles to
`lib/os.loi` and `lib/net.loi`.

## Block B3 — standard-library linear handles

### A cross-module release flow must be `open`

The module resolver already enforced `open`/`sealed` for names written in source,
but the release call inserted by `guard` had no source-level name to check. A
probe with a sealed `handle_release` in another module was accepted by `check`
and would fail at link time. The resolver now derives the release name from the
guard's declared type or initializer result and runs it through the same
visibility check. The exact diagnostic is:

```text
error: 'handle_release' is sealed in module 'resource' and cannot be used from module 'main'
 --> /tmp/opencode/loi-friction-b3/main.loi:5:21
  |
5 |     guard handle <- make_handle() {
  |                     ^^^^^^^^^^^^^ sealed item used here

1 error
```

The stdlib release flows `file_release` and `tcp_stream_release` are therefore
`open`. Probe: `/tmp/opencode/loi-friction-b3`.

### Read buffers must be caller-owned

`file_read` and `tcp_stream_recv` take the destination with `<->`. Passing a
non-empty slice literal or a buffer that escapes its `protect` region is
rejected by the existing ownership and region rules. Callers allocate the
buffer in an enclosing `protect` block and pass it with `<->`; the release flow
then runs before that region is restored.

## Block B4 — binding generator input buffers

### `const T *` parameters were reported as skipped

Real headers pass read-only buffers as `const unsigned char *` or `const int *`
plus a separate length. The generator reported them as unusable:

```text
error-equivalent skip: sample_checksum: parameter 'bytes' is a const pointer to a scalar
```

Those functions appeared in `vendor/sdl2/skipped.txt` and
`vendor/raylib/skipped.txt` even though the C call is safe: the callee only
reads the buffer for the duration of the call. **Resolved by Block B4:** the
parameter now binds as a borrowed `[T]` slice and the generated shim passes
`slice.data`. The C length parameter stays explicit rather than being guessed
from a neighbouring argument name, so the caller writes
`sample_checksum(bytes, bytes.len)`. `const char *` remains `cstr`, and const
pointer results remain skipped because no Loi type expresses who owns the
returned storage.

## Block B5 — binding generator writable buffers

### Mutable `char *` parameters were reported as skipped

Functions that fill a caller-owned buffer, such as
`int sample_fill(char *buf, int len)`, were skipped:

```text
error-equivalent skip: sample_fill: parameter 'buf' is a mutable char buffer; use a vendored shim
```

The C call is safe as long as the caller owns the storage and passes its real
length, which is exactly what a mutable Loi slice expresses. **Resolved by
Block B5:** the parameter binds as `<-> [u8]` and the caller writes
`sample_fill(<-> buffer, count)`. The generated shim receives `LoiSlice *`
because the C backend passes `<->` slices by pointer, then forwards
`buf->data`. The length parameter stays explicit, and mutable pointer results
remain skipped because ownership of the returned storage is unspecified.

## Block B6 — C callbacks

### Flows could not be passed across the FFI boundary

C APIs such as reducers, sorters and event dispatchers take function pointers,
but Loi flows were not values and the bindgen skip report said:

```text
error-equivalent skip: sample_for_each: parameter 'cmp' is a function pointer
```

Binding these calls by hand still required a global C thunk per callback, with
a bespoke signature and a way for the thunk to reach the Loi flow. **Resolved
by Block B6:** Loi has a fixed-ABI `callback` type
(`int32_t (*)(int32_t, int32_t)`) and a `callback(flow)` intrinsic. The checker
accepts only a non-extern flow with exactly `<- a: i32, <- b: i32 => i32`, and
the generated C passes that flow's symbol directly: no thunk, trampoline or
userdata is needed. Callback values are copyable and storable, but Loi cannot
call, compare or dereference them, so the narrow ABI stays honest.

The generator still skips C function pointers: automatically choosing a thunk
would have to guess closure state and lifetime, which Loi does not express.
Declare those `extern flow`s by hand; `examples/callbacks.loi` and
`examples/callbacks_shim.c` show the complete pattern.

### The fixed callback ABI excluded common C callbacks

Attempted design: bind a C visitor that takes a value callback such as
`int32_t (*)(int32_t)` and a notifier such as `void (*)(int32_t)`. Writing the
`extern flow` parameter as plain `callback` made the checker reject the Loi
flow with:

```text
error: argument 'f' has type callback(i32) => i32, expected callback
```

Language requirement: let a callback type carry the C signature while keeping
flows non-first-class and thunk-free. **Resolved:** callback types may now be
written as `callback(T, ...) => R`, with `void` as a result; `callback(flow)`
infers the signature from a non-extern, all-`<-` flow and the use site checks
that it matches. The backend emits one guarded typedef per signature and still
passes the flow symbol directly. `examples/callback_signatures.loi` and
`examples/callback_signatures_shim.c` show the complete pattern.

## Block L1 — deterministic stdio byte I/O

### `read_stdin()` waits for EOF

The first LSP transport probe used `read_stdin()` for a `Content-Length` body.
It blocked until the client closed stdin, so a request/response loop could never
make progress with a live connection. `read_line()` reads headers incrementally,
but there was no way to request exactly N body bytes. **Resolved by Block L1:**
`lib/stdio.loi` exposes `stdio_read_exact(<-> data) => i64`, backed by repeated
`fread` in `lib/stdio_shim.c`. It returns when the buffer is full, at EOF, or on
error; a short non-negative count is an EOF signal the caller must handle.

### stdout needed an explicit flush

`printf` and `print` write through C stdio. When stdout is a pipe, output stays
buffered, so a server can wait for the next request while its response is still
in userspace. **Resolved by Block L1:** `stdio_write_all(<- data) => i64` writes
all bytes and `stdio_flush()` calls `fflush(stdout)` before the next read. The
frame-echo test keeps stdin open and uses a 10-second timeout to prove the loop
does not depend on EOF.

### Mixing raw descriptors with stdio would reorder output

`lib/os.loi` and `lib/net.loi` use file descriptors and `read`/`write`, while the
print intrinsics and the new stdio flows use `FILE*`. Buffering makes their order
unpredictable on the same stream. The contract is therefore explicit: use the
stdio flows for stdin/stdout, keep raw descriptors for files and sockets, and
never mix the two APIs on one stream. `examples/stdio_frames.loi` shows the
complete framing pattern; a zero-length frame is its shutdown sentinel.

## Opt-in LLVM backend

### The draft IR was not compilable LLVM IR

`src/ir.rs` emits a textual draft with pseudo-instructions such as `ptr<T>`,
`slice<T>`, `type{bits}`, `jump`, `condjump`, `imm`, `allocslice`, `indexslice`
and `bitnot`. Feeding its output to `clang -x ir` fails at the first type, and
`--emit-ir` has always been a debugging view rather than a second production
backend. Turning it into real LLVM IR would have meant rewriting working draft
code used by existing tooling. **Resolved:** a separate `src/llvm.rs` lowers the
checked, monomorphized AST directly to textual IR with opaque pointers and the
same C symbols; the C11 backend and `--emit-ir` are untouched.

### Ownership features need an agreed LLVM ABI

A first attempt to route the whole language through the new emitter reached
`protect`, borrowed slice views, callbacks and `comptime` values without an
agreed LLVM representation. **Resolved for regions and scalar slices:** the
LLVM backend now uses `%LoiSlice = type { ptr, i64 }` to match C's `LoiSlice`
ABI, supplies its own bump-arena and checked-index helpers, and lowers
`protect` by saving and restoring the arena top. `examples/llvm_slices.loi`
builds with both backends and prints identical output, including the runtime
diagnostic for an out-of-bounds index. Callbacks, `comptime`, shapes, marks,
choices, opaque handles, globals and composites with slice elements still have
no agreed ABI and remain rejected with an `LLVM backend does not support ...`
diagnostic. C11 is still the production path and the semantic oracle.
