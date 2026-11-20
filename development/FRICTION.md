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
statically bounded region. Current library workaround: `vec_as_slice<T>`
allocates a new arena slice inside the caller's `protect` and copies the live
prefix into it.

Block A3 deliberately does **not** address this case: its read views are
lexical to the call and never escape the callee. Returning a borrowed slice
needs return-alias summaries and borrow-region tracking (rejecting moves and
writes to the source while the view is live), which is a separate follow-up.

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

### Generic comparisons fail for composite `T`

Attempted design: generic ordering helpers (`math_min<T>`, `math_max<T>`) over
arbitrary `T`. The Phase 0 probe `d_generic_cmp.loi` confirmed:

```text
error: operator requires numeric operands, found Bad and Bad
```

**Partially resolved by Block A5.** Generic type parameters can now declare
`T: Clone + Copy + Eq` bounds. The `Eq` bound makes equality search work for
every type the checker already accepts with `==`: numbers, bool, marks and
`[u8]`. `vec_find<T>` and `slice_find<T>` now declare `T: Eq` instead of being
numeric-only, and `vec_find<bool>` is covered by a library test. `Copy` marks
flows that pass `T` by value; `Clone` documents flows that copy elements.

Still open: ordering. No `Ord` bound was added, so `<`/`>` on generic `T` are
still numeric-only and `math_min<T>`, `math_max<T>`, `math_clamp<T>`,
`math_sum<T>`, `vec_sort<T>` and `vec_binary_search<T>` keep their numeric
restriction. Structural equality for shapes, choices and non-byte slices is
also still unavailable; a future `Eq` implementation would need code generation
for those types. Language requirement: an `Ord` constraint (and broader
structural `Eq`) before ordering helpers can be honest about arbitrary `T`.

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

### Declaration names remain global per compilation unit

Module qualification disambiguates references, not declarations or C symbols.
Two imported modules that both declare `helper` still fail in the checker even
when every call is qualified:

```text
error: duplicate function 'helper'
```

Probe: `/tmp/opencode/loi-b1-friction`. Isolating declarations per module needs
module-qualified C mangling, an ABI-visible change deferred to a later block.

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

### No standard-library resource API yet

`guard` works with any shape that has a matching release flow, but Loi still
has no long-lived file or socket handle library. A real program must declare
its own shape and release flow first; `examples/guard_resources.loi` shows the
pattern. Adding OS-backed resources to `lib/net.loi`/`lib/os.loi` is future
work and needs its own shim and ABI review.
