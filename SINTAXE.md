# Loi — Canonical Syntax (SINTAXE)

**This file is the canonical surface reference.** Every example, test, library,
website snippet and generated binding must use exactly these forms. When in
doubt, this file wins; if this file and the compiler disagree, fix one of them
in the same change.

The language specification and rationale live in
[`development/LOI_SPEC.md`](development/LOI_SPEC.md). Current compiler state
lives in [`development/PROGRESS.md`](development/PROGRESS.md).

**Rule of thumb:** *braces are scopes, brackets are data, arrows are movement.*

---

## 1. Lexical rules

- Source files end in `.loi`.
- Comments are `// ...` and run to the end of the line.
- Statements are separated by newlines. A `}` also ends the previous statement.
- Keywords (the complete set):

  ```text
  alloc  and  as  break  choice  clone  comptime  continue  else  extern
  false  flow   for  give   if      import  in   is    mark    mut
  not    opaque open or     protect sealed  shape   true  while
  ```

- Direction glyphs: `<-` (read), `->` (write), `<->` (read/write), `=>` (result).
- Multi-character tokens use longest match first:

  ```text
  <->   <-   =>   ..   ->   <<=   >>=   <<   >>
  +=    -=   *=   /=   %=    &=    |=    ^=
  ==    !=   <=   >=   &     |     ^     ~
  ```

- Identifiers start with a letter or `_`. The single name `_` is a discard.
- Integer literals: decimal, `0x` hexadecimal, `0b` binary, with `_` separators.
- Float literals: decimal point and/or exponent (`1.0`, `1e9`, `2.5e-3`).
- String literals use double quotes. Escapes: `\n`, `\t`, `\r`, `\0`, `\\`,
  `\"`, `\xNN` (ASCII `0x00`–`0x7f`).

---

## 2. Types

| Type | Meaning |
|---|---|
| `i8 i16 i32 i64` | signed integers |
| `u8 u16 u32 u64` | unsigned integers |
| `f32 f64` | floating point |
| `bool` | `true` / `false` |
| `[T]` | slice: pointer + length, bounds checked |
| `Name` | a declared `shape`, `mark` or `choice` |
| `Name<T, ...>` | an instantiation of a generic shape |
| `Name` | a declared `opaque` foreign handle |
| `cstr` | NUL-terminated C string handle (FFI only) |

There is no `void`, no pointer type, no array type and no `nil`. A function
with no `=> T` returns nothing.

- Slices are written **`[T]`**, never `[]T`.
- Generic instantiations are written **`Vec<T>`**. There is no type inference:
  every generic call and literal spells its type arguments.
- Strings are `[u8]` at runtime. A string literal is a `[u8]` value.
- `cstr` values are constructed only from string literals or returned by
  `extern flow` declarations. They can be passed back to C but not indexed,
  measured or compared.

---

## 3. Declared data types

### 3.1 Shapes — product types

```loi
shape Vec2 [ x: f64, y: f64 ]

shape Particle [
    pos:   Vec2
    vel:   Vec2
    mass:  f64
    alive: bool = true
]
```

- `shape Name [ ... ]` declares a data-only type. No methods, no inheritance.
- Fields are `name: Type`, separated by commas or newlines; the trailing comma
  is optional.
- A field may have a default: `alive: bool = true`.
- Empty shape: `shape Empty []`.
- Visibility: `open shape Name [...]` exports the shape; the default is
  `sealed`. `sealed shape ...` is allowed for emphasis.
- A shape literal carries its type name and **brackets**:

  ```loi
  origin <- Vec2[ x: 0, y: 0 ]
  empty  <- Empty[]
  ```

- Complete initialization is the norm. Fields with defaults may be omitted; the
  default is applied. `-> shaped` out-parameters are initialized from their
  defaults by the backend before the body runs.

### 3.2 Marks — finite tag sets

```loi
mark Color [ Red, Green, Blue ]

open mark Flag [
    On
    Off
]

c <- Color.Green
if c != Color.Red { ... }
```

- `mark Name [ ... ]` declares a payload-free finite tag set. It is Loi's
  enum-like sum type; there is no C-style `enum` keyword.
- Variants are `Name.Variant` values, separated by commas or newlines; the
  trailing comma is optional. A bare variant name is not a value.
- Visibility follows the usual rule: `open mark ...` exports it, the default is
  `sealed`, and `sealed mark ...` is allowed for emphasis.
- Marks are scalar-like values. `==` and `!=` are defined only between two
  values of the **same** mark; ordering and arithmetic are not.
- A mark may be stored in a flow parameter, a shape field or a `choice` payload.
  It copies like a scalar.

### 3.3 Choices — tagged payloads

```loi
choice Result [ Ok: i32, Err: i64 ]

r <- Result.Ok(42)
if r is Result.Ok {
    print_int(r.Ok)
}
```

- `choice Name [ Variant: Type, ... ]` declares a tagged union. It is Loi's
  union-like sum type; there is no C-style `union` keyword.
- Each variant declares exactly one payload type, separated by commas or
  newlines; the trailing comma is optional. Visibility is `open` or `sealed`
  (the default).
- `Name.Variant(value)` constructs a value with exactly one `<-` argument whose
  type matches the declared payload. A bare `Name.Variant` is not a value.
- `value is Name.Variant` tests the active variant and produces `bool`. The
  right operand must be a variant of the same choice.
- `value.Variant` reads the payload. It is **checked at runtime** against the
  active tag; a wrong read aborts with
  `loi: choice 'Name' does not hold 'Variant' at <line>:<column>`. `is` does not
  statically narrow the type, so the same check applies even inside a matching
  branch. Payload reads are values; assigning to `value.Variant` is rejected.
- Payload types are restricted to numeric scalars, `bool`, `mark` and
  slice-free shapes. Slices, shapes containing slices, nested choices, `cstr`
  and `opaque` handles are rejected because they would make the generated C
  union unsafe. There are no generic choices.
- Choice names and mark names share the type namespace with shapes, opaque
  handles, flows and globals; duplicates are compile errors.

### 3.4 Generic shapes

```loi
shape Vec<T> [
    data: [T]
    len:  u64
]

shape Pair<T, U> [
    first:  T
    second: U
]

numbers <- Vec<i32>[ data: [i32: 1, 2, 3], len: 3 ]
pair    <- Pair<i32, bool>[ first: 1, second: true ]
nested  <- Vec<Vec<i32>>[ data: [Vec<i32>:], len: 0 ]
```

- Type parameters are declared after the shape name: `shape Name<T, U> [...]`.
- Every instantiation supplies all type arguments explicitly; there is no
  inference. `Vec[...]` without `<...>` is an error.
- Generic declarations are templates. The compiler monomorphizes each used
  instantiation into a concrete shape before type checking and code generation.
- Type arguments may be primitives, shapes, slices, or other generic
  instantiations. The lexer's `>>` is split when closing nested arguments, and
  the split is undone when a speculative parse backtracks.
- A template that is never instantiated is still checked once, with `i32`
  substituted for every type parameter. This catches name, type and ownership
  errors in unused code. It is a smoke test, not a proof that the body is valid
  for every `T`; real uses are checked with their actual arguments.
- Instantiation is not allowed to recurse indefinitely. A template such as
  `f<T>` that instantiates `f<Vec<T>>` stops at a depth limit with a clear
  diagnostic instead of generating types forever.
- The generated C name for `Vec<i32>` is `Vec__i32`. If a user declaration
  already uses that name, or two instantiations mangle to the same identifier,
  the compiler reports the collision and suggests renaming the declaration or
  changing the type argument.

---

## 4. Bindings and mutability

```loi
answer <- 42                 // immutable
mut total: i64 <- 0          // mutable, explicit type
mut buffer: [u8]             // uninitialized; only valid as an `->` target
name: Vec2 <- Vec2[ x: 1, y: 2 ]
```

- `name <- value` declares an immutable binding.
- `mut name <- value` declares a mutable binding.
- `name: Type` and `mut name: Type` declare without an initializer. An
  uninitialized binding can only be passed to an `->` parameter; reading it
  first is a compile error.
- Assignment requires `mut`: `total = 5`, `total += 1`, `-=`, `*=`, `/=`, `%=`,
  `&=`, `|=`, `^=`, `<<=`, `>>=`.
- Shadowing in the same scope is a duplicate-declaration error.
- Scalars copy implicitly. Composite values move by ownership.

Top-level bindings are globals:

```loi
GREETING: [u8] <- "hello\n"
open mut LIMIT: i32 <- 3
```

Global initializers must be compile-time constants: literals, constant
expressions, constant globals, shape/slice literals and `comptime` calls. The C
backend emits them as static initializers, including nested shapes, slices and
strings. These initializers are strict C11: shapes use brace/designated
initializers (no compound literals), and slice constants are backed by named
`static` arrays emitted before the globals. Globals can be read or cloned, but
`give` cannot transfer ownership out of a global or out of a field/element of
one; use `clone(...)` instead.

---

## 5. Flows — functions

A flow declares, in its signature, every direction data moves.

### 5.1 Inline form

```loi
flow add(<- a: i32, <- b: i32) => i32 {
    => a + b
}

flow no_args_no_result {
}
```

### 5.2 Vertical form (canonical for real code)

```loi
open flow world_new
    <- count: u32
    <- size: Vec2
    -> w: World
{
    ...
}

sealed flow clamp
    <- v: f64
    <- lo: f64
    <- hi: f64
    => f64
{
    if v < lo { => lo }
    if v > hi { => hi }
    => v
}
```

- Visibility comes **before** `flow`: `open flow`, `sealed flow`. Default is
  `sealed`.
- Parameter direction is mandatory.
- The result clause is `=> T`. Omit it for no result.
- The body is always a `{ ... }` block.
- `main` is the entry point and returns `i32` by convention:

  ```loi
  open flow main
      => i32
  {
      => 0
  }
  ```

### 5.3 Generic flows

```loi
flow sum<T>
    <- xs: Vec<T>
    => i32
{
    mut total: i32 <- 0
    for i in 0..xs.len {
        total += xs.data[i] as i32
    }
    => total
}

flow main
    => i32
{
    xs <- Vec<i32>[ data: [i32: 1, 2, 3], len: 3 ]
    => sum<i32>(xs)
}
```

- Declare type parameters after the flow name: `flow name<T, U>`.
- Calls spell their type arguments: `sum<i32>(xs)`. There is no inference, and
  a generic flow without type arguments is an error.
- Passing a type argument to a non-generic flow is an error, as is the wrong
  number of arguments.
- Generic flows are monomorphized per instantiation, so each concrete type
  argument produces a separate compiled function. Unused generic flows are
  checked with `i32` placeholders, and recursive instantiations hit the same
  depth limit as generic shapes.

### 5.4 `comptime`

```loi
comptime flow double
    <- x: i32
    => i32
{
    => x * 2
}

LIMIT: i32 <- comptime double(21)   // 42, computed at compile time

flow next
    => i32
{
    => comptime double(4)           // 8, emitted as a literal
}
```

- `comptime flow` declares a pure flow that the compiler can evaluate. Its
  parameters must be `<-`; it may not use `alloc`, `protect`, I/O, `give`,
  `clone`, or mutate globals. Calls may only target other `comptime` flows and
  the pure math intrinsics (`sqrt`, `floor`, `ceil`, `fabs`, `pow`, `fmod`,
  `len`).
- `comptime expr` forces compile-time evaluation. It is valid only when the
  expression is constant: literals, constant globals, pure operators/casts and
  `comptime` calls.
- Global initializers and shape defaults are constant contexts. A call to a
  non-`comptime` flow there is an error.
- Evaluation is step-limited and call-depth-limited, so a recursive
  `comptime` flow produces a diagnostic instead of hanging the compiler.
- `comptime` arithmetic has the same semantics as runtime arithmetic for the
  result type: signed integer overflow is a compile error, unsigned arithmetic
  wraps at its declared width, and shifts are checked at that width. `as` is
  still an explicit truncating cast, so `300 as i8` is `44`. `f32` results are
  rounded to `f32` precision, so `comptime 1.0 / 3.0` used as `f32` matches the
  value the C backend computes at runtime.

---

## 6. Directions

| Direction | At the declaration | At the call site | Semantics |
|---|---|---|---|
| `<-` | `<- x: T` | `f(x)` — no marker | read-only use of the caller value |
| `->` | `-> x: T` | `f(-> x)` | callee initializes the caller variable |
| `<->` | `<-> x: T` | `f(<-> x)` | callee reads and writes the caller variable |
| `=>` | `=> T` | result | the value flows out of the flow |

Rules:

- A `<-` parameter may be read but never stored or returned. Keep data with
  `clone(x)`.
- The C backend passes `<-` composites by value; slice headers inside them
  still refer to the caller's storage. The checker's borrow rules keep the
  parameter observationally read-only.
- An `->` parameter does not need an initial value at the call site.
- `<->` and `->` are exclusive: the same value cannot be passed twice, nor be
  used by another argument in the same call while a write direction is active.
- Passing the same value twice as `<-` is allowed.
- A call may declare any number of `->` parameters plus one `=> T`.
- `=>` is sugar for a hidden out-parameter; the surface meaning is "result".

---

## 7. Calling flows

```loi
sum <- add(2, 3)
world_new(256, Vec2[ x: 800, y: 600 ], -> w)   // `-> w` is required
step(<-> w, 1.0 / 60)                           // `<-> w` is required
bounce(<-> p, 0)
```

- Mark `->` and `<->` at every call site. Do not mark `<-`.
- All arguments are positional; there are no named arguments.
- A `flow` may be called before its declaration; declarations are collected
  first.

---

## 8. Results

```loi
=> value        // return a value
=>              // return from a flow with no result
```

- `return` is **not** Loi syntax.
- Every path of a non-void flow must produce a result.
- Returning data that points into a `<-` parameter is an error; return
  `clone(...)` or allocate owned data instead.

---

## 9. Control flow

```loi
if condition {
    ...
} else if other {
    ...
} else {
    ...
}

while condition {
    ...
}

for i in 0..n {
    ...
}

for <- item in items {
    ...
}

for <-> item in items {
    ...
}

continue
break
```

- Ranges are half-open: `0..n` visits `0, 1, ..., n - 1`.
- `for <- x in xs` reads each element.
- `for <-> x in xs` mutably aliases each element in turn.
- Conditions are `bool`; there is no truthiness.
- `and`, `or`, `not` are the logical operators. `&&`, `||`, `!` are invalid.
- Equality `==` / `!=` works for scalars, `bool`, values of the same `mark`,
  and `[u8]` (byte-wise). Shapes, choices and other slices must be compared by
  testing fields, variants and payloads explicitly.

---

## 10. Expressions and operators

```loi
a + b   a - b   a * b   a / b   a % b
-a      not flag
a < b   a <= b   a > b   a >= b   a == b   a != b
value is Name.Variant
a and b   a or b
a & b   a | b   a ^ b   ~a
a << n  a >> n
value as i64
xs[i]          // bounds checked
shape.field
xs.len         // u64
```

- Integer arithmetic is checked and platform-independent:
  - signed `+`, `-`, `*` and unary `-` **trap** on overflow;
  - unsigned `+`, `-`, `*` wrap modulo their declared width;
  - `/` and `%` trap on a zero divisor;
  - `MIN / -1` traps, while `MIN % -1` is `0`;
  - `/` truncates toward zero and `%` takes the sign of the dividend.
- `&`, `|`, `^` and `~` require integer operands and produce the checked
  common integer type. `~` is bitwise complement.
- `<<` and `>>` require integer operands. The shift count must be in
  `[0, width)`; an out-of-range count traps with `shift count out of range`.
  `<<` wraps at the declared width, `>>` is logical for unsigned operands and
  arithmetic for signed ones.
- A trap prints `loi: <message> at <line>:<column>` and exits with status 1.
  Literal-only expressions that overflow, divide by a constant zero or shift by
  an out-of-range constant are rejected at compile time instead.
- Compound assignments follow the binary rules, including `%=`, `&=`, `|=`,
  `^=`, `<<=` and `>>=`; `as` remains an unchecked truncating cast and is the
  explicit way to wrap a value.
- `value is Name.Variant` tests a `choice` tag and yields `bool`. Its right
  operand is variant syntax, not a general expression.
- `as` is the only cast form. No `(T)x`.
- Precedence, from lowest to highest:
  `or` → `and` → comparisons (including `is`) → `|` → `^` → `&` → shifts → `+` `-` →
  `*` `/` `%` → `as` → unary `-` `~` `not`. When in doubt, use parentheses;
  the formatter preserves them. Bitwise and shift operators do not accept
  `bool`; use an integer type.
- Slice indexing is bounds checked at runtime and panics on failure.

---

## 11. Ownership

- Every value has exactly one owner and one live name.
- `give(x)` transfers ownership; `x` is dead afterwards. A global, or a
  field/element of a global, cannot be given because there is no enclosing
  function that owns it; use `clone(...)` instead.

  ```loi
  a <- Vec2[ x: 1, y: 2 ]
  b <- give(a)
  // a is dead here
  ```

- `clone(x)` makes an explicit duplicate. It is required for composite values
  and rejected for any value that contains a slice.
- Scalars copy implicitly; `clone` is never needed for them.
- `give` and `clone` are compiler intrinsics, not ordinary flows.
- Two names for the same address (double reference) is a compile error.
- Slices and slice-containing composites move with ownership; there is no
  implicit deep copy.

---

## 12. Slices, allocation and `protect`

```loi
protect {
    mut items: [Item]
    alloc(-> items, Item, 4)
    items[0] = Item[ value: 1 ]

    for i in 0..items.len {
        items[i].value *= 2
    }
}
```

- `alloc(-> xs, T, n)` allocates `n` elements of `T` and writes the slice into
  `xs`. `alloc` is only legal inside a `protect` region (directly or through a
  call whose caller provides one).
- Everything allocated inside `protect { ... }` dies at the closing brace.
- Nothing allocated inside may escape the block. Writing scalar results into
  variables declared **before** the block is how data gets out.
- The arena grows in chunks instead of hitting a fixed 64 MiB limit.
  `--arena-size BYTES` sets the size of the first chunk (default 64 MiB);
  later chunks grow geometrically, and a single allocation larger than the
  current chunk gets its own chunk. Chunks are never moved, so existing slices
  stay valid. Leaving the block restores the arena offset.
- `protect` is memory-only. There is no `defer`, no `free`, no destructor.
- `len(xs)` and `xs.len` both return `u64`.

Slice literals:

```loi
inferred <- [1, 2, 3]                 // [i32]
explicit: [i32] <- [i32: 1, 2, 3]
empty: [i32] <- [i32:]
```

A slice literal has automatic storage in the enclosing lexical block; it must
not escape that block. The checker rejects returning it, assigning it to a
variable declared outside the block, or embedding it in a shape that then
escapes. Bind it to a local in the same block, or use `alloc` for longer-lived
data.

---

## 13. Imports

```loi
import "lib/std.loi"
```

- Imports are textual inclusion resolved by the driver.
- Relative paths are resolved from the importing file's directory; absolute
  paths are accepted.
- Each file is included at most once per compilation unit.
- Imports must appear as top-level items.
- Common import for the standard library: `import "lib/std.loi"`.
- The standard library is explicit; there is no implicit prelude.

---

## 14. Intrinsics and standard library

Compiler intrinsics (always available, cannot be redeclared):

| Intrinsic | Signature |
|---|---|
| `print` | `(<- [u8])` |
| `eprint` | `(<- [u8])` |
| `panic` | `(<- [u8])` |
| `print_int` | `(<- i8 \| i16 \| i32 \| i64 \| u8 \| u16 \| u32 \| u64)` |
| `printf` | `(<- [u8], ...)` | Formatted output; literal format string plus typed varargs |
| `print_float` | `(<- f32 \| f64)` |
| `read_file` | `(<- [u8]) => [u8]` |
| `write_file` | `(<- [u8], <- [u8]) => bool` |
| `read_stdin` | `() => [u8]` |
| `read_line` | `() => [u8]` |
| `sqrt` `floor` `ceil` `fabs` | `(<- f64) => f64` |
| `pow` `fmod` | `(<- f64, <- f64) => f64` |
| `len` | `([T]) => u64` |
| `give` | ownership transfer |
| `clone` | explicit duplicate |
| `alloc` | `(-> [T], T, u64)` inside `protect` |

### File, stdin and numeric output

- `read_file(<- path)` reads the whole file into arena memory. Paths and
  contents are byte slices, not C strings. A missing or unreadable file traps.
- `write_file(<- path, <- data)` writes the exact bytes and returns whether the
  write and close succeeded. It does not trap on ordinary I/O failure.
- `read_stdin()` reads the rest of standard input, including newlines.
- `read_line()` reads one line without the terminating `\n` (and without a
  trailing `\r`).
- `read_file`, `read_stdin` and `read_line` allocate. Call them inside a
  `protect` block, and do not let their result escape that block.
- `print_int` writes only the digits: no newline and no padding. It accepts any
  integer width and prints unsigned widths unsigned.
- `print_float` writes the shortest decimal that round-trips to the same
  value (`0.1`, `1`, `1.5`). To separate values, print a literal:
  `print_int(n) print(" ")`.

### Formatted output

`printf(fmt, ...)` lowers to C `printf`. The first argument must be a string
literal: the checker parses its conversions and accepts exactly one value per
conversion. Supported conversions are `d i u x X o f F e E g G a A s c` and
`%%`; flags, numeric width/precision and length modifiers are forwarded to C.
Dynamic `*` width/precision is rejected because it would consume an extra
vararg that Loi cannot type-check.

`d`, `i` and `c` accept integers; `u`, `x`, `X` and `o` accept integers; the
floating conversions accept floats; `s` accepts a string literal or `cstr`.
Every vararg is cast to the C type its conversion reads, so conversions stay
well-defined; use `ll` for 64-bit values:

```loi
flow main {
    n: i32 <- 42
    big: i64 <- 9000000000
    printf("n=%d big=%lld f=%.2f s=%s pct=100%%\n", n, big, 0.1, "ok")
}
```

```loi
protect {
    mut data: [u8] <- read_file("input.bin")
    if not write_file("copy.bin", data) {
        panic("write failed\n")
    }
}
print_int(42)
print(" ")
print_float(0.1)
print("\n")
```

### Library modules

Every library file is imported explicitly with the canonical `import` form.
Names are prefixed by module (`math_*`, `ctype_*`, `vec_*`, `slice_*`, `str_*`,
`hash_*`, `crc32_*`, `map_*`, `rng_*`, `os_*`, `net_*`) because imports are
textual and there are no namespaces yet.

| File | Helpers | Needs a shim? |
|---|---|---|
| `lib/std.loi` | `min_i32`, `max_i32`, `abs_i32`, `clamp_i32`, `min_f64`, `max_f64`, `abs_f64`, `clamp_f64`, `str_len`, `str_eq`, `str_starts_with`, `str_find` | no |
| `lib/math.loi` | `MATH_PI`, `MATH_E`, `MATH_TAU`, `math_factorial`, generic `math_min<T>` / `math_max<T>` / `math_clamp<T>` / `math_sum<T>`, `math_lerp`, `math_smoothstep`, `math_deg_to_rad`, `math_rad_to_deg`, `math_hypot`, `math_near`, `math_sign_i32`, `math_gcd_u64`, `math_pow_u64`, `math_dot_f64` | no |
| `lib/ctype.loi` | `ctype_is_digit`, `ctype_is_upper`, `ctype_is_lower`, `ctype_is_alpha`, `ctype_is_alnum`, `ctype_is_hex_digit`, `ctype_is_space`, `ctype_is_ascii`, `ctype_to_lower`, `ctype_to_upper`, `ctype_hex_value`, `ctype_parse_u64`, `ctype_count_digits`, `ctype_equals_ignore_case` | no |
| `lib/vec.loi` | `Vec<T>`, `vec_new<T>`, `vec_len<T>`, `vec_capacity<T>`, `vec_is_empty<T>`, `vec_get<T>`, `vec_set<T>`, `vec_push<T>`, `vec_pop<T>`, `vec_find<T>`, `vec_from_slice<T>`, `vec_as_slice<T>`, `vec_reserve<T>`, `vec_clear<T>`, `vec_reverse<T>`, `vec_sort<T>`, `vec_binary_search<T>` | no; call inside `protect` |
| `lib/slice.loi` | `slice_len<T>`, `slice_is_empty<T>`, `slice_get<T>`, `slice_first<T>`, `slice_last<T>`, `slice_find<T>`, `slice_equals_u8`, `slice_clone<T>`, `slice_copy<T>`, `slice_reverse<T>` | no; allocators need `protect` |
| `lib/string.loi` | `STRING_MAX_LEN`, `str_is_ascii_space`, `str_trim`, `str_concat`, `str_reverse`, `str_repeat`, `str_count`, `str_index_of`, `str_contains` | no; owned results need `protect` |
| `lib/hash.loi` | `HASH_OFFSET`, `HASH_PRIME`, `hash_bytes_seeded`, `hash_bytes`, `hash_str`, `hash_u64_seeded`, `hash_u64`, `hash_pair_u64` | no |
| `lib/crc32.loi` | `CRC32_POLY`, `CRC32_INIT`, `CRC32_XOROUT`, `CRC32_TABLE`, `crc32_update`, `crc32_extend`, `crc32_bytes` | no |
| `lib/map.loi` | `IntMap<V>`, `StrMap<V>`, `map_int_new<V>`, `map_int_len<V>`, `map_int_capacity<V>`, `map_int_contains<V>`, `map_int_get<V>`, `map_int_get_or<V>`, `map_int_lookup<V>`, `map_int_set<V>`, `map_int_remove<V>`, `map_int_clear<V>`, and the matching `map_str_*` flows | no; mutators need `protect` |
| `lib/random.loi` | `Rng`, `rng_new`, `rng_state`, `rng_next_u64`, `rng_next_u32`, `rng_below_u64`, `rng_pick<T>`, `rng_shuffle<T>`; a zero seed uses `RNG_FALLBACK_SEED` | no |
| `lib/os.loi` | `os_pid`, `os_unix_time`, `os_sleep`, `os_has_env`, `os_env_i32`, `os_errno`, `os_exit` | `lib/os_shim.c` |
| `lib/net.loi` | `NetIpv4Result`, `net_ipv4`, `net_ipv4_octet`, `net_port_is_valid`, `net_parse_ipv4`, `net_tcp_connect`, `net_close`, `net_send`, `net_recv` | `lib/net_shim.c` |

`lib/math.loi` and `lib/ctype.loi` are pure Loi: the math module uses generic
flows and a `comptime flow` for factorials. Documented library flows and their
shapes are `open` so generated headers can expose them; internal helpers stay
`sealed`. An exported allocator still needs a caller's `protect` region — Loi
callers are checked, and foreign callers follow the same contract.
`lib/vec.loi` and `lib/slice.loi` are pure Loi; `Vec<T>` stores cloned values,
so `T` must be cloneable without slice fields, and ordering/search flows accept
numeric `T` only. The arena keeps older growth blocks until `protect` ends, so
pre-size with `vec_reserve<T>` when the final length is known to avoid repeated
growth and its peak memory. `lib/string.loi` returns owned `[u8]` values, so
its transforming flows need the caller's `protect`; its queries do not.
`str_repeat` rejects an overflowing count with a flow-naming error.
`lib/hash.loi` is pure: it implements 64-bit FNV-1a, hashes `u64` values as
little-endian bytes, and needs no `protect`. `lib/crc32.loi` is also pure:
`comptime` flows generate a 16-entry reflected nibble table stored as a static
slice. `crc32_update` folds raw chunk state, while `crc32_extend` continues an
already finalized CRC in the same way as zlib's `crc32(crc, buf)`.
`lib/map.loi` provides `IntMap<V>` and `StrMap<V>` with linear scans and
value-only generics: `V` must be cloneable without slice fields. `StrMap<V>`
stores byte keys in one contiguous arena buffer, so no slice-of-slices value is
formed. Mutations need the caller's `protect`. `map_int_get` / `map_str_get`
abort on a missing key with a flow-naming message; prefer `map_*_get_or` for a
fallback value or `map_*_lookup` for a `found` status plus a default, both of
which scan the map once. `lib/random.loi` is a pure, seeded xorshift64*
generator with `u64` / `u32` draws, bounded draws, picking and in-place
Fisher–Yates shuffling; it needs no `protect`, is not cryptographically secure,
and replaces a zero seed with the nonzero `RNG_FALLBACK_SEED`. Loi has no
character literals, so byte queries use numeric values while substring queries
use string slices; see `development/FRICTION.md`.
`lib/os.loi` and `lib/net.loi` are
thin POSIX bindings: their `extern flow` declarations call symbols defined in
the matching C shim, so a program that imports them must compile the shim:

```sh
loi build main.loi -o main --c-source lib/os_shim.c --c-source lib/net_shim.c
```

The socket helpers expose integer descriptors only; no foreign pointer type
enters ordinary Loi values. The shims are POSIX-only and are not part of the
closed intrinsic set.

---

## 15. Foreign C interop (`opaque`, `cstr`, `extern flow`)

Foreign declarations live in **vendored** files. Hand-written application code
imports them; it does not declare its own `extern` items unless it is writing a
binding.

### 15.1 Opaque handles

```loi
opaque SDL_Window
opaque SDL_Renderer
```

- An `opaque Name` is a foreign handle. The checker treats it as a distinct,
  copyable scalar; the backend represents it as `void *`.
- It cannot be dereferenced, indexed or inspected. It can only be stored,
  passed to flows and returned.
- `opaque` declarations may be `open` (exported) or `sealed` (default).

### 15.2 C strings

```loi
extern flow SDL_CreateWindow
    <- title: cstr
    <- x: i32
    <- y: i32
    <- w: i32
    <- h: i32
    <- flags: u32
    => SDL_Window
```

- `cstr` is an FFI-only, copyable handle to a NUL-terminated byte sequence.
- A string literal may initialize a `cstr` directly:

  ```loi
  title: cstr <- "Loi + SDL2"
  ```

- `cstr` values cannot be indexed, measured or compared in Loi. Convert only in
  a vendored shim when a real byte string is needed.

### 15.3 Extern flows

```loi
extern flow SDL_Init
    <- flags: u32
    => i32
```

- An `extern flow` has a signature but **no body**.
- The Loi name is the C symbol by default. To call a different C symbol
  (typically a generated shim), use `as`:

  ```loi
  extern flow sdl_create_window as loi_sdl_create_window
      <- title: cstr
      ...
      => SDL_Window
  ```

- Directions lower exactly like Loi flows: `<-` passes by value, `->`/`<->`
  pass an address.
- Supported parameter/result types: scalars, `bool`, `opaque` handles, `cstr`,
  and shapes whose fields are themselves FFI-safe. Slices, callbacks,
  variadics and unions require a vendored C shim. Loi flows cannot declare
  variadic parameters; `printf` is the sole variadic intrinsic (section 14).
- Extern flows are not checked for a body, ownership effects or allocation.
  Their contracts are the binding author's responsibility.

### 15.4 Building with C libraries

```sh
loi build game.loi -o game \
    --c-source vendor/sdl2/sdl2_shim.c \
    --cc-flag -Ivendor/sdl2/include \
    --cc-flag -lSDL2

# raylib 5.5
loi build game.loi -o game \
    --c-source vendor/raylib/raylib_shim.c \
    --cc-flag -Ivendor/raylib/include \
    --cc-flag -lraylib \
    --cc-flag -lm
```

- `--c-source FILE` compiles and links an extra C source (a vendored shim).
- `--cc-flag FLAG` forwards a flag to the C compiler and linker; use it for
  include paths and `-l` libraries. On link steps these flags follow the
  objects, so `-lSDL2` works in the expected order.
- `gen` still emits only the Loi translation unit. Extern symbols are resolved
  at link time.

### 15.5 Generating bindings from a C header

`tools/bindgen/bindgen.py` turns a C header into a vendored `.loi` file plus,
when needed, a C shim:

```sh
python3 tools/bindgen/bindgen.py \
    --header vendor/sdl2/include/SDL2/SDL.h \
    --include vendor/sdl2/include \
    --include vendor/sdl2/include/SDL2 \
    --output vendor/sdl2/sdl2.loi \
    --shim vendor/sdl2/sdl2_shim.c \
    --skipped vendor/sdl2/skipped.txt \
    --module sdl2 \
    --prefix SDL_
```

- Scalars and enums become `i32`/`u32`/... parameters and results.
- Complete, pointer-free C records become `shape`s; incomplete or unsafe
  records become `opaque` handles.
- Integer `#define` macros and enum constants become typed globals.
- Calls needing layout conversion are emitted as `extern flow ... as
  loi_bind_<module>_<name>` and implemented in the generated shim with
  `memcpy` plus `_Static_assert` size/alignment checks.
- Unsupported declarations are never guessed: callbacks, variadics, unions,
  bitfields, inline arrays, pointer-to-pointer, mutable `char *` buffers,
  wide strings and pointers to unsafe records are skipped and listed in the
  skip report.

Generated bindings are conservative by design. Add a small hand-written shim
for the specific unsupported calls a program needs. `vendor/sdl2/` and
`vendor/raylib/` are complete worked examples, including their skip reports and
build instructions.

---

## 16. Formatting rules (`loi fmt`)

- 4-space indentation, no tabs.
- One statement per line.
- Shape, mark and choice bodies and flow signatures are vertical when they do
  not fit on one line; `loi fmt` chooses the canonical layout.
- Shape literals keep their single-line form when written that way.
- The formatter emits Flow Syntax only. Running `loi fmt` twice must be
  byte-identical (idempotent).
- `loi fmt` preserves `//` comments: standalone comments stay above the next
  declaration/statement, and trailing comments stay on their line. Comments
  inside multi-line expressions may move to the next statement boundary.
  Formatting is idempotent after that first normalization.
- `format_program` in the Rust API still drops comments; the CLI uses the
  comment-aware path.
- Do not hand-align fields with spaces; alignment is cosmetic and may change.

---

## 17. Never use (removed syntax)

The following forms are **not Loi** and must never appear in new code, tests,
examples, docs or generated bindings:

| Forbidden | Canonical |
|---|---|
| `fun f(...)` | `flow f ...` |
| `interface Name { ... }` | `shape Name [ ... ]` |
| `enum Name { ... }` | `mark Name [ ... ]` |
| `union Name { ... }` | `choice Name [ Variant: T, ... ]` |
| `let x = v` | `x <- v` |
| `var x = v` | `mut x <- v` |
| `in x: T` | `<- x: T` |
| `out x: T` | `-> x: T` |
| `inout x: T` | `<-> x: T` |
| `-> T` as a result | `=> T` |
| `return v` | `=> v` |
| `move(x)` | `give(x)` |
| `copy(x)` | `clone(x)` |
| `visible` / `hidden` | `open` / `sealed` |
| `[]T` | `[T]` |
| `T{ field: v }` | `T[ field: v ]` |
| `[]T{ a, b }` | `[T: a, b]` |
| `for in x in xs` | `for <- x in xs` |
| `for inout x in xs` | `for <-> x in xs` |
| `f(out x)` | `f(-> x)` |
| `alloc(out xs, ...)` | `alloc(-> xs, ...)` |
| `&&`, `\|\|`, `!` | `and`, `or`, `not` |
| `(T)x` | `x as T` |
| `&x` (address-of), `*p` (dereference), `nil` | no pointers in Loi; `&` is bitwise AND only |
| `.lum` files | `.loi` files |

---

## 18. Complete canonical example

```loi
// Braces are scopes, brackets are data, arrows are movement.

shape Vec2 [ x: f64, y: f64 ]

shape Particle [
    pos:   Vec2
    vel:   Vec2
    mass:  f64
    alive: bool = true
]

open flow spawn
    <- at: Vec2
    -> p: Particle
{
    p.pos = clone(at)
    p.vel = Vec2[ x: 0, y: 0 ]
    p.mass = 1
}

sealed flow clamp
    <- v: f64
    <- lo: f64
    <- hi: f64
    => f64
{
    if v < lo { => lo }
    if v > hi { => hi }
    => v
}

open flow main
    => i32
{
    mut mass: f64
    protect {
        mut p: Particle
        spawn(Vec2[ x: 10, y: 20 ], -> p)
        p.pos.x = clamp(p.pos.x + 5, 0, 100)
        mass = p.mass
    }
    if mass != 1 {
        => 1
    }
    => 0
}
```

Reference programs:

- `examples/flow_syntax.loi` — complete syntax tour.
- `examples/particles.loi` — same program, canonical output of `loi fmt`.
- `examples/tictactoe.loi` — deterministic self-play.
- `examples/language_basics.loi` — imports, globals, strings and slices.
- `examples/marks.loi` — finite tag sets, equality and state transitions.
- `examples/choices.loi` — tagged payloads, `is` tests and checked reads.
- `examples/systems.loi` — bitwise flags, shifts and file/stdin I/O.
- `lib/std.loi` — standard library written in canonical Loi.
