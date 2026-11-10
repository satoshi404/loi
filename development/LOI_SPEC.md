# Loi Language — Specification & Development Protocol

**Version:** 0.3.0-draft  
**Status:** Active Development  
**Last Updated:** 2026-11-10
**Author:** Clayton Pedro (Satoshi)

Items marked **(provisional)** are temporary decisions made to close a gap. They can change. The nine former *Open Design Questions* are resolved in section 5. The distinctive surface (Loi Flow Syntax) is the only surface, adopted in 5.10 and fully cleaned up: legacy forms are no longer accepted, and `fmt` emits the canonical Flow Syntax.

The canonical surface reference is [`SINTAXE.md`](../SINTAXE.md). This file defines the language rules; when a syntax example here and `SINTAXE.md` disagree, `SINTAXE.md` wins for the surface form and both must be corrected in the same change.

---

## 1. Purpose of this Document

This file is the **single source of truth** for the Loi programming language.

Any AI agent (OpenCode or others) working on this project **must**:

1. Read this entire document before making changes.
2. Follow the rules and architecture defined here.
3. After every significant step, update the companion file `development/PROGRESS.md` with:
   - What was done
   - Current state of each component
   - Known bugs / limitations
   - Next recommended steps
   - Important design decisions made

The goal is that any new model can open `development/PROGRESS.md` and immediately understand exactly where the project is, without needing the previous conversation history.

---

## 2. Language Overview

**Name:** Loi  
**File extension:** `.loi` (final; see 5.8)  
**Type:** Compiled systems language  
**Paradigm:** Procedural + value-oriented (no OOP, no methods)

**Key goals:**

- Explicit memory and ownership control, without lifetimes or a borrow checker
- Every function signature states what comes in and what goes out
- Simple, regular syntax that is easy to lex and parse
- Strong metaprogramming (`comptime`) — semantics still open

### Core Design Decisions

| Feature              | Decision                                                                 |
|----------------------|--------------------------------------------------------------------------|
| Composite types      | Only `shape`: a named set of fields, data only, no methods                |
| Visibility           | `open` exports; `sealed` is the default                                  |
| Parameters           | Direction is mandatory: `<-`, `->`, `<->`                                |
| Results              | `->` parameters and/or a single `=> T`                                   |
| Ownership            | One owner per value. No implicit copies                                  |
| Transfer             | `give(x)` — the source variable is dead afterwards                       |
| Duplication          | Explicit `clone(x)`                                                      |
| Aliasing             | Two names for the same address are a compile error (double reference)    |
| Memory scope         | `protect { }` — everything allocated inside lives only there             |
| Arrays               | Slices `[T]` (pointer + length, bounds checked)                          |
| Cleanup              | No `defer`. `protect` covers memory                                      |
| Variables            | `name <- value` immutable, `mut name <- value` mutable                   |
| Globals              | Top-level bindings with constant initializers (provisional)              |
| Strings              | Double-quoted literals with decoded escapes; runtime type `[u8]`         |
| Slice literals       | `[a, b]` (inferred) or `[T: a, b]` (explicit)                           |
| Length               | `len(xs)` and `xs.len`, both `u64`                                       |
| Imports              | `import "path.loi"` textual inclusion, resolved by the driver (provisional) |
| Operators            | `and`, `or`, `not`, `as`, `+=` and friends                               |

---

## 3. Concrete Syntax (Reference Example)

The canonical reference is `examples/particles.loi`, mirrored by the syntax tour in `examples/flow_syntax.loi`. Any implementation must be able to lex, parse and type-check the reference program; `loi fmt` must produce stable canonical output for both files.

The example below is the complete reference program in canonical Flow Syntax. Section 5.10 defines the spelling of every construct.

```loi
// The reference simulation. Braces are scopes, brackets are data,
// arrows are movement.

shape Vec2 [ x: f64, y: f64 ]

shape Particle [
    pos:   Vec2
    vel:   Vec2
    mass:  f64
    alive: bool = true
]

shape World [
    particles: [Particle]
    gravity:   Vec2
    bounds:    Vec2
    tick:      u64 = 0
]

open flow world_new
    <- count: u32
    <- size: Vec2
    -> w: World
{
    mut ps: [Particle]
    alloc(-> ps, Particle, count)          // belongs to the caller's protect
    for i in 0..count {
        ps[i] = Particle[
            pos:  Vec2[ x: i as f64 / count as f64 * size.x, y: size.y / 2 ],
            vel:  Vec2[ x: i as f64 * 0.1 - 5, y: 0 ],
            mass: 1 + (i % 3) as f64,
        ]
    }
    w.particles = give(ps)                 // ps is dead, w owns the array
    w.gravity   = Vec2[ x: 0, y: -9.81 ]
    w.bounds    = clone(size)              // size is borrowed, so clone it
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

sealed flow bounce
    <-> p: Particle
    <- axis: u8
{
    if axis == 0 { p.vel.x = -p.vel.x } else { p.vel.y = -p.vel.y }
}

open flow step
    <-> w: World
    <- dt: f64
{
    w.tick += 1
    for <-> p in w.particles {
        if not p.alive { continue }
        p.vel.x += w.gravity.x * dt
        p.vel.y += w.gravity.y * dt
        p.pos.x += p.vel.x * dt
        p.pos.y += p.vel.y * dt
        if p.pos.x < 0 or p.pos.x > w.bounds.x {
            p.pos.x = clamp(p.pos.x, 0, w.bounds.x)
            bounce(<-> p, 0)
        }
        if p.pos.y < 0 or p.pos.y > w.bounds.y {
            p.pos.y = clamp(p.pos.y, 0, w.bounds.y)
            bounce(<-> p, 1)
        }
    }
}

open flow total_mass
    <- w: World
    => f64
{
    mut m: f64 <- 0
    for <- p in w.particles { m += p.mass }
    => m
}

open flow main
    => i32
{
    mut mass: f64                          // exists before the protect
    protect {
        mut w: World
        world_new(256, Vec2[ x: 800, y: 600 ], -> w)
        for _ in 0..3600 { step(<-> w, 1.0 / 60) }
        mass = total_mass(w)               // scalar result written outside
    }                                      // w and all its memory die here
    => 0
}
```

---

## 4. Detailed Rules

### 4.1 Declarations and literals

- `shape Name [ field: Type, ... ]` declares a data type. Fields may have a default value (`alive: bool = true`). No methods.
- A literal always carries the type name and uses brackets: `Vec2[ x: 1, y: 2 ]`.
- `name <- value` creates an immutable binding; `mut name <- value` creates a mutable one. A declaration without a value (`mut w: World`) is uninitialized and can only be passed as `->`.
- Control flow: `if` / `else`, `while`, `for i in a..b`, `for <- p in xs` (read), `for <-> p in xs` (mutate), `continue`, `break`, and `=> value` for returns.

### 4.2 Parameter directions

Every parameter declares its direction. The direction is mandatory.

| Direction | Meaning                                                              |
|-----------|----------------------------------------------------------------------|
| `<-`      | Read-only access to the caller's value. The checker forbids storing or returning a borrowed part; the C backend passes composites by value (slice headers still point at the caller's storage). |
| `->`      | The function initializes the caller's variable                       |
| `<->`     | Read and write access to the caller's value                          |

- At the call site, `->` and `<->` are marked (`step(<-> w, dt)`, `world_new(..., -> w)`). `<-` is not marked, to keep calls readable.
- A `<-` parameter can be read but never stored or returned. To keep part of it, use `clone(...)`.

### 4.3 Ownership, `give` and `clone`

- Every value has exactly one owner and one live name.
- `give(x)` transfers ownership. After it, `x` is dead and must not be used.

```loi
a <- 7
b <- give(a)         // a is no longer usable
```

- There is no implicit copy of composite values. `clone(x)` makes an explicit duplicate in new memory.
- Two names for the same address is a compile error (**double reference**).
- `<->` is exclusive: while a value is passed as `<->`, no other use of it is allowed in the same call.
- Passing the same value twice as `<-` (`f(a, a)`) is allowed, because both are reads.
- Scalars (`i32`, `f64`, `bool`, ...) are copied implicitly. `clone()` is required only for composite values (confirmed in 5.7).
- `clone` of a type that contains a slice is a compile error (confirmed in 5.7).
- `give(x)` is a compiler intrinsic, not an ordinary function: it kills the variable. The parser must treat it specially.

### 4.4 Results: `->` and `=>`

- A function may declare any number of `->` parameters and/or a single `=> T` result.
- `=> T` is sugar for a hidden `->` parameter. The compiler may lower it that way internally.
- Returning a composite value that holds a slice moves ownership to the receiver, with no copy.
- Returning something that points inside a `<-` parameter is an error. Return `clone(...)` of the part instead. This rule is implemented and confirmed by the Phase 2 review (5.7).

### 4.5 `protect` blocks and allocation

```loi
protect {
    // Any allocation made here lives only here
}
```

- Everything allocated inside a `protect` block lives only inside it. At the end of the block all of it is dead.
- Nothing allocated inside can escape the block.
- Code inside may write into memory that existed **before** the block (as `mass` does in the example). That is how results get out. **(provisional)**
- An `alloc` made inside a called function belongs to the nearest enclosing `protect` of the **caller**, not to the callee. **(provisional)**
- `alloc` outside any `protect` is a compile error, so there is no manual `free`. **(provisional)**
- Static and dynamic checks must prevent escape.
- **Implementation:** `protect` is a bump arena that grows in chunks. `--arena-size BYTES` (default 64 MiB) sets the size of the first chunk, not a hard cap. Later chunks grow geometrically, and an allocation larger than the current chunk gets a dedicated chunk. Chunks are never moved or freed, so slices already handed out stay valid; leaving the block restores the arena offset. The arena is process-global and not thread-safe.

### 4.6 Visibility

- `open` makes a function or type usable from other modules. `sealed` keeps it private to its module.
- The default when neither is written is `sealed` (see 5.3).

### 4.7 Performance annotations (deferred, experimental)

These ideas come from the first draft. They are **not** part of the core, and they are moved to Phase 3 because of feasibility problems:

- **Freshness** (was `f64@3`): counting instructions only makes sense after lowering and register allocation, so it is a backend analysis, not a type rule. A possible form is a block attribute with a warning instead of a type error.
- **Spatial tier** (was `*T@L1`): `prefetch` is only a hint and `madvise` works on pages, so tier is better as an attribute (`#hot(xs)`) than as part of the type.
- **Causal sequencing** (was `~>`): data dependencies already prevent reordering, so the operator is only useful for hidden dependencies. A possible form is an `ordered { }` block.

None of these should be implemented before the core (Phases 0 to 2) is stable.

### 4.8 Literals, globals, imports and equality (provisional)

These rules were added to unblock real programs and are implemented and tested. The decisions that were waiting on section 5 are now settled (5.7 and 5.9); the remaining provisional notes describe implementation limits, not open language questions.

**String literals**

- A string literal is written with double quotes: `"hello"`.
- Supported escapes: `\n`, `\t`, `\r`, `\0`, `\\`, `\"` and `\xNN` for ASCII bytes (`0x00`-`0x7f`).
- The runtime type is `[u8]`. The value is the decoded UTF-8 bytes; its length is the byte count. For C interop the backend stores an extra trailing NUL, but that byte is not part of the slice length.
- A string literal has static storage and may be used to initialize a `[u8]` global, and may be passed to or bound as `cstr`.

**Slice literals**

- `[a, b, c]` infers the element type from the elements. The explicit form is `[T: a, b, c]`; `[T:]` is the empty literal.
- The backing array has automatic storage in the enclosing lexical block. A slice literal must not escape that block: the checker rejects returning it, storing it in a variable declared outside the block, or embedding it in a shape that then escapes. Bind it to a local in the same block. Use `alloc` for data that must outlive the block.

**Length**

- `len(xs)` is a compiler intrinsic and `xs.len` is member syntax; both require `[T]` and have type `u64`.
- `len` is a reserved intrinsic name; user code may not redeclare it (5.9).

**Globals**

- Top-level bindings (`name <- value`, `mut name: T <- value`) are globals. They are initialized once, before any function runs.
- Initializers must be compile-time constants (literals, constant unary/binary expressions, casts, `clone`, shape literals of constants and slice literals of constants).
- The C backend currently supports scalar, `bool` and `[u8]`-from-string globals only; other composite globals are rejected pending a runtime constructor.
- A global name may not collide with a function or another global. Non-`mut` globals are immutable, as for locals.

**Imports**

- `import "path.loi"` loads another source file. A relative path is resolved from the importing file's directory; absolute paths are accepted.
- Imports are textual: the imported items join the importing program's namespace. Each canonical file is visited at most once per compilation unit, so diamond imports do not duplicate declarations.
- Visibility is not yet enforced across files: all declarations in an imported file are visible to the importer, and multi-file builds currently give sealed functions external linkage so cross-file calls link. This is provisional.

**Equality**

- `==` and `!=` are defined for scalar numeric types and `bool`, with the usual literal coercion rules.
- They are also defined for `[u8]` values and compare bytes structurally (equal length and equal contents), not by pointer identity.
- Other shapes and slices cannot be compared with `==`/`!=`; compare their fields explicitly. This avoids emitting invalid C struct comparisons.

**Builtin intrinsics and standard library**

- Intrinsics: `print(<- [u8])`, `eprint(<- [u8])`, `panic(<- [u8])`, `sqrt/floor/ceil/fabs(<- f64) => f64`, `pow/fmod(<- f64, <- f64) => f64` and `len([T]) => u64`.
- `lib/std.loi` is a minimal, explicitly imported standard library (integer/float min/max/abs/clamp and byte-string helpers). It is written in Loi and uses only the features above.

### 4.9 Foreign C interop

Foreign declarations are ordinary Loi declarations written in vendored binding
files; application code imports them.

**Opaque handles**

- `opaque Name` declares a foreign handle. It has no layout, fields or methods.
- A handle is a copyable scalar and the C backend represents it as `void *`.
- A handle may be stored, passed, returned and compared by identity; it cannot
  be dereferenced, indexed or converted to another type.

**C strings**

- `cstr` is the FFI-only type for a NUL-terminated C string and is represented
  as `const char *`.
- A string literal may initialize a `cstr` or be passed to a `cstr` parameter.
  An arbitrary `[u8]` value may not, because it is not guaranteed to be
  NUL-terminated.
- `cstr` is copyable but cannot be indexed, measured, compared or printed in
  Loi. Convert it in a vendored shim when byte access is needed.

**Extern flows**

- `extern flow Name` declares a signature with no body.
- Directions keep their normal meaning: `<-` passes by value, while `->` and
  `<->` pass an address.
- The Loi name is the C symbol by default. `extern flow Name as c_symbol` calls
  a different symbol, typically a generated or hand-written shim.
- Supported types are scalars, `bool`, `opaque` handles, `cstr` and shapes whose
  fields are themselves FFI-safe. Slices, callbacks, variadics and unions
  require a C shim.
- Extern flows are not checked for a body, ownership effects or allocation
  regions. Their contracts are the binding author's responsibility.
- `loi build ... --c-source FILE` compiles and links extra C sources.
  `--cc-flag FLAG` forwards a flag to the C compiler; on link steps the flags
  follow the objects so `-l` libraries resolve normally.

**Binding generation**

- `tools/bindgen/bindgen.py` generates vendored bindings from a C header using
  clang's JSON AST. It emits `opaque` handles, safe `shape`s, typed integer
  constants and `extern flow` declarations, plus an optional C shim and a skip
  report.
- The generator is conservative: callbacks, variadics, unions, bitfields,
  inline arrays, pointer-to-pointer, mutable `char *` buffers, wide strings and
  pointers to records that are not layout-safe are skipped and reported rather
  than guessed.
- Safe records passed or returned by value, shape pointers and scalar out
  parameters go through generated `memcpy` shims with `_Static_assert`
  size/alignment checks. Generated symbol names are `loi_bind_<module>_<name>`.
- `vendor/sdl2/` is the reference generated binding, including its shim,
  skipped report and build command. `vendor/raylib/` is the raylib 5.5 binding.

### 4.10 Checked integer arithmetic

Integer arithmetic is defined by Loi, not delegated to C's undefined behaviour.
The generated C uses checked helpers built on `__builtin_{add,sub,mul}_overflow`;
each failure prints `loi: <message> at <line>:<column>` and exits with status 1.

- **Signed `+`, `-`, `*` and unary `-`** trap on overflow. The result type is
  the checked common width, so wrapping at `int` width never leaks through.
- **Unsigned `+`, `-`, `*`** wrap modulo `2^N` for their declared width. This is
  defined behaviour, not an error.
- **`/` and `%`** trap when the divisor is zero. `MIN / -1` traps as signed
  overflow; `MIN % -1` evaluates to `0`. Integer `/` truncates toward zero and
  `%` takes the sign of the dividend.
- **Compound assignments** (`+=`, `-=`, `*=`, `/=`) follow the same rules as the
  corresponding binary operator. Shift, bitwise and `%=` forms do not exist yet.
- **`as`** remains an unchecked truncating cast; it is the explicit escape hatch
  for modular conversion.
- **Literal-only constant expressions** are folded by the checker and rejected
  at compile time when they overflow the expected type or divide by a constant
  zero. Non-constant failures are runtime traps.
- **`float` arithmetic** follows C/IEEE-754 and is not trapped.

---

## 5. Design Decisions (formerly Open Questions)

All nine questions from the 0.2 draft are decided. Each entry states the rule, the reason, and whether the compiler implements it. "Provisional" is kept only for rules we expect to revisit with new evidence; it no longer means "undecided".

### 5.1 `comptime` — must be known at compile time (question 1)

**Decision:** `comptime` is a guarantee, not a hint. It means "this must be computable without runtime state", matching Zig's reading rather than C++ `constexpr`'s optional folding.

- Constant contexts require it: global initializers, shape field defaults, slice/array extents, and explicit `comptime` expressions.
- `comptime expr` is an error unless `expr` consists only of literals, constant globals, `comptime` functions and constant-foldable operators/casts. Allocation, `protect`, I/O and `panic` are forbidden.
- `comptime flow f ...` declares a function that may be called in a constant context. Its parameters must be `<-`; it may not use `->`/`<->`, `alloc`, `protect`, I/O or mutate globals. It may also be called at runtime; the annotation only guarantees that the compiler can evaluate it.
- Evaluation is deterministic, has no side effects, and is bounded: a step limit produces a compile error instead of a compiler hang.
- `comptime` does not create a new type or a new storage class.

**Why:** the first draft was ambiguous because a `comptime` function could receive runtime values. A guarantee gives the compiler something it can rely on, keeps constant contexts honest, and leaves ordinary constant folding free to happen anywhere as an optimization.

**Status:** not implemented. `comptime` is lexed but not parsed; it is Phase 3 work and needs the constant evaluator first.

### 5.2 Pointers — none (question 2)

**Decision:** Loi has no general-purpose pointers, no `&expr`, no `*T` and no `nil`.

- The core invariant is one owner and one live name; an address-of operator creates a second name for the same address and would either violate that invariant or require a lifetime system the language does not have.
- Read-only access is already expressed by `<-`, by a slice `[T]`, or by reading a local. These are checked, scoped views, not storable addresses.
- Cyclic and shared structures use handles/indices (5.5).
- Absent values are explicit: a `present`/`valid` flag inside a shape, or a documented sentinel index such as `NO_NODE`. There is no null.
- Foreign pointers are represented only at the FFI boundary, as `opaque` handles (`void *`) or `cstr` (`const char *`); they never enter ordinary Loi values and cannot be dereferenced.

**Why:** adding pointers would give Loi two competing memory models. The features that need them (graphs, optional values, foreign calls) have simpler representations that preserve single ownership.

**Status:** general pointers remain absent. The FFI surface (`opaque`, `cstr`, `extern flow`, generated shims) is implemented; see 4.9.

### 5.3 Default visibility — `sealed` (question 3)

**Decision:** a declaration with neither visibility keyword is `sealed`. `open` is the only way to export.

- Applies to functions, shapes and globals.
- `sealed` may be written for emphasis, but is redundant.
- `main` is exported implicitly by the C backend; the language treats it as the entry point.
- Imports do not yet enforce visibility across files (textual inclusion). That is an implementation gap, not a language rule.

**Why:** least privilege, and it matches the compiler's existing `Visibility::Hidden` default. It also removes the inconsistency between the spec and the AST.

**Status:** implemented in AST, parser, checker and codegen; cross-file enforcement is pending.

### 5.4 Long-lived data — lexical regions, no named region (question 4)

**Decision:** the core has no named region syntax. Lifetime is lexical: allocate in an enclosing `protect` outside the loop and write results into it.

```loi
protect {
    mut frame: Frame
    frame_new(-> frame)           // allocated once, outside the loop
    for tick in 0..3600 {
        frame_reset(<-> frame)    // fill in place, no per-iteration allocation
        step(<-> frame, 1.0 / 60)
    }
}
```

- A value that must outlive one `protect` block belongs to an outer `protect` (or to a caller-owned variable).
- Reusing storage across frames is a library concept: one long-lived owner (a shape holding a `[u8]` arena and an offset) allocated once and reset explicitly. It needs no new keyword.
- Named regions (`protect name { ... }`, `alloc(name, T, n)`) remain a possible extension, but are not planned: they add a second lifetime system for a case nesting already covers.

**Status:** supported by the current `protect`/`alloc` rules; no new syntax.

### 5.5 Cyclic structures — handles, not pointers (question 5)

**Decision:** cycles are represented with integer handles into an owning slice; the slice, not the links, owns the nodes.

```loi
shape Node [
    parent:       u32
    first_child:  u32
    next_sibling: u32
]

no_node: u32 <- 4294967295
```

- Indices are `u32`/`u64`; a sentinel or a `valid` flag marks "none".
- The node array lives in the same `protect` region as the graph, so destruction is region destruction; there is no reference counting and no weak/strong distinction.
- Reuse-after-free is detected with a generation counter: a handle is `shape Handle [ index: u32, generation: u32 ]` and stale handles are rejected by the library that owns the array.
- Parent pointers, doubly linked lists and general graphs all become index fields.

**Why:** handles preserve single ownership, are trivially copyable, and make the owner explicit. They also produce stable, cache-friendly layouts.

**Status:** no compiler feature is required (slices, integers and `comptime` constants suffice). Standard-library graph/arena helpers are future work.

### 5.6 Non-memory resources — linear close, `guard` later (question 6)

**Decision:** resources are owned linearly and closed explicitly. There is no `defer` and no hidden destructor.

- A resource is a shape with a compiler-known release function, written by convention as `<Shape>_release(<-> value: Shape)`.
- The checker must prove that every path releases exactly once before the value dies, and reject use after release, double release, and leaks (returning or overwriting an unreleased resource).
- Scoped sugar, once resource declarations exist:

```loi
guard file <- file_open("data.bin") {
    // use file
}   // compiler inserts file_release(<-> file) on every exit path
```

- `protect` remains memory-only. A resource that owns memory (a file buffer) simply contains a slice owned by the same region.

**Why:** explicit linear ownership fits the existing move rules and needs no garbage collector, no unwinding and no methods. `guard` is sugar over the same rule, not a second mechanism.

**Status:** not implemented; there is no file/socket API yet. `print`/`eprint`/`panic` are the only I/O until this lands.

### 5.7 Scalar and composite copy — confirmed (question 7)

**Decision:** the Phase 2 rules are confirmed.

- Scalars (`i32`, `f64`, `bool`, ...) copy implicitly: they have no identity, no address and no destructor.
- `clone(x)` is a deep value duplication for slice-free composites.
- `clone` of any value that contains a slice (directly or through a shape) is rejected: a copy would either alias the slice (forbidden) or allocate memory without a region/length policy.
- Slices and slice-containing composites move with `give`/ownership transfer, or are duplicated later by an explicit allocating operation such as `dup` in the standard library.

**Why:** this is the smallest rule that keeps single ownership sound today. An allocating copy needs an allocator and a policy for nested slices, which belongs to the library/region design.

**Status:** implemented and covered by the Phase 2 tests.

### 5.8 File extension — `.loi` (question 8)

**Decision:** source files use `.loi`. `.lum` is abandoned and must not appear in new material.

- The canonical reference program is `examples/particles.loi` (plural, matching the existing file).
- `import` paths and CLI examples use `.loi`.

**Why:** the implementation, tests and examples already use `.loi`; keeping two extensions would add no value.

**Status:** done (documentation aligned).

### 5.9 Standard functions — intrinsics plus an explicit library (question 9)

**Decision:** two tiers, closed.

1. **Intrinsics** are compiler-known, reserved names with fixed signatures and a direct backend mapping:
   - I/O: `print`, `eprint`, `panic`
   - math: `sqrt`, `floor`, `ceil`, `fabs`, `pow`, `fmod`
   - `len`

   They cannot be redeclared by user code and are always available (no import).
2. **The standard library** is ordinary Loi in `lib/std.loi`, explicitly imported. It has no implicit prelude. Names are prefixed (`min_i32`, `str_eq`, ...) until a module/namespace design exists.

New functionality goes into the library by default; an intrinsic is added only when the operation must be implemented by the backend or the OS.

**Why:** a tiny closed intrinsic set keeps the compiler small and portable; a Loi-written library exercises the language and can grow without changing the compiler.

**Status:** implemented. `len` is now reserved in the checker like the other intrinsics, and the old "user-defined `len` wins" exception is gone.

### 5.10 Syntax identity — Loi Flow Syntax (adopted, complete)

The semantics are Loi's. The early surface borrowed Rust vocabulary (`fun`, `interface`, `let`, `[]T`, `->`, `move`/`copy`). **Loi Flow Syntax** is now the only surface. The migration is complete:

- the lexer and parser accept only Flow Syntax;
- `loi fmt` emits Flow Syntax;
- `examples/particles.loi`, `examples/flow_syntax.loi`, `lib/std.loi` and the test sources all use canonical Flow Syntax;
- the legacy keywords and forms have been removed, not merely deprecated.

**Principle:** *braces are scopes, brackets are data, arrows are movement.*

The table below records the historical migration mapping. The left column is no longer valid Loi.

| Historical (removed) | Canonical Flow Syntax | Why |
|---|---|---|
| `interface Vec2 { ... }` | `shape Vec2 [ ... ]` | data-only types; brackets mark data |
| `fun f(...)` | `flow f ...` | functions are data flow through directions |
| `in x: T` | `<- x: T` | direction glyph, not a keyword |
| `out x: T` | `-> x: T` | same |
| `inout x: T` | `<-> x: T` | same |
| `-> T` (result) | `=> T` | result is separate from an `out` parameter |
| `return v` | `=> v` | a return is the result flowing out |
| `let x = v` | `x <- v` | binding is a flow into a new name |
| `var x = v` | `mut x <- v` | mutability is a modifier, not a keyword pair |
| `move(x)` | `give(x)` | explicit ownership transfer |
| `copy(x)` | `clone(x)` | explicit duplication |
| `visible` / `hidden` | `open` / `sealed` | module boundary; default `sealed` |
| `[]T` | `[T]` | bracket = data |
| `Vec2{ x: 1 }` | `Vec2[ x: 1 ]` | brace = scope; bracket = value |
| `for in p in xs` | `for <- p in xs` | same direction glyphs |
| `for inout p in xs` | `for <-> p in xs` | same |
| call `f(out w)` | `f(-> w)` | same |
| `alloc(out xs, T, n)` | `alloc(-> xs, T, n)` | allocation writes through an out flow |
| `protect` | `protect` | already distinctive; keep it |

**Implemented details**

- **Shapes** use brackets: `shape Vec2 [ x: f64, y: f64 ]`; the closing `]` may be on its own line. `shape Name []` is an empty shape.
- **Shape literals** use brackets: `Vec2[ x: 1, y: 2 ]` and `Vec2[]`. Bracket literals are unambiguous in `if`/`while`/`for` headers, so the formatter no longer inserts parentheses there.
- **Functions** may be written inline (`flow f(<- x: i32) => i32 { ... }`) or vertically:

  ```loi
  open flow world_new
      <- count: u32
      <- size: Vec2
      -> w: World
  {
      ...
  }
  ```

  Direction clauses use the glyphs exclusively. The result clause is `=> T`. A function with no parameters and no result may use `flow f {`.
- **Bindings** are `x <- v`, `mut x: T <- v`, `x: T` and `mut x: T`. `mut` is required for assignment after initialization.
- **Globals** use the same binding form (`name <- value`, `open mut name: T <- value`); `open` exports and `sealed` is the default.
- **Slice types** are `[T]`. **Slice literals** are `[a, b]` (inferred), `[T: a, b]` (explicit) or `[T:]` (empty). The brace form `[]T{ a, b }` is not accepted.
- **Calls and loops** use the direction glyphs: `step(<-> w, dt)`, `for <- p in xs`, `for <-> p in xs`.
- **Ownership** is `give(...)` and `clone(...)`; their AST and semantics are exactly the old `move`/`copy`.
- **Returns** are `=> value` or `=>`. The `return` keyword is not accepted.

Canonical reference:

```loi
shape Vec2 [ x: f64, y: f64 ]

shape Particle [
    pos:   Vec2
    vel:   Vec2
    mass:  f64
    alive: bool = true
]

shape World [
    particles: [Particle]
    gravity:   Vec2
    bounds:    Vec2
    tick:      u64 = 0
]

open flow world_new
    <- count: u32
    <- size: Vec2
    -> w: World
{
    mut ps: [Particle]
    alloc(-> ps, Particle, count)
    for i in 0..count {
        ps[i] = Particle[
            pos: Vec2[ x: i as f64 / count as f64 * size.x, y: size.y / 2 ],
            vel: Vec2[ x: i as f64 * 0.1 - 5, y: 0 ],
            mass: 1 + (i % 3) as f64,
        ]
    }
    w.particles = give(ps)
    w.gravity = Vec2[ x: 0, y: -9.81 ]
    w.bounds = clone(size)
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

`examples/flow_syntax.loi` is the complete reference. Reformatting `examples/particles.loi` with `loi fmt` yields byte-identical canonical output.

**Migration status**

1. **Additive — done:** the lexical and grammatical forms of Flow Syntax were implemented alongside the old surface.
2. **Transitive — done:** `lib/std.loi`, every example and every parseable Loi source string in the tests were ported to canonical Flow Syntax.
3. **Cleanup — done:** the legacy keywords (`interface`, `fun`, `visible`, `hidden`, `let`, `var`, `out`, `inout`, `move`, `copy`, `return`), the `&` token, brace shape literals and `[]T{ ... }` slice literals were removed. `in` remains only as the range/`for` separator. Sections 3, 4 and 7 are canonical-only.

Alternative identities considered:

- **Minimal rename only** (`fun` → `proc`, `interface` → `shape`): lower risk, but stays structurally Rust-like.
- **Direction-after-name** (`count: u32 in`): readable and Ada-like, but does not solve the bracket/brace/literal resemblance.
- **Arrow bindings only** (`x <- 7`): strong local identity, but signatures would still look like Rust.

The full Flow Syntax was chosen because it makes the three distinctive ideas — directions, ownership transfer and lexical regions — visible in every declaration.

---

## 6. Development Roadmap

### Phase 0 — Foundation (done)

- [x] Project structure
- [x] Lexer
- [x] Parser (produce AST)
- [x] Pretty-printer / AST dump

### Phase 1 — Core Type System (done)

- [x] Basic types (`i32`, `u32`, `u64`, `f64`, `bool`, etc.)
- [x] `shape` declarations, defaults and literals
- [x] Flows + `<-` / `->` / `<->` / `=> T`
- [x] `open` / `sealed`
- [x] Simple type checking

### Phase 2 — Ownership & Memory (done)

- [x] Single owner and alias checks (double reference)
- [x] `give()` and `clone()`
- [x] Slices `[T]` with bounds checking
- [x] `alloc` and `protect` blocks
- [x] Lexical-block escape analysis for slice literals

### Phase 3 — Advanced Features

- [ ] `comptime` (semantics decided in 5.1; implementation pending)
- [ ] Freshness, spatial hints and ordering blocks (see 4.7)

### Phase 4 — Backend

- [x] Code generation to C11 (first target), including a growable `protect` bump arena, checked arithmetic, runtime bounds checks, headers, source maps and multi-file builds
- [x] Foreign C interop (`opaque`, `cstr`, `extern flow`, shims) and the binding generator
- [ ] Later: x86-64 or LLVM. `src/ir.rs` and `src/asm.rs` are drafts; `--emit-ir` emits IR text, while `--emit-asm` is disabled with a clear CLI error until the assembly backend can generate runnable code.

---

## 7. Lexical Notes

- **Keywords:** `shape`, `flow`, `open`, `sealed`, `extern`, `opaque`, `mut`, `give`, `clone`, `protect`, `alloc`, `import`, `comptime`, `if`, `else`, `while`, `for`, `in`, `continue`, `break`, `and`, `or`, `not`, `as`, `true`, `false`.
- **Direction glyphs:** `<-` (read), `->` (write/out), `<->` (read-write), `=>` (result).
- **FFI types:** `opaque Name`, `cstr`.
- `in` is a keyword only as the separator in `for i in a..b` and `for <- p in xs`; it is not a direction.
- String literals are double-quoted; see 4.8 for escapes and type.
- Numeric literals: decimal integers and floats, `_` separators, exponents, and `0x`/`0b` integer prefixes.
- Multi-character tokens use **longest match first**: `<->`, `<-`, `=>`, `..`, `->`, `+=`, `-=`, `*=`, `/=`, `==`, `!=`, `<=`, `>=`.
- Comments start with `//` and run to the end of the line.
- Fields in a `shape` body and arguments in a call are separated by commas or newlines.
- A top-level `:` inside `[...]` marks a shape literal (`Vec2[ x: i32 ]`) or a typed slice literal (`[i32: 1, 2]`); without one, `[...]` is an index or an inferred slice literal.

---

## 8. Implementation Rules for the AI Agent

1. Always update `development/PROGRESS.md` after finishing a meaningful task.
2. Prefer small, correct steps over large incomplete ones.
3. When you encounter an ambiguity in the language design, document it in `development/PROGRESS.md` under "Open Design Questions" and continue with a reasonable temporary decision.
4. Keep the code clean and modular (separate lexer, parser, AST, types, backend, etc.).
5. Write tests as soon as a feature is implemented.
6. Never delete or rewrite large parts of working code without recording why in `development/DECISIONS.md`.
7. Keep the canonical surface in `SINTAXE.md` and the public documentation in `website/docs.html` in sync with the compiler.

---

## 9. Required Companion File

You must maintain `development/PROGRESS.md` with the following structure:

````markdown
# Loi Compiler — Implementation Progress

## Current Status
- Phase: ...
- Last completed task: ...
- Overall health: (Good / Needs attention / Blocked)

## Component Status
| Component       | Status          | Notes |
|-----------------|-----------------|-------|
| Lexer           | ...             |       |
| Parser          | ...             |       |
| AST             | ...             |       |
| Type Checker    | ...             |       |
| ...             | ...             |       |

## Verification
- Command: ...
- Result: ...

## Known Bugs / Limitations
- ...

## Open Design Questions
- ...

## Next Steps (recommended)
1. ...
2. ...
````

This file is the memory of the project. Treat it seriously.

---

## 10. Orientation for a New Agent

1. Read `development/AGENTS.md`, this specification, `SINTAXE.md`,
   `development/PROGRESS.md` and `development/DECISIONS.md`.
2. Build and run the test suite with the host toolchain:
   `export PATH=/root/.cargo/bin:$PATH`,
   `export RUSTFLAGS="-C linker=/usr/bin/gcc"`,
   `cargo test --all-targets`.
3. Reproduce the current state with `loi check examples/flow_syntax.loi` and,
   if `cc` is available, `loi build examples/flow_syntax.loi -o /tmp/flow_demo`.
4. Pick the next item from `development/PROGRESS.md` → "Next Steps", write the
   test with the feature, and update the progress and decision logs when done.
