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
  alloc  and  as  break  clone  comptime  continue  else  extern  false
  flow   for  give  if     import  in      mut       not   opaque
  open   or   protect  sealed  shape  true  while
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
| `Name` | a declared `shape` |
| `Name` | a declared `opaque` foreign handle |
| `cstr` | NUL-terminated C string handle (FFI only) |

There is no `void`, no pointer type, no array type and no `nil`. A function
with no `=> T` returns nothing.

- Slices are written **`[T]`**, never `[]T`.
- Strings are `[u8]` at runtime. A string literal is a `[u8]` value.
- `cstr` values are constructed only from string literals or returned by
  `extern flow` declarations. They can be passed back to C but not indexed,
  measured or compared.

---

## 3. Shapes — data only

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

Global initializers must be compile-time constants. Only scalar, `bool` and
`[u8]`-from-string globals are emitted by the C backend today.

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
- Equality `==` / `!=` works for scalars, `bool`, and `[u8]` (byte-wise).
  Shapes and other slices must be compared field by field.

---

## 10. Expressions and operators

```loi
a + b   a - b   a * b   a / b   a % b
-a      not flag
a < b   a <= b   a > b   a >= b   a == b   a != b
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
- `as` is the only cast form. No `(T)x`.
- Precedence, from lowest to highest:
  `or` → `and` → comparisons → `|` → `^` → `&` → shifts → `+` `-` →
  `*` `/` `%` → `as` → unary `-` `~` `not`. When in doubt, use parentheses;
  the formatter preserves them. Bitwise and shift operators do not accept
  `bool`; use an integer type.
- Slice indexing is bounds checked at runtime and panics on failure.

---

## 11. Ownership

- Every value has exactly one owner and one live name.
- `give(x)` transfers ownership; `x` is dead afterwards.

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

`lib/std.loi` provides ordinary Loi helpers with prefixed names:
`min_i32`, `max_i32`, `abs_i32`, `clamp_i32`, `min_f64`, `max_f64`, `abs_f64`,
`clamp_f64`, `str_len`, `str_eq`, `str_starts_with`, `str_find`.

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
  variadics and unions require a vendored C shim.
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
- Shape bodies and flow signatures are vertical when they do not fit on one
  line; `loi fmt` chooses the canonical layout.
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
- `examples/systems.loi` — bitwise flags, shifts and file/stdin I/O.
- `lib/std.loi` — standard library written in canonical Loi.
