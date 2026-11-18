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
caller-owned buffer can be resized by the callee. Current workaround:
`vec_new<T>` uses `alloc(-> v.data, T, 1)`, so an empty vector has capacity one.
`alloc(..., 0)` was measured to work and produce length 0, but the allocator
still has to be called from inside `protect`.

### Generic comparisons fail for composite `T`

Attempted design: generic ordering helpers (`math_min<T>`, `math_max<T>`) over
arbitrary `T`. The Phase 0 probe `d_generic_cmp.loi` confirmed:

```text
error: operator requires numeric operands, found Bad and Bad
```

Language requirement: trait constraints/ord bounds on generic type parameters.
Current workaround: document ordering/search/sort flows as numeric-only where
needed, and avoid pretending that `T` is comparable.

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
