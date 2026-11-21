# Loi — Canonical Syntax (SINTAXE)

**This file is the canonical surface reference.** Every example, test, library,
website snippet and generated binding must use exactly these forms. When in
doubt, this file wins; if this file and the compiler disagree, fix one of them
in the same change.

The language specification and rationale live in
[`development/LOI_SPEC.md`](development/LOI_SPEC.md). Current compiler state
lives in [`development/PROGRESS.md`](development/PROGRESS.md). Building the
compiler requires Rust 1.88 or newer, as declared by `rust-version` in
`Cargo.toml`.

**Rule of thumb:** *braces are scopes, brackets are data, arrows are movement.*

---

## 1. Lexical rules

- Source files end in `.loi`.
- Comments are `// ...` and run to the end of the line.
- Statements are separated by newlines. A `}` also ends the previous statement.
- Keywords (the complete set):

  ```text
  alloc  and  as  break  choice  clone  comptime  continue  else  extern
  false  flow   for  give   guard   if      import  in   is    mark    mut
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
  A qualified name such as `math::sum` or `geo::Point` is one identifier token
  whose parts are separated by `::`; member access still uses `.`
  (`geo::Color.Red`).
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
| `callback` | C function pointer with fixed ABI (FFI only) |
| `callback(T, ...) => R` | C function pointer with an explicit signature (FFI only); `R` may be `void` |

There is no general `void`, no pointer type, no array type and no `nil`. A
function with no `=> T` returns nothing; `void` is written only as a callback
result.

- Slices are written **`[T]`**, never `[]T`.
- Generic instantiations are written **`Vec<T>`**. There is no type inference:
  every generic call and literal spells its type arguments.
- Strings are `[u8]` at runtime. A string literal is a `[u8]` value.
- `cstr` values are constructed only from string literals or returned by
  `extern flow` declarations. They can be passed back to C but not indexed,
  measured or compared.
- `callback` values are constructed only with `callback(flow)` (section 15.6).
  The signature is inferred from the target flow and must match the declared
  callback type at each use. They are copyable and may be stored in locals,
  shapes and slices, but Loi cannot call, index or dereference them.

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
- Marks are scalar-like values. `==`, `!=` and the ordering operators are
  defined only between two values of the **same** mark; variants compare in
  declaration order. Arithmetic is not defined for marks.
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
  They may carry constraints (`shape Slot<T: Copy>`); see §5.4.
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
expressions, constant globals, shape/slice literals, mark variants, choice
constructors and `comptime` calls. The C backend emits them as static
initializers, including nested shapes, slices, strings, marks and choices.
These initializers are strict C11: shapes use brace/designated initializers (no
compound literals), choices use brace initializers with a tag and payload, and
slice constants are backed by named `static` arrays emitted before the globals.
Globals can be read or cloned, but `give` cannot transfer ownership out of a
global or out of a field/element of one; use `clone(...)` instead.

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

- Declare type parameters after the flow name: `flow name<T, U>`. They may
  carry bounds; see §5.4.
- Calls spell their type arguments: `sum<i32>(xs)`. There is no inference, and
  a generic flow without type arguments is an error.
- Passing a type argument to a non-generic flow is an error, as is the wrong
  number of arguments.
- Generic flows are monomorphized per instantiation, so each concrete type
  argument produces a separate compiled function. Unused generic flows are
  checked with `i32` placeholders, and recursive instantiations hit the same
  depth limit as generic shapes.

### 5.4 Generic constraints

```loi
flow copy_row<T: Clone>
    <- row: T
    => T
{
    => clone(row)
}

flow find<T: Clone + Eq>
    <- xs: [T]
    <- needle: T
    => i64
{
    for i in 0..xs.len {
        if xs[i] == needle {
            => i as i64
        }
    }
    => -1
}

shape Slot<T: Copy> [
    value: T
]

slot <- Slot<i32>[ value: 7 ]
index <- find<i32>([i32: 1, 2], 2)
```

- A type parameter may list constraints after a colon: `T: Clone`,
  `T: Clone + Eq`, `T: Clone + Copy + Ord`. Separate several parameters with
  commas: `find<T: Eq, U>`. Bounds apply to shapes and flows alike.
- `Clone`, `Copy`, `Eq` and `Ord` are the only accepted constraint names in this
  version. They are ordinary identifiers recognized only in bound position,
  not new keywords; any other name is a parse error naming the unknown
  constraint.
- A bound is checked when the template is instantiated with concrete type
  arguments, and the diagnostic points at the call or type use that triggered
  it, using source spelling (`Vec<i32>`, not the mangled `Vec__i32`). The
  exact set of types satisfying each bound is a language rule recorded in
  `LOI_SPEC.md` §5.1a.
- Bounds are documentation and validation only: they do not change mangled
  names, generated C or the ABI. `Clone` is satisfied by every Loi type,
  but cloning a value with slices still allocates and needs `protect`.
- `Ord` guarantees `<`, `<=`, `>` and `>=` for the instantiated type;
  `vec_sort<T>` and `vec_binary_search<T>` declare it. Opaque handles and
  callbacks have no order.

### 5.5 `comptime`

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
  expression is constant: literals, constant globals, mark variants, choice
  constructors, pure operators/casts (including `is` and equality) and
  `comptime` calls.
- Global initializers, shape defaults and slice elements are constant contexts.
  A call to a non-`comptime` flow there is an error; the diagnostic underlines
  the offending call rather than the whole initializer.
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

- A `<-` parameter may be read but never stored. Its whole value cannot be
  returned; use `clone(x)` for an owned copy. A slice-typed view of its storage
  may be returned — see §6 "Borrowed slice views".
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
- Equality `==` / `!=` is structural: numbers, `bool`, values of the same
  `mark`, `cstr`, opaque handles, shapes, choices and slices whose elements
  support equality. Shapes compare fields in declaration order, choices
  compare the active variant before its payload, and slices compare length and
  then elements. Operands must have the same type, except that numeric
  operands of different widths coerce to their common type.
- Ordering `<`, `<=`, `>` and `>=` follows the same lexicographic rule for
  numbers, `bool`, marks, `cstr`, shapes, choices and slices. Opaque handles
  and callbacks are not orderable; callbacks cannot be compared at all.

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
- `==`, `!=`, `<`, `<=`, `>` and `>=` compare structurally; the full rule is
  in §9.
- `as` is the only cast form. No `(T)x`.
- Precedence, from lowest to highest:
  `or` → `and` → comparisons (including `is`) → `|` → `^` → `&` → shifts → `+` `-` →
  `*` `/` `%` → `as` → unary `-` `~` `not`. When in doubt, use parentheses;
  the formatter preserves them. Bitwise and shift operators do not accept
  `bool`; use an integer type.
- Slice indexing is bounds checked at runtime and panics on failure.

### Evaluation order

- Operands are evaluated left to right. This covers binary operators, call and
  intrinsic arguments, slice and shape literal elements, an assignment target
  before its value, an index base before its index, and a range start before its
  end.
- Each operand's value is captured before a later operand runs. A program whose
  result would depend on C's unspecified operand or argument order is therefore
  well defined, not rejected.
- `and` and `or` still short-circuit: the right operand runs only when the left
  one does not decide the result.
- A compound assignment evaluates its target place first; when the right operand
  has effects, the target's old value is captured before that operand runs.
- The C backend uses generated temporaries for this order. No source syntax
  changes: `xs[first()] = second()` always calls `first()` before `second()`.

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

- `clone(x)` makes an explicit duplicate. It is required for composite values.
  The source is only read, so it stays live after the clone.
- `clone` is a **deep copy**: when the value contains slices, every slice gets
  fresh arena storage, so mutating the clone never changes the source (and vice
  versa). Shapes and nested shapes are cloned recursively. Types without slices
  are still copied bit-wise.
- A deep clone allocates, so it is only legal inside a `protect` region (directly
  or through a flow whose caller provides one). Its result may not escape that
  region.
- Scalars copy implicitly; `clone` is never needed for them.
- `give(x.field)` and `give(x[i])` consume only that place when it holds a
  composite value. Siblings stay live, and a later write to the consumed place
  reinitializes it. Scalar fields and elements are copied without killing the
  owner; `give(x)` of the whole variable still kills `x`.

  ```loi
  tmp: u64 <- give(xs[i])
  xs[i] = give(xs[j])
  xs[j] = tmp
  ```

- A runtime index in a consumed path is a wildcard: after `give(xs[i])` of a
  composite element, every element of `xs` is considered moved until the whole
  slice is reinitialized.
- `give` and `clone` are compiler intrinsics, not ordinary flows.
- Two names for the same address (double reference) is a compile error.
- Slices and slice-containing composites move with ownership; there is no
  implicit deep copy.

### Lexical read views

A `<-` parameter is a read-only view of the caller's value that lives for the
call. The checker summarizes which paths each flow reads and forwards that
summary through flows that pass a `<-` parameter on to another flow. A
partially consumed composite may be passed as a read-only view when those reads
avoid every consumed path; a flow that reads a moved path is still rejected.

```loi
shape Pair [
    left: [u8]
    right: u64
]

flow right_only
    <- p: Pair
    => u64
{
    => p.right         // reads only p.right
}

flow main
    => i32
{
    protect {
        mut p: Pair
        alloc(-> p.left, u8, 1)
        p.right = 7
        taken <- give(p.left)       // p.left is consumed; p.right stays live
        print_int(right_only(p))    // ok: the view touches only p.right
        print_int(taken[0])
    }
    => 0
}
```

The view is lexical: it lasts for the call. A flow may not store the `<-`
parameter or return a non-slice part of it; `clone(...)` makes an owned copy.

### Borrowed slice views

`view(xs, start, end)` is a compiler intrinsic that returns a zero-copy
subslice of `xs`. It requires `start <= end <= xs.len` (checked at runtime) and
accepts only `<-` arguments. The result aliases the caller's storage: no
element is copied and no arena allocation happens. Because a slice is itself a
view over storage, a flow may return one borrowed from a `<-` parameter:

```loi
open flow window
    <- data: [u8]
    <- from: u64
    <- to: u64
    => [u8]
{
    => view(data, from, to)
}

flow main
    => i32
{
    protect {
        frame: [u8] <- [u8: 1, 2, 3, 4]
        head <- window(frame, 0, 2)
        tail <- view(head, 1, 2)     // a view of a view stays borrowed
        print_int(head[0] + tail[0]) // 1 + 2
    }
    => 0
}
```

Loans follow the value, not the flow boundary:

- A view keeps its source borrowed while the variable holding the view is in
  scope. Writing to, moving from, or passing as `<->` any overlapping path of
  the source is a compile error. The diagnostic points at the view creation.
- The loan ends when the view's scope ends. A source may be written again
  after that brace, provided the view did not escape it.
- Views may be returned from flows and forwarded through any number of flows.
  A view can reach a caller inside a returned composite or through a `->`
  parameter whose type contains a slice; in each case the caller's argument
  becomes the borrowed source at the call site.
- A loan tracks both its source and the slot of the holder that stores the
  view. Overwriting that slot releases the loan; writing a sibling field does
  not, and selecting a sub-place of a view keeps the loan with the selected
  value.
- `clone(x)` does not borrow: its result owns fresh storage.
- The source's storage region still applies. A view of a block-local slice
  literal cannot be stored in a variable that outlives that block.
- `lib/slice.loi` exposes `slice_view<T>` and `lib/vec.loi` exposes
  `vec_view<T>` for the same zero-copy operation. `vec_as_slice<T>` remains the
  detached, allocating copy.

### Linear resources and `guard`

`guard` binds a non-memory resource and releases it once on every exit path.
The release flow is found by convention from the shape name: `Connection` uses
`connection_release`, `HttpClient` uses `http_client_release`. Its signature
must be exactly `flow <name>_release(<-> value: Shape)` with no result.

```loi
sealed flow connection_release
    <-> conn: Connection
{
    conn.alive = false
}

flow exchange
    <- id: u64
    => u64
{
    guard conn: Connection <- connect(id) {
        if conn.id == 0 { => 0 }    // connection_release runs first
        print_int(conn.id)
    }
    => 0
}
```

- The type annotation is optional; `guard conn <- connect(id)` infers it from
  the initializer.
- The release is inserted on normal completion, `=>`, `break` and `continue`.
  Nested guards unwind in LIFO order, and `break`/`continue` release only the
  guards entered inside the loop being left.
- The binding is immutable. Explicitly calling the release flow on it, moving
  it out with `give` or `<->`, or returning it is a compile error. Returning a
  scalar field is fine.
- `guard` requires a `shape`. `protect` is separate and still memory-only;
  release runs before any enclosing `protect` arena is restored. `guard` is
  not allowed in `comptime` flows.

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
- `protect` is memory-only. There is no `defer`, no `free`, no destructor;
  non-memory resources use `guard` (§11).
- `len(xs)` and `xs.len` both return `u64`. `xs.len` is also writable through a
  mutable place (including a `->` or `<->` parameter), so a caller-owned buffer
  can be resized by a callee. The new length is used by runtime bounds checks;
  writing a length larger than the allocated storage is the programmer's
  responsibility. A `<-` parameter stays read-only.

Slice literals:

```loi
inferred <- [1, 2, 3]                 // [i32]
explicit: [i32] <- [i32: 1, 2, 3]
empty: [i32] <- [i32:]
```

A **non-empty** slice literal has automatic storage in the enclosing lexical
block; it must not escape that block. The checker rejects returning it,
assigning it to a variable declared outside the block, or embedding it in a
shape that then escapes. Bind it to a local in the same block, or use `alloc`
for longer-lived data.

An **empty** slice literal (`[T:]`) needs no storage: it is the null slice
`{ data: NULL, len: 0 }`. It may escape freely — be returned, assigned into an
out-parameter, or stored in a shape declared outside a `protect` block. This is
how an empty collection can be constructed without allocating; the first
growth then uses `alloc` inside `protect`. Indexing an empty slice is still
bounds-checked at runtime, so it aborts rather than dereferencing null.

---

## 13. Imports and modules

```loi
import "lib/std.loi"              // binds module `std`
import "lib/math.loi" as m        // binds module `m`

sum <- m::math_sum<u8>([u8: 1, 2, 3])
p   <- m::Point[ x: 1, y: 2 ]
```

- Each source file is a **module**. Its default name is the file stem
  (`math.loi` → `math`); `as name` overrides that name for one import.
- An import brings the imported module's items into the program and binds its
  name in the importing file. Relative paths resolve from the importing file's
  directory; absolute paths are accepted. Each canonical file is loaded once
  per compilation unit; a diamond import does not duplicate declarations.
- A module's items may be named **unqualified**, exactly as before, or
  **qualified** as `module::item` in value, type and constructor positions
  (`m::flow`, `m::Shape`, `m::MARK.Variant`, `m::gen<i32>(x)`).
- `open` items are usable from other modules. `sealed` items (the default) are
  private: referring to one from another module is an error, qualified or not.
  A module may always use its own sealed items unqualified.
- Import cycles are errors. A file that appears twice under different aliases
  is one module with two names.
- Two modules may declare the same item name. An unqualified name binds to the
  current module when it declares that name, and to the only other declaration
  when the name is unique elsewhere. If several modules declare it and the
  current module does not, the name is ambiguous and must be qualified.
- Names shared by several modules get a module-qualified internal name and
  distinct C symbols (`a::helper` → `loi_fn_a__helper`). Names declared in only
  one module keep their original spelling and C symbols, so single-module
  programs are ABI-compatible. `main` and compiler intrinsics cannot be mangled;
  `extern flow ... as <symbol>` keeps its explicit C symbol. See `LOI_SPEC.md`
  §4.8 for the full rule.
- Imports must appear as top-level items. The standard library is explicit;
  there is no implicit prelude. Common import: `import "lib/std.loi"`.

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
| `view` | `(<- [T], <- u64, <- u64) => [T]` — zero-copy borrowed subslice |
| `callback` | `(<- flow) => callback(T, ...) => R` — C function pointer whose signature comes from the flow (section 15.6) |
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
- `read_stdin()` reads until EOF. Framed protocols need `lib/stdio.loi`
  instead, below.
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

### Deterministic stdio byte I/O

`lib/stdio.loi` is an explicit library for byte-oriented protocols. Import it
and link `lib/stdio_shim.c`:

```loi
import "lib/stdio.loi"

open flow echo_one_frame
    => i32
{
    protect {
        mut body: [u8]
        alloc(-> body, u8, 5)
        got <- stdio_read_exact(<-> body)
        if got != 5 {
            => 1
        }
        written <- stdio_write_all(body)
        if written != got {
            => 1
        }
        stdio_flush()
    }
    => 0
}
```

```sh
loi build main.loi -o main --c-source lib/stdio_shim.c
```

- `stdio_read_exact(<-> data) => i64` fills `data`, stopping when it is full, at
  EOF, or on error. It returns the byte count, or `-1` on error. A short
  non-negative result means EOF arrived before the buffer was full.
- `stdio_write_all(<- data) => i64` writes all bytes to standard output and
  returns the byte count, or `-1` on error.
- `stdio_flush()` flushes standard output. Call it before waiting for the next
  request, because C stdio buffers output when stdout is a pipe.

All three flows use the same C `FILE*` streams as `print`, `printf` and
`read_line`; do not mix them with the raw descriptors from `lib/os.loi` or
`lib/net.loi` on the same stream. A realistic LSP-style frame echo is
`examples/stdio_frames.loi`; `Content-Length: 0` is that demo's shutdown
sentinel.

### JSON documents

`lib/json.loi` parses and serializes JSON in pure Loi, with no shim. It uses a
node table plus a byte arena for decoded strings; node handles are indices into
the document, not pointers.

```loi
import "lib/json.loi"

open flow request_method
    <- request: [u8]
    => bool
{
    protect {
        mut doc: JsonDoc
        ok <- json_parse(request, -> doc)
        if not ok {
            => false
        }
        root <- json_root(<- doc)
        method <- json_object_get(<- doc, root, "method")
        print(json_string(<- doc, method))
        print("\n")
    }
    => true
}
```

- `json_parse(<- text, -> doc) => bool` returns `false` on malformed input or
  trailing bytes. It clones the input and allocates tables, so call it inside
  `protect`. After a failed parse, `json_root(<- doc)` is `-1`.
- `json_kind(<- doc, <- node) => JsonKind` reports `Null`, `Bool`, `Number`,
  `String`, `Array` or `Object`; invalid handles report `Null`.
- `json_bool`, `json_string` and `json_number_text` read scalar nodes. Strings
  are decoded from `\"`, `\\`, `\/`, `\n`, `\r`, `\t`, `\b`, `\f` and `\uXXXX`
  (including surrogate pairs) to UTF-8; numbers keep their raw lexeme, and
  `json_number_i64` returns the integral part truncated toward zero.
- `json_array_len` / `json_array_get` walk arrays, and `json_object_get` finds a
  member by decoded key. Both accessors return `-1` on a miss; object order is
  preserved.
- `json_stringify(<- doc) => [u8]` emits compact JSON with decoded strings and
  raw numbers. It allocates, so call it inside `protect`.

A realistic `initialize` request decoder is `examples/json_rpc.loi`.

### Library modules

Every library file is imported explicitly with the canonical `import` form.
Each file is a module (§13); items may be named unqualified or as
`module::item`. Library names keep their established prefixes (`math_*`,
`ctype_*`, `vec_*`, `slice_*`, `str_*`, `option_*`, `result_*`, `hash_*`,
`crc32_*`, `map_*`, `rng_*`, `os_*`, `net_*`, `stdio_*`, `json_*`) so their C symbols stay stable even
when a user module declares the same name; module-qualified mangling applies only
to names that actually repeat.

| File | Helpers | Needs a shim? |
|---|---|---|
| `lib/std.loi` | `min_i32`, `max_i32`, `abs_i32`, `clamp_i32`, `min_f64`, `max_f64`, `abs_f64`, `clamp_f64`, `str_len`, `str_eq`, `str_starts_with`, `str_find` | no |
| `lib/math.loi` | `MATH_PI`, `MATH_E`, `MATH_TAU`, `math_factorial`, generic `math_min<T>` / `math_max<T>` / `math_clamp<T>` / `math_sum<T>`, `math_lerp`, `math_smoothstep`, `math_deg_to_rad`, `math_rad_to_deg`, `math_hypot`, `math_near`, `math_sign_i32`, `math_gcd_u64`, `math_pow_u64`, `math_dot_f64` | no |
| `lib/ctype.loi` | `ctype_is_digit`, `ctype_is_upper`, `ctype_is_lower`, `ctype_is_alpha`, `ctype_is_alnum`, `ctype_is_hex_digit`, `ctype_is_space`, `ctype_is_ascii`, `ctype_to_lower`, `ctype_to_upper`, `ctype_hex_value`, `ctype_parse_u64`, `ctype_count_digits`, `ctype_equals_ignore_case` | no |
| `lib/option.loi` | `Option<T>`, `option_none<T>`, `option_some<T>`, `option_is_some<T>`, `option_is_none<T>`, `option_get_or<T>`, `option_unwrap<T>` | no; cloning slice payloads needs `protect` |
| `lib/result.loi` | `Result<T, E>`, `result_ok<T, E>`, `result_err<T, E>`, `result_is_ok<T, E>`, `result_is_err<T, E>`, `result_unwrap_or<T, E>`, `result_unwrap<T, E>`, `result_unwrap_err<T, E>` | no; cloning slice payloads needs `protect` |
| `lib/vec.loi` | `Vec<T>`, `vec_new<T>`, `vec_len<T>`, `vec_capacity<T>`, `vec_is_empty<T>`, `vec_get<T>`, `vec_set<T>`, `vec_push<T>`, `vec_pop<T>`, `vec_find<T>`, `vec_from_slice<T>`, `vec_view<T>`, `vec_as_slice<T>`, `vec_reserve<T>`, `vec_clear<T>`, `vec_reverse<T>`, `vec_sort<T>`, `vec_binary_search<T>` | no; `vec_new<T>` needs no `protect`, growing flows do |
| `lib/slice.loi` | `slice_view<T>`, `slice_len<T>`, `slice_is_empty<T>`, `slice_get<T>`, `slice_first<T>`, `slice_last<T>`, `slice_find<T>`, `slice_equals_u8`, `slice_clone<T>`, `slice_copy<T>`, `slice_reverse<T>` | no; allocators need `protect` |
| `lib/string.loi` | `STRING_MAX_LEN`, `str_is_ascii_space`, `str_trim`, `str_concat`, `str_reverse`, `str_repeat`, `str_count`, `str_index_of`, `str_contains` | no; owned results need `protect` |
| `lib/hash.loi` | `HASH_OFFSET`, `HASH_PRIME`, `hash_bytes_seeded`, `hash_bytes`, `hash_str`, `hash_u64_seeded`, `hash_u64`, `hash_pair_u64` | no |
| `lib/crc32.loi` | `CRC32_POLY`, `CRC32_INIT`, `CRC32_XOROUT`, `CRC32_TABLE`, `crc32_update`, `crc32_extend`, `crc32_bytes` | no |
| `lib/map.loi` | `IntMap<V>`, `StrMap<V>`, `map_int_new<V>`, `map_int_len<V>`, `map_int_capacity<V>`, `map_int_contains<V>`, `map_int_get<V>`, `map_int_get_or<V>`, `map_int_lookup<V>`, `map_int_set<V>`, `map_int_remove<V>`, `map_int_clear<V>`, and the matching `map_str_*` flows | no; mutators need `protect` |
| `lib/random.loi` | `Rng`, `rng_new`, `rng_state`, `rng_next_u64`, `rng_next_u32`, `rng_below_u64`, `rng_pick<T>`, `rng_shuffle<T>`; a zero seed uses `RNG_FALLBACK_SEED` | no |
| `lib/os.loi` | `os_pid`, `os_unix_time`, `os_sleep`, `os_has_env`, `os_env_i32`, `os_errno`, `os_exit` | `lib/os_shim.c` |
| `lib/net.loi` | `NetIpv4Result`, `net_ipv4`, `net_ipv4_octet`, `net_port_is_valid`, `net_parse_ipv4`, `net_tcp_connect`, `net_close`, `net_send`, `net_recv` | `lib/net_shim.c` |
| `lib/stdio.loi` | `stdio_read_exact`, `stdio_write_all`, `stdio_flush` | `lib/stdio_shim.c` |
| `lib/json.loi` | `JsonKind`, `JsonDoc`, `json_parse`, `json_root`, `json_kind`, `json_bool`, `json_string`, `json_number_text`, `json_number_i64`, `json_array_len`, `json_array_get`, `json_object_get`, `json_stringify` | no; parsing and stringifying need `protect` |

`lib/math.loi` and `lib/ctype.loi` are pure Loi: the math module uses generic
flows and a `comptime flow` for factorials. Documented library flows and their
shapes are `open` so generated headers can expose them; internal helpers stay
`sealed`. An exported allocator still needs a caller's `protect` region — Loi
callers are checked, and foreign callers follow the same contract.
`lib/option.loi` and `lib/result.loi` provide the official optional and
error-carrying values. `Option<T>` is a shape with `present: bool` and a `value: T`
field; `Result<T, E>` adds `ok: bool` and an `error: E` field. They are shapes,
not `choice` values: choices are not generic and their payloads may not contain
slices. The unselected field is zero-initialized and must be read only through
`option_get_or`, `option_unwrap`, `result_unwrap_or`, `result_unwrap` or
`result_unwrap_err`; the last three abort with a flow-naming message. The
constructors clone their input, so a `protect` region is required when `T` or
`E` contains slices. See `examples/options_results.loi`.
`lib/vec.loi` and `lib/slice.loi` are pure Loi; `Vec<T>` stores cloned values,
so `clone<T>` must be available — including recursive deep clones for `T` that
contain slices. Search flows declare `T: Eq` and ordering flows such as
`vec_sort<T>` declare `T: Clone + Ord`, so both work with whole composite
values; see `examples/ordering.loi`. The arena keeps older growth blocks until
`protect` ends, so
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
value-only generics: `V` must be cloneable (recursive deep clone covers values
that contain slices). `StrMap<V>` stores byte keys in one contiguous arena
buffer, so no slice-of-slices value is formed. Mutations need the caller's
`protect`. `map_int_get` / `map_str_get`
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

They also provide linear handles for `guard`: `File` from `lib/os.loi` has
`file_open_read`, `file_open_write`, `file_read`, `file_write` and `file_release`;
`TcpStream` from `lib/net.loi` has `tcp_stream_connect`, `tcp_stream_send`,
`tcp_stream_recv` and `tcp_stream_release`. Both release flows are `open`, so
the implicit call inserted by `guard` works in the importing module. Handles
own integer descriptors only; no foreign pointer type enters ordinary Loi
values. Read buffers passed with `<->` must be owned by an active `protect`
region. The shims are POSIX-only and are not part of the closed intrinsic set.
A runnable example is `examples/handles.loi`.

### Generated C headers

`loi gen FILE -o FILE.c --emit-h FILE.h` (and the same option on `loi build`)
writes a self-contained C11 header. Its include guard comes from the header
file name, so two headers can be included in one translation unit. The header
defines `LoiSlice`, visible marks, choices and shapes in dependency order, and
prototypes for visible flows; every declaration is guarded and the prototype
block is wrapped in `extern "C"` for C++ callers. The matching flow definitions
in the generated `.c` have external linkage; sealed flows stay internal and do
not appear in headers. A generated `.c` can be linked with C code that includes
one or more generated headers without duplicate type definitions.

### LLVM IR (opt-in)

The C11 backend remains the default and the production path. A scalar and
slice subset can additionally be emitted as real LLVM textual IR:

```sh
loi gen FILE --emit-llvm-ir FILE.ll
loi build FILE --backend llvm -o program
```

`--backend` is build-only and accepts `c` (the default) or `llvm`. The LLVM
path supports integer, floating-point and boolean scalars, flow calls,
`if` / `while` / `for ... in ...`, compound assignment, short-circuit `and` /
`or`, `print_int`, `print_float`, and the `sqrt`, `floor`, `ceil`, `fabs` and
`pow` intrinsics. Slices use the C ABI layout `{ ptr, i64 }` (`void* data`,
`size_t len`) and support slice literals, `<- [T]` parameters, `len`, `xs.len`,
checked indexing and index assignment, `alloc(-> xs, T, n)` inside `protect`,
byte-slice `print` / `eprint` / `panic`, `view`, and `==` / `!=` via byte-wise
comparison. `protect` saves and restores the bump-arena top exactly as in C11.
It emits opaque-pointer IR with the same C symbols as the C11 backend and links
it with `clang`; a different compiler can be selected with `CC`.

Everything outside that subset is rejected with an `LLVM backend does not
support ...` diagnostic instead of being lowered incorrectly: `guard`, shapes,
marks, choices, opaque handles, globals, callback values, `cstr`, `comptime`,
`foreach`, indirect calls, field access other than `len`, and slices whose
element is itself a slice or a user-defined composite. `build --backend llvm`
is limited to a single source file and an executable; it rejects
`--emit obj|lib`, `--c-source`, `--emit-c`, `--emit-h`, `--emit-ir`,
`--emit-llvm-ir`, `--arena-size` and `--source-map` with exit 2.
`examples/llvm_scalars.loi` and `examples/llvm_slices.loi` are complete programs
that build with both backends and print the same output.

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
  `callback` as an input-only parameter, borrowed and mutable slices, and
  shapes whose fields are themselves FFI-safe. Variadics and unions require a
  vendored C shim. Loi flows cannot declare variadic parameters; `printf` is
  the sole variadic intrinsic (section 14).
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
- A `const T *` parameter becomes a borrowed `[T]` slice. The generated shim
  passes the slice's `data` pointer; any C length parameter stays explicit, so
  the call is `sample_checksum(bytes, bytes.len)`. `const char *` remains `cstr`.
- A mutable `char *` parameter becomes `<-> [u8]`, a writable byte slice. The
  compiler passes `<->` slices as a pointer to the slice struct, so the shim
  takes `LoiSlice *` and passes `buf->data`; the length parameter stays
  explicit. Writes go straight into the caller's storage.
- Complete, pointer-free C records become `shape`s; incomplete or unsafe
  records become `opaque` handles.
- Integer `#define` macros and enum constants become typed globals.
- Calls needing layout conversion are emitted as `extern flow ... as
  loi_bind_<module>_<name>` and implemented in the generated shim with
  `memcpy` plus `_Static_assert` size/alignment checks.
- Unsupported declarations are never guessed: callbacks, variadics, unions,
  bitfields, inline arrays, pointer-to-pointer, wide strings, pointer results
  and pointers to unsafe records are skipped and listed in the skip report.

Generated bindings are conservative by design. Add a small hand-written shim
for the specific unsupported calls a program needs. `vendor/sdl2/` and
`vendor/raylib/` are complete worked examples, including their skip reports and
build instructions.

### 15.6 C callbacks

`callback` is the fixed C ABI `int32_t (*)(int32_t, int32_t)`. Declare a C
function that takes one with `extern flow`, then pass a Loi flow through the
`callback(flow)` intrinsic:

```loi
extern flow reduce_range
    <- start: i32
    <- end: i32
    <- f: callback
    => i32

flow add
    <- acc: i32
    <- value: i32
    => i32
{
    => acc + value
}

flow main
    => i32
{
    sum <- reduce_range(1, 5, callback(add))
    print_int(sum)
    => 0
}
```

Other C function-pointer signatures are written with an explicit callback type.
`=> void` may be omitted when the callback returns nothing:

```loi
extern flow visit
    <- count: i32
    <- f: callback(i32) => void

extern flow sum_mapped
    <- count: i32
    <- f: callback(i32) => i32
    => i32

flow report
    <- value: i32
{
    print_int(value)
}

flow square
    <- value: i32
    => i32
{
    => value * value
}

flow main
    => i32
{
    visit(3, callback(report))
    => sum_mapped(3, callback(square))
}
```

- `callback(flow)` accepts exactly one non-`extern` Loi flow. Its parameters
  must all use `<-`; their types and result become the callback's signature.
  The flow's generated C symbol is passed directly; no thunk or userdata is
  emitted, and `callback` remains a closed intrinsic that cannot be redeclared.
- The inferred signature must match the callback type at the use site. The
  plain `callback` type is exactly `callback(i32, i32) => i32`.
- Callback values are copyable. They can be stored in locals, `shape` fields
  and slices, and passed back to C, but they cannot be called, compared, used
  in arithmetic or dereferenced from Loi.
- A `callback` parameter must use `<-`; C owns the call, so the value is
  input-only. An `extern flow` cannot return `callback`; ordinary Loi flows may
  return callback values.
- The generator still skips C function-pointer declarations; declare the
  `extern flow` by hand. The C backend emits one guarded typedef per signature.
  See `examples/callbacks.loi` and `examples/callback_signatures.loi` with their
  matching `_shim.c` files for complete buildable programs.

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
- `examples/views.loi` — zero-copy slice views and their lexical loans.
- `examples/options_results.loi` — official `Option<T>` and `Result<T, E>` in a settings parser.
- `examples/systems.loi` — bitwise flags, shifts and file/stdin I/O.
- `examples/ordering.loi` — structural equality and `Ord` sorting/search.
- `examples/module_mangling.loi` — two modules exposing the same API names.
- `lib/std.loi` — standard library written in canonical Loi.
