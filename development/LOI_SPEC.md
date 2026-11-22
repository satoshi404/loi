# Loi Language — Specification & Development Protocol

**Version:** 0.3.0-draft
**Status:** Active Development
**Last Updated:** 2026-11-22
**Author:** Clayton Pedro (Satoshi)

Items marked **(provisional)** are temporary decisions made to close a gap. They can change. The nine former *Open Design Questions* are resolved in section 5. The distinctive surface (Loi Flow Syntax) is the only surface, adopted in 5.10 and fully cleaned up: legacy forms are no longer accepted, and `fmt` emits the canonical Flow Syntax.

The canonical surface reference is [`SINTAXE.md`](../SINTAXE.md). This file defines the language rules; when a syntax example here and `SINTAXE.md` disagree, `SINTAXE.md` wins for the surface form and both must be corrected in the same change.

The implementation requires Rust 1.88 or newer (the verified MSRV declared in
`Cargo.toml`) and a C11 compiler for the production backend.

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
- Strong metaprogramming (`comptime`) — pure, step-limited compile-time evaluation plus code-as-value templates (`quote`/`splice`)

### Core Design Decisions

| Feature              | Decision                                                                 |
|----------------------|--------------------------------------------------------------------------|
| Composite types      | `shape` for products; `mark` and `choice` for sums; data only, no methods |
| Visibility           | `open` exports; `sealed` is the default                                  |
| Parameters           | Direction is mandatory: `<-`, `->`, `<->`                                |
| Results              | `->` parameters and/or a single `=> T`                                   |
| Ownership            | One owner per value. No implicit copies                                  |
| Transfer             | `give(x)` — the source variable is dead afterwards                       |
| Duplication          | Explicit `clone(x)`                                                      |
| Aliasing             | Two names for the same address are a compile error (double reference)    |
| Memory scope         | `protect { }` — everything allocated inside lives only there             |
| Arrays               | Slices `[T]` (pointer + length, bounds checked)                          |
| Generics             | `shape Vec<T>` / `flow f<T>`; explicit arguments, monomorphized, optional `T: Clone + Copy + Eq + Ord` bounds (§5.1a) |
| Compile-time values  | `comptime flow` (pure), `comptime expr` and `Code` templates; globals require constants |
| Cleanup              | No `defer`. `protect` covers memory; `guard` covers linear resources (§5.6) |
| Variables            | `name <- value` immutable, `mut name <- value` mutable                   |
| Globals              | Top-level bindings with constant initializers of any shape               |
| Strings              | Double-quoted literals with decoded escapes; runtime type `[u8]`         |
| Slice literals       | `[a, b]` (inferred) or `[T: a, b]` (explicit)                           |
| Length               | `len(xs)` and `xs.len`, both `u64`                                       |
| Imports              | `import "path.loi"` loads a module (file); `open import "path.loi"` re-exports its `open` items; `as` aliases it, `module::item` qualifies references (§4.8) |
| Operators            | `and`, `or`, `not`, `is`, `as`, bitwise `&` `\|` `^` `~`, shifts `<<` `>>`, compound `+=` and friends |

---

## 3. Concrete Syntax (Reference Example)

The canonical reference is `examples/particles.loi`, mirrored by the syntax tour in `examples/flow_syntax.loi`. Any implementation must be able to lex, parse and type-check the reference program; `loi fmt` must produce stable canonical output for both files.

The example below is the complete reference program in canonical Flow Syntax. Section 5.10 defines the spelling of every construct.

```loi
// The reference simulation. Braces are scopes, brackets are data,
// arrows are movement.

shape Vec2 { x: f64, y: f64 }

shape Particle {
    pos:   Vec2
    vel:   Vec2
    mass:  f64
    alive: bool = true
}

shape World {
    particles: [Particle]
    gravity:   Vec2
    bounds:    Vec2
    tick:      u64 = 0
}

open flow world_new
    <- count: u32
    <- size: Vec2
    -> w: World
{
    mut ps: [Particle]
    alloc(-> ps, Particle, count)          // belongs to the caller's protect
    for i in 0..count {
        ps[i] = Particle {
            pos:  Vec2[ x: i as f64 / count as f64 * size.x, y: size.y / 2 ],
            vel:  Vec2[ x: i as f64 * 0.1 - 5, y: 0 ],
            mass: 1 + (i % 3) as f64,
        }
    }
    w.particles = give(ps)                 // ps is dead, w owns the array
    w.gravity   = Vec2 { x: 0, y: -9.81 }
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
        world_new(256, Vec2 { x: 800, y: 600 }, -> w)
        for _ in 0..3600 { step(<-> w, 1.0 / 60) }
        mass = total_mass(w)               // scalar result written outside
    }                                      // w and all its memory die here
    => 0
}
```

---

## 4. Detailed Rules

### 4.1 Declarations and literals

- `shape Name { field: Type, ... }` declares a data type. Fields may have a default value (`alive: bool = true`). No methods.
- A literal always carries the type name and uses braces: `Vec2 { x: 1, y: 2 }`.
- `name <- value` creates an immutable binding; `mut name <- value` creates a mutable one. A declaration without a value (`mut w: World`) is uninitialized and can only be passed as `->`.
- Control flow: `if` / `else`, `while`, `for i in a..b`, `for <- p in xs` (read), `for <-> p in xs` (mutate), `continue`, `break`, and `=> value` for returns.

### 4.1a Sum types: `mark` and `choice`

- `mark Name { A, B, ... }` declares a payload-free finite tag set. A value is
  written `Name.A`. Marks are scalar-like: they copy implicitly and `==` / `!=`
  are defined only between two values of the same mark. Ordering and arithmetic
  are not defined.
- `choice Name { A: T, B: U, ... }` declares a tagged union with one payload per
  variant. `Name.A(value)` constructs a value; the constructor requires exactly
  one `<-` argument whose type matches the declared payload. Bare
  `Name.A` is only valid as the right operand of `is`, never as a value.
- `value is Name.A` tests the active variant and yields `bool`. The left operand
  must have that choice type and the variant must belong to it. `is` does not
  change the static type of `value`.
- `value.A` reads the active payload. The checker verifies that `A` is a variant
  of the value's choice, but the tag is checked at runtime because there is no
  static narrowing. A wrong read aborts with
  `loi: choice 'Name' does not hold 'A' at <line>:<column>`, using the read's
  source position. Payload reads are not assignable: construct a new choice
  value instead.
- Payloads may be numeric scalars, `bool`, marks or slice-free shapes. Slices,
  shapes containing slices, nested choices, `cstr` and `opaque` handles are
  rejected so the C union remains a fixed-size, ownership-free value. Choices
  are not generic.
- Marks and choices use `open` / `sealed` visibility like shapes and share the
  type namespace with shapes, opaque handles, flows and globals. Variants are
  scoped to their declaration; duplicate variant names are errors.

### 4.2 Parameter directions

Every parameter declares its direction. The direction is mandatory.

| Direction | Meaning                                                              |
|-----------|----------------------------------------------------------------------|
| `<-`      | Read-only access to the caller's value. The checker forbids storing a borrowed part; a borrowed slice view may be returned (§4.4a). The C backend passes composites by value (slice headers still point at the caller's storage). |
| `->`      | The function initializes the caller's variable                       |
| `<->`     | Read and write access to the caller's value                          |

- At the call site, `->` and `<->` are marked (`step(<-> w, dt)`, `world_new(..., -> w)`). `<-` is not marked, to keep calls readable.
- A `<-` parameter can be read but never stored. Its whole value cannot be returned; to keep an owned copy use `clone(...)`. A slice view of its storage may be returned under §4.4a. The read-only view is lexical: it lasts for the call, and its precise extent is the read-view summary described in 4.3.

### 4.3 Ownership, `give` and `clone`

- Every value has exactly one owner and one live name.
- `give(x)` transfers ownership. After it, `x` is dead and must not be used. A global, or a field/element of a global, cannot be given (there is no enclosing function that owns it); use `clone(...)` instead.

```loi
a <- 7
b <- give(a)         // a is no longer usable
```

- There is no implicit copy of composite values. `clone(x)` makes an explicit duplicate in new memory. For values that contain slices the duplicate is deep: each slice gets fresh arena storage, nested shapes are cloned recursively, and the source stays live and unchanged. Because it allocates, such a clone requires an active `protect` region (or a flow whose caller supplies one), and its result may not escape that region. Slice-free composites remain a bit-wise copy.
- Two names for the same address is a compile error (**double reference**).
- `<->` is exclusive: while a value is passed as `<->`, no other use of it is allowed in the same call.
- Passing the same value twice as `<-` (`f(a, a)`) is allowed, because both are reads.
- Scalars (`i32`, `f64`, `bool`, ...) are copied implicitly. `clone()` is required only for composite values (confirmed in 5.7).
- Partial moves are path-sensitive: giving a composite field or element kills only that place, leaves siblings alive, and can be reversed by writing the place again. Giving a scalar sub-place copies it without killing the owner; giving a whole scalar variable remains an explicit transfer.
- A consumed path containing a runtime index is a wildcard over every element of that slice. Reads of any element are rejected until the whole slice is reinitialized.
- Function-effect summaries carry consumed paths, so a callee that gives `p.left` kills only `p.left` at the call site.
- Read-view summaries complement consumption: for each `<-` parameter, a flow records the root-relative paths it reads. A whole-parameter `<-` argument passed on to another flow inherits the callee's read paths (joined with the argument path) through a fixpoint; an unknown or external callee is assumed to read the whole value.
- A composite with consumed paths may be passed to a `<-` parameter when every effective read path avoids the consumed paths. A read that overlaps a consumed path is reported as a use of the moved value. This keeps a whole-place read sound while allowing a caller to hand a partially consumed value to a flow that only touches live fields (Block A3).
- A deep `clone` of a slice-containing type allocates in the current `protect` region; out-of-region clones and escaping results are compile errors (see 5.7, updated by Block A2).
- `give(x)` is a compiler intrinsic, not an ordinary function: it kills the variable. The parser must treat it specially.

### 4.4 Results: `->` and `=>`

- A function may declare any number of `->` parameters and/or a single `=> T` result.
- `=> T` is sugar for a hidden `->` parameter. The compiler may lower it that way internally.
- Returning a composite value that holds a slice moves ownership to the receiver, with no copy.
- Returning a non-slice value that points inside a `<-` parameter is an error. Return `clone(...)` of the part instead.
- Returning a slice-typed view of a `<-` parameter is allowed under §4.4a. The result is a view whose lifetime is the caller's storage region.

### 4.4a Borrowed slice views (Block A3)

`view(xs, start, end)` is a compiler intrinsic returning a zero-copy subslice.
`xs` must be `[T]`, `start` and `end` must be integers, and all three arguments
use `<-`. At runtime `start <= end <= xs.len` is checked; an invalid range
traps. The result is `{ data: xs.data + start * sizeof(T), len: end - start }`.

A returned slice is a view over its storage, not an owned composite. Loi tracks
the storage borrowed by every live view with **lexical loans**:

- Creating a view adds a loan on the source place. The loan is attached to the
  value and is carried through `let`, assignment, member/index access and
  calls. `clone` stops the chain because it allocates fresh storage.
- Each flow records which `<-` parameters may escape as views through the `=>`
  result or a `->` parameter whose type contains a slice. The summary is
  computed to a fixpoint, so a view may be forwarded through any number of
  flows, including inside a returned composite; at each call site the loan is
  rebound to the corresponding argument and destination place.
- A loan records the source place and the holder-relative slot that stores the
  view, keeping them distinct. Overwriting the slot releases that loan without
  disturbing loans in sibling slots, and reading a sub-place relocates the slot
  with the value.
- While a loan is live, writing, moving or mutably borrowing an overlapping
  path of the source is a compile error. The diagnostic labels the place where
  the view was created.
- A loan lives exactly as long as the variable holding the view. Its scope end
  releases it. Loans merge as a union across branches and are restored with
  the rest of the checker's control state.
- The ordinary region rules still apply. A view of a block-local slice literal
  cannot be stored in a variable that outlives the block, and a view of a
  `protect`-allocated slice cannot escape that `protect`.
- `lib/slice.loi`'s `slice_view<T>` and `lib/vec.loi`'s `vec_view<T>` expose the
  same operation as ordinary library flows. `vec_as_slice<T>` keeps its
  historical detached-copy behaviour.

**Why:** early Loi rejected every borrowed return because a view could outlive
the storage it pointed at. A closed intrinsic plus summary-driven loans gives
zero-copy subslice access without introducing named lifetimes, runtime handles
or a new ABI; the existing region taints already describe storage lifetime.

**Status:** implemented in the checker and the C11 backend (advisory helper
`loi_view`), with positive, negative and runtime tests.

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
- **Implementation:** the unguarded-allocation check treats only `main` as an entry point. An uncalled helper may rely on its caller's `protect` regardless of visibility, so exported library allocators can be `open`; Loi call sites are still checked through the call graph. A foreign caller must supply the `protect` region by contract.

### 4.6 Visibility

- `open` makes a function or type usable from other modules. `sealed` keeps it private to its module (§4.8).
- The default when neither is written is `sealed` (see 5.3). Visibility is now
  enforced by the driver's module resolver, not only by header emission.

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
- A **non-empty** literal's backing array has automatic storage in the enclosing lexical block. It must not escape that block: the checker rejects returning it, storing it in a variable declared outside the block, or embedding it in a shape that then escapes. Bind it to a local in the same block. Use `alloc` for data that must outlive the block.
- The **empty** literal `[T:]` allocates no storage: it is the null slice with length zero. It has no region taint, so it may be returned, assigned into a `->`/`<->` parameter, or stored in a shape that outlives the creating block. This lets a collection constructor produce an empty value without an `alloc` or a surrounding `protect`; the first growth still needs `alloc` inside `protect`. Runtime indexing checks length before dereferencing, so indexing the null slice aborts rather than reading invalid memory (Block A4).

**Length**

- `len(xs)` is a compiler intrinsic and `xs.len` is member syntax; both require `[T]` and have type `u64`.
- `xs.len` is a writable place when `xs` is mutable (including `->`/`<->` parameters). Writing it changes the length used by runtime bounds checks; the checker does not track capacity, so setting a length larger than the allocated storage is the programmer's responsibility. A `<-` parameter remains read-only (Block A4).
- `len` is a reserved intrinsic name; user code may not redeclare it (5.9).

**Globals**

- Top-level bindings (`name <- value`, `mut name: T <- value`) are globals. They are initialized once, before any function runs.
- Initializers must be compile-time constants (literals, constant unary/binary expressions, casts, `clone`, shape literals of constants, slice literals of constants, mark variants and choice constructors).
- The C backend emits every constant global as a strict C11 static initializer, including nested shapes, slices, strings, marks and choices. Diagnostics for a forbidden subexpression point at that subexpression.
- A global name may not collide with a function or another global. Non-`mut` globals are immutable, as for locals.

**Modules and imports (Block B1)**

- A source file is a module. `import "path.loi"` loads it and binds its file
  stem as a module name; `import "path.loi" as name` binds `name` instead.
  Paths resolve as before. Each canonical file is loaded once per compilation
  unit, so diamond imports do not duplicate declarations; a cycle is an error
  rather than a silently resolved dedup.
- Imported items still join the compilation unit. The driver resolves module
  names before monomorphization and rewrites references to the declaration's
  internal name; the canonical spelling is in `SINTAXE.md` §13. A name that is
  unique in the compilation unit keeps its source spelling and its existing C
  symbol.
- `open` items may be referenced from any module; `sealed` items (the default)
  may only be referenced from their own module, whether the reference is
  qualified or unqualified. The resolver reports a sealed cross-module reference
  with both module names and the item's declaration span. A module may refer to
  itself by its own name, so self-qualified access to a sealed item is legal.
- `open import "path.loi"` (optionally `as name`) is a re-export: it brings the
  imported module's `open` declarations into the importer's public surface,
  transitively through further `open import`s. It does **not** create a new
  declaration, change the owning module, or change the generated C symbol.
  `sealed` declarations are never re-exported, `sealed import` is rejected, and
  both import forms participate in the same cycle check.
- Qualified names support one level (`module::item`) in this version. There are
  no nested module paths, module-wide visibility or global search paths: paths
  are relative to the importing file (or absolute), and a re-export is a
  declaration-level view, not a namespace.
- **Same names in several modules:** a declaration name may repeat in different
  modules. All contributions with that source name receive the internal name
  `<module>::<name>`; the C backend sanitizes `::` to `__`, so `a::helper`
  becomes `loi_fn_a__helper`. Only a name shared by several modules is mangled,
  which keeps the ABI of single-module programs unchanged. `main`, intrinsics
  and names declared twice inside one module are never mangled: the first two
  cannot be shadowed and the third remains a duplicate-name error.
- **Unqualified resolution:** an unqualified name binds to the current module
  when that module declares it, otherwise to the only declaration elsewhere.
  When several modules declare it and the current module does not, the resolver
  reports `ambiguous name '<name>'` and lists the qualified candidates.
- **ABI visibility:** this rule changes generated C symbol names only for
  duplicated declarations. Headers, multi-file objects and generic instances
  use the same mangled names. `extern flow ... as <symbol>` keeps its explicit
  link name, and several modules may intentionally share one explicit link name.
  Two resolved names that sanitize to the same C identifier are rejected before
  emission unless both are explicit links to that same symbol.

**Equality and ordering**

- `==` and `!=` are defined for scalar numeric types and `bool`, with the usual literal coercion rules, and for two values of the same `mark`, `cstr`, `opaque` handle, shape, choice or slice.
- Comparisons are structural: both operands must have the same type (numeric operands of different widths coerce to their common type). A shape compares fields in declaration order; a choice compares the active variant first, then its payload; a slice compares length and then elements. `[u8]` slices compare bytes by content, not pointer identity.
- `<`, `<=`, `>` and `>=` use the same lexicographic rule for numbers, `bool`, marks, `cstr`, shapes, choices and slices of orderable elements. `cstr` compares as a NUL-terminated byte string. `opaque` handles have identity but no order, and `callback` values are neither comparable nor orderable.
- The checker, monomorphizer and C backend share this rule through `types::ComparisonCaps`/`ComparisonEnv`. The backend emits on-demand `loi_eq_*`/`loi_ord_*` helpers; `[u8]` keeps the `loi_slice_eq` fast path.
- **Implementation limit:** compile-time (`comptime`/global initializer) comparisons still require numeric `ConstValue`s; ordering a composite at compile time reports `comparison requires numeric compile-time values` (see `FRICTION.md`). Runtime comparisons are unaffected.

**Builtin intrinsics and standard library**

- Intrinsics: `print(<- [u8])`, `eprint(<- [u8])`, `panic(<- [u8])`, `print_int(<- integer)`, `print_float(<- f64)`, `printf(<- [u8], ...)` (literal format string plus typed varargs), `sqrt/floor/ceil/fabs(<- f64) => f64`, `pow/fmod(<- f64, <- f64) => f64`, `len([T]) => u64` and
`callback(<- flow) => callback`.
- The library is a set of explicitly imported files: `lib/std.loi` (integer/float and byte-string helpers, including integer-to-decimal conversion), `lib/math.loi` (generic and `comptime` math), `lib/ctype.loi` (ASCII byte helpers), `lib/option.loi` and `lib/result.loi` (optional and error-carrying values), `lib/json.loi` (JSON documents), `lib/jsonrpc.loi` (JSON-RPC 2.0 transport), `lib/lsp.loi` (LSP diagnostics and completion bridge), `lib/process.loi` (process spawning with captured output), `lib/os.loi` and `lib/net.loi` (thin POSIX bindings with C shims), and `lib/stdio.loi` (deterministic byte I/O on the standard streams, also via a C shim). The pure files use only the features above; the five shim-backed files (`os`, `net`, `stdio`, `process`, `lsp`) use `extern flow` declarations and must be linked with their shims.

**Optional and error values (Block A6).**

- `Option<T>` is `shape Option<T> { present: bool = false, value: T }` and
  `Result<T, E>` is `shape Result<T, E> { ok: bool = true, value: T, error: E }`,
  both declared in the standard library. They are shapes because Loi choices
  are not generic and their payloads may not contain slices.
- A `->` out-parameter zero-initializes fields with no default, so the field
  that is not selected is safe to hold as bytes but is not a meaningful value.
  Read it only through the library flows: `option_get_or`, `option_unwrap`,
  `result_unwrap_or`, `result_unwrap` and `result_unwrap_err`. The three
  `unwrap` flows abort with a flow-naming message when the value is absent or
  an error is present.
- Constructors and readers take `<-` inputs and clone them, so a caller needs
  an active `protect` region when `T` or `E` contains slices. With scalar
  payloads the flows are pure and need no region.
- There is no `nil` and no implicit optional unwrapping; presence is always an
  explicit `bool` checked by `option_is_some` / `option_is_none` or
  `result_is_ok` / `result_is_err`.

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
- Supported types are scalars, `bool`, `opaque` handles, `cstr`, `callback`
  as an input-only parameter, borrowed and mutable slices, and shapes whose
  fields are themselves FFI-safe. Variadics and unions require a C shim.
- Extern flows are not checked for a body, ownership effects or allocation
  regions. Their contracts are the binding author's responsibility.

**C callbacks (Block B6, extended with explicit signatures).**

- `callback` is the FFI-only function-pointer type. Plain `callback` is the
  fixed C signature `int32_t (*)(int32_t, int32_t)`; `callback(T, ...) => R`
  names another signature, with `void` allowed only as `R`. Callback values are
  copyable and can be stored in locals, shape fields and slices, but Loi cannot
  call, compare or dereference them.
- `callback(flow)` is a closed compiler intrinsic whose type is inferred from a
  non-`extern` flow: every parameter must use `<-`, and the parameter types and
  result form the callback signature. The checker rejects unknown or `extern`
  flows, non-input parameters and any use whose inferred signature differs from
  the declared callback type. A callback parameter must use `<-`, and an
  `extern flow` cannot return a callback; ordinary Loi flows may.
- The generated C passes the flow's symbol directly. There is no thunk,
  trampoline or userdata, so the C declaration must use the same signature. The
  fixed ABI keeps the shared `LoiCallback` typedef; other signatures get one
  guarded, structurally named typedef shared by generated C and emitted headers.
- The binding generator still skips C function-pointer parameters and results;
  declare those `extern flow`s by hand. `examples/callback_signatures.loi` with
  `examples/callback_signatures_shim.c` is the complete worked program.
- `loi build ... --c-source FILE` compiles and links extra C sources.
  `--cc-flag FLAG` forwards a flag to the C compiler; on link steps the flags
  follow the objects so `-l` libraries resolve normally.

**Binding generation**

- `tools/bindgen/bindgen.py` generates vendored bindings from a C header using
  clang's JSON AST. It emits `opaque` handles, safe `shape`s, typed integer
  constants and `extern flow` declarations, plus an optional C shim and a skip
  report.
- The generator is conservative: callbacks, variadics, unions, bitfields,
  inline arrays, pointer-to-pointer, wide strings, pointer results and
  pointers to records that are not layout-safe are skipped and reported rather
  than guessed.
- A `const T *` parameter is an input buffer and maps to a borrowed `[T]` slice;
  the shim passes `slice.data`. A companion C length parameter is not inferred
  and remains an explicit Loi argument, so the caller passes `bytes.len` when
  the API requires a count. `const char *` is still `cstr`, and a `const T *`
  result stays unsupported because ownership of the buffer is unspecified.
- A mutable `char *` parameter maps to `<-> [u8]`, a writable byte slice. The
  compiler passes `<->` slices as a pointer to the slice struct, so the shim
  takes `LoiSlice *` and forwards `buf->data`; the length parameter stays
  explicit. Writes land in the caller's storage, and a mutable pointer result
  remains unsupported.
- Safe records passed or returned by value, shape pointers and scalar out
  parameters go through generated `memcpy` shims with `_Static_assert`
  size/alignment checks. A shim that takes a slice emits the same guarded
  `LoiSlice` layout as the C backend (`void * data`, `size_t len`).
  Generated symbol names are `loi_bind_<module>_<name>`.
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
- **Compound assignments** (`+=`, `-=`, `*=`, `/=`, `%=`) follow the same rules as
  the corresponding binary operator. The bitwise (`&=`, `|=`, `^=`) and shift
  (`<<=`, `>>=`) forms follow their binary operators too.
- **Bitwise `&`, `|`, `^` and `~`** require integer operands and produce the
  checked common integer type. They do not accept `bool`.
- **Shifts `<<` and `>>`** require integer operands. The count must be in
  `[0, width)`; an out-of-range count traps with `shift count out of range`.
  Left shifts wrap at the declared width. Right shifts are logical for
  unsigned operands and arithmetic for signed ones.
- **`as`** remains an unchecked truncating cast; it is the explicit escape hatch
  for modular conversion.
- **Literal-only constant expressions** are folded by the checker and rejected
  at compile time when they overflow the expected type or divide by a constant
  zero. Non-constant failures are runtime traps.
- **`float` arithmetic** follows C/IEEE-754 and is not trapped.

### 4.11 Evaluation order

Loi, not the C compiler, defines when operands run. Every expression is
evaluated strictly left to right: operator operands, call and intrinsic
arguments, literal elements, an assignment target before its value, an index
base before its index, and a range start before its end. A value read for an
earlier operand is captured before a later operand can change it.

`and` and `or` are the single exception to "all operands run": their right
operand is evaluated only when the left one does not already decide the result.
Within an evaluated operand the same left-to-right rule applies. Compound
assignment evaluates its target place first and captures the old value before an
effectful right operand runs.

This rule makes previously ambiguous programs well defined rather than illegal;
the checker does not reject multiple side effects in one expression. The C
backend must sequence with generated temporaries instead of relying on C's
unspecified evaluation order. The canonical spelling is in `SINTAXE.md` under
"Evaluation order".

---

## 5. Design Decisions (formerly Open Questions)

All nine questions from the 0.2 draft are decided. Each entry states the rule, the reason, and whether the compiler implements it. "Provisional" is kept only for rules we expect to revisit with new evidence; it no longer means "undecided".

### 5.1 `comptime` — must be known at compile time (question 1)

**Decision:** `comptime` is a guarantee, not a hint. It means "this must be computable without runtime state", matching Zig's reading rather than C++ `constexpr`'s optional folding.

- Constant contexts require it: global initializers, shape field defaults, slice/array extents, and explicit `comptime` expressions.
- `comptime expr` is an error unless `expr` consists only of literals, constant globals, `comptime` functions and constant-foldable operators/casts. Allocation, `protect`, I/O and `panic` are forbidden.
- `comptime flow f ...` declares a function that may be called in a constant context. Its parameters must be `<-`; it may not use `->`/`<->`, `alloc`, `protect`, I/O or mutate globals. It may also be called at runtime; the annotation only guarantees that the compiler can evaluate it.
- Evaluation is deterministic, has no side effects, and is bounded: a step limit produces a compile error instead of a compiler hang.
- Compile-time arithmetic follows the same rules as the runtime for the expected result type: signed overflow is an error, unsigned arithmetic wraps at its declared width, shift counts are checked at that width, and `as` remains the explicit truncating cast. `f32` results are rounded to `f32` precision before comparison or emission.
- `comptime` does not create a new type or a new storage class.

**Why:** the first draft was ambiguous because a `comptime` function could receive runtime values. A guarantee gives the compiler something it can rely on, keeps constant contexts honest, and leaves ordinary constant folding free to happen anywhere as an optimization.

**Status:** implemented. The parser accepts `comptime flow` and `comptime expr`; `const_eval.rs` evaluates globals, shape defaults and `comptime` expressions with purity checks, a step budget and a call-depth limit, using the expected type to reproduce runtime width, overflow and `f32` rounding. Marks and choices are first-class compile-time values: mark variants, choice constructors, `is`, equality and payload reads evaluate in constant contexts and match runtime results. Global initializers of any shape are emitted as strict C11 static initializers: plain brace/designated initializers for shapes and choices (choice tags and payloads use nested braces), mark variant macros and named `static` arrays for slice data, with no compound literals. Forbidden calls in a constant report the offending subexpression span.

### 5.1a Generics — explicit instantiation, monomorphized

**Decision:** generic declarations use angle brackets: `shape Vec<T>` and `flow sum<T>`. Every use supplies explicit type arguments: `Vec<i32>`, `sum<i32>(xs)`. There is no inference in the first version.

- Type parameters may be used anywhere a type appears: fields, parameters, results, locals, slice elements and nested instantiations (`Vec<Vec<i32>>`).
- Monomorphization runs before type checking and code generation. It removes every template and emits a concrete declaration per instantiation, with mangled names such as `Vec__i32` and `sum__Vec__i32`.
- The wrong number of type arguments, a missing argument list on a generic declaration, or type arguments on a non-generic flow are compile errors.
- A template that no use instantiates is still validated once with `i32` for every type parameter, so unused broken code is diagnosed. This is a checker-only smoke test; code generation only sees real instantiations.
- Recursive instantiation (for example `f<T>` calling `f<Vec<T>>`) is rejected at a depth limit with a single clear diagnostic.
- A generated name that collides with a user declaration, or two instantiations that mangle to the same identifier, are reported as collisions with both the readable instantiation and the generated name.
- Growable collections remain library work; generics make a future `Vec<T>` implementation possible without changing the language.

**Generic constraints (Block A5, extended with `Ord`).** A type parameter may
declare bounds with `T: Clone + Copy + Eq + Ord`; the canonical spelling is in
`SINTAXE.md` §5.4. There are exactly four constraints in this version:

- `Clone` is satisfied by every Loi type. It documents that a flow copies its
  argument; since deep clone (§4.3) is recursive, this includes shapes and
  choices containing slices. Cloning a value with slices allocates in the
  caller's `protect` region and propagates the allocation effect to callers.
  The bound is still validated, so a misspelled bound cannot be treated as
  `Clone`, and it is a contract for future implementations where some types
  may not be cloneable.
- `Copy` is satisfied by numbers, `bool`, marks, `opaque` handles and `cstr`:
  exactly the types the checker may copy implicitly without `give` or
  `clone`. Shapes, choices and slices are composite and do not satisfy it.
- `Eq` is satisfied by exactly the types the checker accepts with `==`/`!=`:
  numbers, `bool`, marks, `cstr`, `opaque` handles, and shapes, choices or slices
  whose fields, payloads and elements all satisfy `Eq`. `callback` does not.
- `Ord` is satisfied by the types the checker accepts with `<`, `<=`, `>` and
  `>=`: numbers, `bool`, marks, `cstr`, and shapes, choices or slices whose parts
  all satisfy `Ord`. `opaque` handles and `callback` do not.

Bounds are checked in the monomorphizer, when concrete type arguments are
known; a violation is reported at the call site or type use that triggered the
instantiation, with source type spelling. An unused template is still checked
with `i32` for every parameter, and `i32` satisfies all four bounds. Bounds
do not affect mangling, generated C or ABI. `vec_find<T>`/`slice_find<T>`
declare `T: Eq`, while `vec_sort<T>`/`vec_binary_search<T>` declare
`T: Clone + Ord`; `math_min<T>` and friends still require numeric `T` because
arithmetic, not ordering, is their requirement. Constraint names are ordinary
identifiers, not keywords, and an unknown name is a parse error.

**Why:** explicit arguments make monomorphization predictable and keep the first implementation small. Inference can be added later without changing the surface syntax. Deferring bound validation to the monomorphizer avoids re-checking abstract `T` bodies with speculative type rules and keeps the diagnostic on code the user wrote; sharing one structural rule between checker, monomorphizer and backend means a bound never promises a comparison the C backend cannot emit.

**Status:** implemented and tested for shapes, flows, nested instantiations, speculative `>>` token splitting, depth limits, name collisions, unused-template validation and all four bounds (positive and negative cases at call sites and type uses).

### 5.1b Code as a value — comptime templates

**Decision:** `Code` is a comptime-only type holding a block of Loi statements. `quote { ... }` constructs it inside a `comptime flow`; `unquote(expr)` injects a scalar compile-time value into a quoted block; `splice expr` evaluates a `Code`-producing expression at compile time and inserts its statements into the caller. The canonical spelling is in [`SINTAXE.md`](../SINTAXE.md) §5.5a.

- `Code` is data, not a runtime value. It has no C representation, is not comparable, and cannot be stored in a runtime variable or returned by an ordinary flow.
- `quote` is valid only in a `comptime flow`. Quoted statements are code data, not executed by the comptime evaluator; only the inner expression of an `unquote` is evaluated. Misplaced `quote` or `unquote` is reported before expansion.
- An `unquote` inner expression must evaluate to a scalar constant. Composite constants and nested `Code` values are rejected with distinct diagnostics; the canonical messages are listed in `SINTAXE.md` §5.5a.
- `splice` requires a comptime expression whose value is `Code`. Any other value, or an evaluation failure, is rejected with the underlying reason attached as help.
- Expansion runs after module resolution but before monomorphization, type checking, linting and both backends. Every later phase sees the expanded program. Comptime flows that produce `Code` are stripped before C and LLVM emission because they have no runtime body.
- Templates insert already-instantiated statements. There is no hygiene or generated-name facility: identifiers written in a quoted block resolve at the splice site, and a splice expression cannot depend on runtime state.

**Status:** implemented. The parser, AST, pretty-printer and type system accept the three forms; `src/expand.rs` validates placement and expands splices through the pure const evaluator. The C11 production backend and the opt-in LLVM backend lower the expanded program. Positive and negative tests cover parsing, formatting, expansion, placement errors and value-kind errors; `examples/code_as_value.loi` is the realistic two-caller template. Remaining limitations are recorded in `FRICTION.md`.

### 5.2 Pointers — none (question 2)

**Decision:** Loi has no general-purpose pointers, no address-of (`&expr`), no dereference (`*p`), no `*T` and no `nil`. `&` exists only as the bitwise AND operator.

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
- Cross-file visibility is enforced by the driver's module resolver: `open` items are visible from any module, `sealed` items only from their own (§4.8).

**Why:** least privilege, and it matches the compiler's existing `Visibility::Hidden` default. It also removes the inconsistency between the spec and the AST.

**Status:** implemented in AST, parser, checker, codegen and the driver's module resolver; §4.8 defines the cross-file rule.

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
shape Node {
    parent:       u32
    first_child:  u32
    next_sibling: u32
}

no_node: u32 <- 4294967295
```

- Indices are `u32`/`u64`; a sentinel or a `valid` flag marks "none".
- The node array lives in the same `protect` region as the graph, so destruction is region destruction; there is no reference counting and no weak/strong distinction.
- Reuse-after-free is detected with a generation counter: a handle is `shape Handle { index: u32, generation: u32 }` and stale handles are rejected by the library that owns the array.
- Parent pointers, doubly linked lists and general graphs all become index fields.

**Why:** handles preserve single ownership, are trivially copyable, and make the owner explicit. They also produce stable, cache-friendly layouts.

**Status:** no compiler feature is required (slices, integers and `comptime` constants suffice). Standard-library graph/arena helpers are future work.

### 5.6 Non-memory resources — linear close with `guard` (question 6), revised by Block B2

**Decision:** resources are owned linearly and released explicitly. There is no `defer` and no hidden destructor.

- A resource is a `shape` whose name maps by convention to a release flow:
  `<snake_case_shape>_release`. `File` maps to `file_release`, `Connection`
  maps to `connection_release`. The release flow must exist and have exactly
  the signature `flow <name>_release(<-> resource: Shape)` with no `=>` result.
  A wrong or missing flow is a compile error at the `guard`.
- `guard name <- initializer { ... }` binds a resource for the lexical block
  and inserts the release call on **every** exit path: normal completion,
  `=> return`, `break` and `continue`. Nested guards unwind in LIFO order.
  `break`/`continue` release only the guards entered inside the loop being
  left.
- The release call is emitted before any enclosing `protect` arena is restored,
  so a release that reads slices stored in that region sees live data.
- The binding is immutable in source. Explicitly calling the release flow on a
  guarded resource, moving it out (`give`, `<->`), or returning it is a compile
  error: the `guard` owns the release. Returning a scalar field of the resource
  is allowed because it copies the field and leaves the resource intact.
- Only `shape` types qualify; marks, choices, opaque handles and scalars cannot
  be guarded. `guard` is rejected in `comptime` flows, like `protect`.
- `protect` remains memory-only. A resource that owns memory (a file buffer)
  simply contains a slice owned by the same region.

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
    guard conn <- connect(id) {
        if conn.id == 0 { => 0 }   // release runs before this return
        print_int(conn.id)
    }
    => 0
}
```

**Why:** explicit linear ownership fits the existing move rules and needs no garbage collector, no unwinding and no methods. `guard` is sugar over the same rule, not a second mechanism. Deriving the release name from the shape avoids a second declaration and keeps ownership visible at the binding.

**Status:** implemented (checker and C backend). The convention-named release
flow is validated statically; release calls are inserted on normal, return,
`break` and `continue` paths with LIFO unwinding. The module resolver also
checks the synthesized release name against `open`/`sealed`, so a release flow
used by `guard` across modules must be `open`. The standard library provides
linear `File` and `TcpStream` handles backed by thin POSIX shims; whole-file and
stdin access still goes through the `read_file`, `write_file`, `read_stdin` and
`read_line` intrinsics. `lib/stdio.loi` adds incremental `stdio_read_exact`,
`stdio_write_all` and `stdio_flush` flows for framed protocols; they share the C
stdio streams with the print intrinsics and require `lib/stdio_shim.c`.
`lib/json.loi` adds pure-Loi JSON documents: a node table with a decoded-string
arena and a separate child-handle arena, `json_*` accessors, and compact
`json_stringify` output. Strings decode the standard escapes (including
`\uXXXX` surrogate pairs), numbers keep their raw lexeme, object order is
preserved, and malformed input or trailing bytes make `json_parse` return
`false`. The same document type can be assembled with `json_new`, the
`json_new_*` node constructors, `json_array_push`, `json_object_set` and
`json_set_root`; builder numbers keep their lexeme in the string arena, marked
by `JsonNode.from_strings`, so the read accessors work on parsed and built nodes
alike. `json_stringify` escapes every control byte in strings and keys.
`lib/std.loi` provides `i64_to_bytes` and `u64_to_bytes` for the shortest
decimal representation, including `i64::MIN` without signed overflow. Parsing,
building, stringifying and integer conversion allocate, so callers need a
`protect` region.
`lib/jsonrpc.loi` composes the framing of `lib/stdio.loi` with the validation of
`lib/json.loi`: `jsonrpc_read_frame` reads one `Content-Length` body,
`jsonrpc_parse_request` fills a `JsonRpcRequest` (`doc`, `id`, `method`,
`has_id`) after checking the JSON-RPC 2.0 envelope; `id` is optional for
notifications. `jsonrpc_send_result` / `jsonrpc_send_error` emit framed, flushed
replies and `jsonrpc_send_notification` emits a framed message without an `id`.
EOF, a malformed header and a zero-length frame end the exchange; all allocating
flows require `protect`. `lib/process.loi` spawns a child through
`lib/process_shim.c` (`fork`, `/bin/sh -c`, output redirected to temporary files,
`waitpid`) and exposes it as a linear `Process` used with `guard`:
`process_spawn_args` shell-quotes an explicit program and argument list,
`process_stdout` / `process_stderr` copy the captured streams into the caller's
`protect` region, and `process_exit_code` reports the status. `lib/lsp.loi`
bridges the compiler's `--error-format json` diagnostics to
`textDocument/publishDiagnostics`: an LSP server spawns
`loi check FILE --error-format json` with `process_spawn_args`, reads the
compiler's stderr and calls `lsp_publish_diagnostics`, which maps 1-based
compiler spans to 0-based LSP ranges, severity strings to 1/2/3, and assembles
the notification with the JSON builder so messages and URIs are escaped by the
serializer. Empty or malformed input clears previous markers. `lsp_write_document`
stores the buffer sent by `didOpen` / `didChange` in a caller-chosen shadow path
through `lib/lsp_shim.c`, and `lsp_remove_document` deletes it after the check;
the compiler therefore sees the client's text without ever overwriting the
client's file. The example server derives the shadow from a hash of the URI and
places it in the document's own directory so relative imports keep working, and
falls back to reading the real file only for notifications with no text. The
server advertises full-document synchronization, the diagnostics provider and
`completionProvider` (`resolveProvider: false`, `triggerCharacters: ["."]`) in
`initialize`. `os_env` reads the `LOI_COMPILER` setting used to locate the
compiler. The server is shipped as a separate `loi-lsp` executable (like
`clangd` beside `clang`); `cargo build` creates it next to `loi`, and its
launcher caches the compiled Loi server on first use.

`loi complete FILE --line N --column N` exposes the same source-level names to
editors. It tokenizes the raw buffer with 1-based positions and prints a compact
JSON array of `{ "label", "kind", "detail" }` items; it never checks or reports
diagnostics, so a half-typed program still yields candidates. `kind` uses LSP
`CompletionItemKind` numbers: 3 intrinsic/flow, 5 local, 7 shape/opaque, 8
module, 12 mark/choice and 14 keyword. The LSP server keeps a separate cached
copy of each `didOpen` / `didChange` buffer because the check shadow is deleted
after each check; `didClose` removes that cache. Completion requests convert
LSP's 0-based `line`/`character` to the compiler's 1-based positions, call
`loi complete`, and translate the result into a `textDocument/completion`
response. Compiler `help` text has no LSP diagnostic field, so the server
appends it to the displayed message instead.

`loi lsp-info FILE --kind KIND [--line N --column N] [--uri URI] [--new-name NAME]`
exposes compiler-backed editor information. Supported kinds: `hover`,
`definition`, `signature`, `references`, `prepareRename`, `rename`,
`documentSymbol`, `semanticTokens`, `inlayHint`, `foldingRange`. Position-based
kinds take 1-based `--line`/`--column`; `rename` requires `--new-name`; `--uri`
is echoed in location-bearing results. The server advertises the corresponding
capabilities, converts LSP 0-based positions, and forwards requests over the
cached editor buffer. It decodes `%XX` escapes in `file://` URIs and drops the
extra slash before a Windows drive, keeping the separate stdio server model.

The compiler also reports source-level warnings for bindings that are never
read: local `<-` bindings, `for` variables and `guard` bindings. A `_` binding and
parameters are deliberate and stay quiet. Warnings are computed from the
original AST of the checked file, never from an imported module, so their spans
and names are the ones the user wrote. They carry severity `warning` in
`--error-format json` but do not make a program invalid: `check` and `build` exit
non-zero for errors only. Warnings are suppressed when errors are present, so a
broken program does not collect style notes about bindings left unused by the
error. Embedders that want errors only can call `check`; the CLI and the LSP
diagnostics path call `check_with_warnings_from`. Verification and examples live
in `PROGRESS.md`.

### 5.7 Scalar and composite copy — confirmed (question 7), revised by Block A2

**Decision:** the Phase 2 rules were confirmed, then extended by Block A2 to
support recursive deep clones.

- Scalars (`i32`, `f64`, `bool`, ...) copy implicitly: they have no identity, no address and no destructor.
- `clone(x)` duplicates a slice-free composite bit-wise (no allocation).
- A `clone` of a value containing a slice (directly, in a nested shape, or as a slice of slice-containing elements) is a **deep copy**: every slice gets fresh arena storage via the current region, so the result never aliases the source. The source is only read and stays live.
- The clone's result carries the active `protect` region. A clone outside every
  `protect` (including through a flow) is rejected as an unguarded allocation,
  and a clone result may not escape its region. A flow whose body deep-clones is
  treated as an allocator: callers must cover it with `protect`.
- Slices and slice-containing composites still move with `give`/ownership transfer.

**Why:** the original rejection avoided aliasing but blocked ordinary collection
use (`Vec<[u8]>`). With a lexical `protect` region the allocator and lifetime
policy already exist, so `clone` can allocate fresh storage with no dangling
aliases. Helper generation is type-directed and recursive; choices cannot carry
slices, so their helpers stay bit-wise.

**Status:** implemented and covered by Block A2 checker and codegen tests.

### 5.8 File extension — `.loi` (question 8)

**Decision:** source files use `.loi`. `.lum` is abandoned and must not appear in new material.

- The canonical reference program is `examples/particles.loi` (plural, matching the existing file).
- `import` paths and CLI examples use `.loi`.

**Why:** the implementation, tests and examples already use `.loi`; keeping two extensions would add no value.

**Status:** done (documentation aligned).

### 5.9 Standard functions — intrinsics plus an explicit library (question 9)

**Decision:** two tiers, closed.

1. **Intrinsics** are compiler-known, reserved names with fixed signatures and a direct backend mapping:
   - I/O: `print`, `eprint`, `panic`, `print_int`, `print_float`, `printf`, `read_file`, `write_file`, `read_stdin`, `read_line`
   - math: `sqrt`, `floor`, `ceil`, `fabs`, `pow`, `fmod`
   - `len`, `view`, `callback`

   They cannot be redeclared by user code and are always available (no import).
   `view` is type-directed: its element type comes from the source slice, so it
   is checked specially rather than registered with one concrete signature.
   `callback` takes a flow name, checks its signature in the checker and emits
   that flow's C symbol; it likewise cannot use one fixed AST-level signature.
   `read_file`, `read_stdin` and `read_line` allocate in the current `protect`
   region and follow the same escape rules as `alloc`.
 2. **The library** is ordinary Loi organized as packages: a root such as
    `lib/std.loi` re-exports its internal files under `lib/std/*.loi` with
    `open import`. The packages are `core`, `std`, `runtime`, `math`, `os`,
    `net`, `encoding` and `lsp`; flat paths from earlier releases remain as
    compatibility facades. Thin `extern flow` bindings stay at package edges
    with matching C shims (`os_shim.c`, `net_shim.c`, `stdio_shim.c`,
    `process_shim.c`, `lsp_shim.c`). Files are explicitly imported; there is no
    implicit prelude. Each file is a module with a `module::item` qualified form
    (§4.8); library names keep their established prefixes (`min_i32`,
    `math_sum`, `ctype_is_digit`, ...) so their C symbols stay stable even when
    a user module declares the same name.

New functionality goes into the library by default; an intrinsic is added only when the operation must be implemented by the backend or the OS.

**Why:** a tiny closed intrinsic set keeps the compiler small and portable; a Loi-written library exercises the language and can grow without changing the compiler.

**Status:** implemented. `len`, `view` and `callback` are reserved in the checker; the old "user-defined `len` wins" exception is gone. `printf` is the only variadic intrinsic: its first argument must be a string literal, and the conversions in that literal determine the number and kind of the remaining arguments. User-declared variadic flows remain out of scope.

### 5.10 Syntax identity — Loi Flow Syntax (adopted, complete)

The semantics are Loi's. The early surface borrowed Rust vocabulary (`fun`, `interface`, `let`, `[]T`, `->`, `move`/`copy`). **Loi Flow Syntax** is now the only surface. The migration is complete:

- the lexer and parser accept only Flow Syntax;
- `loi fmt` emits Flow Syntax;
- `examples/particles.loi`, `examples/flow_syntax.loi`, `lib/std.loi` and the test sources all use canonical Flow Syntax;
- the legacy keywords and forms have been removed, not merely deprecated.

**Principle:** *braces are scopes and type bodies, brackets are data, arrows are movement.*

The table below records the historical migration mapping. The left column is no longer valid Loi.

| Historical (removed) | Canonical Flow Syntax | Why |
|---|---|---|
| `interface Vec2 { ... }` | `shape Vec2 { ... }` | data-only types; braces delimit type bodies |
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
| `[]T` | `[T]` | bracket = slice type |
| `Vec2{ x: 1 }` | `Vec2 { x: 1 }` | brace = value body |
| `for in p in xs` | `for <- p in xs` | same direction glyphs |
| `for inout p in xs` | `for <-> p in xs` | same |
| call `f(out w)` | `f(-> w)` | same |
| `alloc(out xs, T, n)` | `alloc(-> xs, T, n)` | allocation writes through an out flow |
| `protect` | `protect` | already distinctive; keep it |

**Implemented details**

- **Shapes** use braces: `shape Vec2 { x: f64, y: f64 }`; the closing `}` may be on its own line. `shape Name {}` is an empty shape.
- **Shape literals** use braces: `Vec2 { x: 1, y: 2 }` and `Vec2 {}`. They are unambiguous in `if`/`while`/`for` headers, so the formatter no longer inserts parentheses there.
- **Marks and choices** use braces too: `mark Color { Red, Green }` and `choice Result { Ok: i32, Err: i64 }`. Legacy `[]` bodies still parse for migration, but `loi fmt` emits `{}`.
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
shape Vec2 { x: f64, y: f64 }

shape Particle {
    pos:   Vec2
    vel:   Vec2
    mass:  f64
    alive: bool = true
}

shape World {
    particles: [Particle]
    gravity:   Vec2
    bounds:    Vec2
    tick:      u64 = 0
}

open flow world_new
    <- count: u32
    <- size: Vec2
    -> w: World
{
    mut ps: [Particle]
    alloc(-> ps, Particle, count)
    for i in 0..count {
        ps[i] = Particle {
            pos: Vec2[ x: i as f64 / count as f64 * size.x, y: size.y / 2 ],
            vel: Vec2[ x: i as f64 * 0.1 - 5, y: 0 ],
            mass: 1 + (i % 3) as f64,
        }
    }
    w.particles = give(ps)
    w.gravity = Vec2 { x: 0, y: -9.81 }
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
- [x] Zero-copy slice views with lexical loans and safe borrowed returns (§4.4a)

### Phase 3 — Advanced Features

- [x] `comptime` (semantics in 5.1; pure evaluator implemented)
- [x] Code-as-value templates: `quote` / `unquote` / `splice` (semantics in 5.1b)
- [x] Generics (`Vec<T>` shapes and flows, explicit arguments, monomorphization)
- [ ] Freshness, spatial hints and ordering blocks (see 4.7)

### Phase 4 — Backend

- [x] Code generation to C11 (first target), including a growable `protect` bump arena, checked arithmetic, runtime bounds checks, headers, source maps and multi-file builds
- [x] Foreign C interop (`opaque`, `cstr`, `callback`, `extern flow`, shims) and the binding generator
- [x] Opt-in LLVM backend for the scalar and scalar-slice subset (§6.2); C11 stays the default production backend.
- [ ] Later: x86-64. `src/ir.rs` and `src/asm.rs` are drafts; `--emit-ir` emits draft IR text, while `--emit-asm` is disabled with a clear CLI error until the assembly backend can generate runnable code.

### 6.1 Generated headers

An emitted header is self-contained C11: it defines the shared guarded
`LoiSlice` and `LoiCallback` types, then visible marks, choices and shapes in
dependency order,
then prototypes for visible flows. Every type declaration is guarded so two
headers can be included together, the top-level guard is derived from the
output file name, and prototypes are wrapped in `extern "C"` for C++ callers.
Generated C uses the same guarded type definitions. Flows whose prototypes are
exported have external definitions in the `.c`; sealed flows remain `static
inline` and are never declared in a header.

### 6.2 Opt-in LLVM backend

The C11 backend is the production target. LLVM is an additional, opt-in target
for the scalar and slice subset; it does not replace or change C11 semantics. A
program that builds with both must print the same values, diagnostics and
status.

The LLVM emitter works from the checked, monomorphized AST in a module separate
from `src/ir.rs`: that module is an unfinished draft whose text is not valid
LLVM IR. Generated IR uses opaque pointers and the same C symbol names as the
C11 backend, so scalar `extern flow` declarations link against ordinary C
objects. `clang` is used to assemble and link it; the target triple is left to
the toolchain and the module is compiled with `-Wno-override-module`.

Supported: signed and unsigned integers, `f32`/`f64`, `bool`, string literals,
`print_int`, `print_float`, the math intrinsics `sqrt`/`floor`/`ceil`/`fabs`/
`pow`, flow calls (including recursion and scalar `extern flow`),
`if`/`else`, `while`, `for i in a..b`, `break`, `continue`, compound assignment
and short-circuit `and`/`or`. Locals are lowered to `alloca`/`load`/`store`;
control flow uses explicit `br` blocks.

Slices lower to the same C ABI as `LoiSlice`: the named LLVM type
`%LoiSlice = type { ptr, i64 }` holds `void* data` and `size_t len`, so
`extern flow` declarations and slice parameters keep C-compatible calling
conventions. The emitter supplies IR helpers that mirror the C runtime:
`loi_alloc` is a bump allocator, `loi_alloc_slice` checks the byte-count
product, `loi_index` checks the upper bound, `loi_slice_eq` compares bytes and
`loi_view` checks the view range. `protect` saves and restores the arena top.
The supported slice expressions are slice literals, `<- [T]` parameters,
`len(x)` / `x.len`, indexing and index assignment, `alloc(-> x, T, n)`,
`view(x, a, b)`, byte-wise `print` / `eprint` / `panic` and `==` / `!=`.
Element types are restricted to scalars with a static byte size; nested slices
and composite elements are rejected.

Every other construct is rejected before emission with an `LLVM backend does
not support ...` diagnostic rather than lowered approximately: `guard`,
shapes, marks, choices, opaque handles, globals, callback values, `cstr`,
`comptime`, `foreach`, indirect calls, field access other than `len` and the
remaining intrinsics. `build --backend llvm` is restricted to a single source
file and an executable; C-only build options are rejected with exit 2. The
canonical CLI spelling and the full rejection list are in `SINTAXE.md` §14;
`examples/llvm_scalars.loi` and `examples/llvm_slices.loi` are reference
programs for both backends.

**Status:** implemented for the scalar and scalar-slice subset; C11 remains the
default and the semantic oracle.

---

## 7. Lexical Notes

- **Keywords:** `alloc`, `and`, `as`, `break`, `choice`, `clone`, `comptime`, `continue`, `else`, `extern`, `false`, `flow`, `for`, `give`, `guard`, `if`, `import`, `in`, `is`, `mark`, `mut`, `not`, `opaque`, `open`, `or`, `protect`, `quote`, `sealed`, `shape`, `splice`, `true`, `unquote`, `while`.
- **Direction glyphs:** `<-` (read), `->` (write/out), `<->` (read-write), `=>` (result).
- **FFI types:** `opaque Name`, `cstr`.
- `in` is a keyword only as the separator in `for i in a..b` and `for <- p in xs`; it is not a direction.
- String literals are double-quoted; see 4.8 for escapes and type.
- Numeric literals: decimal integers and floats, `_` separators, exponents, and `0x`/`0b` integer prefixes.
- Multi-character tokens use **longest match first**: `<->`, `<-`, `=>`, `..`, `->`, `<<=`, `>>=`, `<<`, `>>`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `==`, `!=`, `<=`, `>=`.
- Comments start with `//` and run to the end of the line.
- Fields in a `shape` body and arguments in a call are separated by commas or newlines.
- A top-level `:` inside `[...]` marks a shape literal (`Vec2 { x: i32 }`) or a typed slice literal (`[i32: 1, 2]`); without one, `[...]` is an index or an inferred slice literal.

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
