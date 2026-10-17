# Lexical Capabilities

> Permissions that exist only at compile time, are tracked linearly, and erase
> to nothing in the generated code.

This document specifies **lexical capabilities**: the first of loi's three new
pillars (capabilities, dual effects, typed dataflow). It is a design document and
a user guide for what is implemented today.

## 1. Motivation

In low-level code, "can this code write here?" is usually answered by
convention, a comment, or a global flag. loi makes it a **typed, lexical token**:

- a capability is a value of type `@cap(...)`;
- it is **linear** — using it (transferring ownership) consumes it;
- it is **refined** — a stronger permission can be weakened to a weaker one;
- it is **zero-cost** — no runtime representation, no ABI slot.

The conceptual model:

> `!Effects` says what a function **does** (a ceiling the caller must permit).
> `@needs` says what a function **requires to exist** (a floor the caller must
> supply). Capabilities are the tokens that satisfy both. See
> [`needs.md`](needs.md) for the dual-effects half.

## 2. Syntax

### 2.1 Declaring capability kinds

```loi
@capability(Read, Write, Exec, IO, Alloc, Filesystem)
```

If a program declares no kinds, any name is accepted (permissive mode). As soon
as one `@capability(...)` appears, unknown names are rejected with a helpful
error.

### 2.2 The capability type

```loi
Type    ::= ... | '@cap' '(' 'borrow'? CapAtom ('+' CapAtom)* ')'
CapAtom ::= Ident
```

- `@cap(Write)` — an owned proof of `Write`.
- `@cap(Read + Alloc)` — a proof of both permissions at once.
- `@cap(borrow Read)` — a **borrowed** proof: valid only for the call, never
  consumed, never storable.

### 2.3 Statements

```loi
Stmt       ::= ... | Grant | Refine | Drop | WithCap | ExprStmt
Grant      ::= '@grant' '(' CapSet ')' Ident ';'
Refine     ::= '@refine' '(' CapAtom '->' CapAtom ')' Ident ';'
Drop       ::= '@drop' '(' Ident ')' ';'
WithCap    ::= '@with_cap' '(' CapSet ')' Block
WithClause ::= 'with' CapSet          // postfix on an ExprStmt: ambient borrow
```

- `@grant(Write) w;` — birth a permission. Only allowed at a **root** (§4).
- `@refine(Write -> Read) w;` — weaken `w` in place.
- `@drop(w);` — explicitly discard an owned capability.
- `@with_cap(Write + Alloc) { ... }` — make capabilities you already hold
  **ambient** (borrowed) for a block. It *borrows* the set from a live binding;
  it does not conjure it, and the ambient bindings never escape the block.
- `poke(&buf) with Write;` — the same idea for a single statement: the expression
  is evaluated with `Write` ambient (borrowed) for that statement only. It is
  exactly `@with_cap(Write) { poke(&buf); }`.

A capability can also be introduced as an ordinary binding:

```loi
@cap(Write) w = make_writer(buf);
```

### 2.4 Signatures

```loi
FuncSig ::= '(' ParamList? ')' (':' Type | '->' Type)? Effects? Needs? Gives?
Needs   ::= '@needs' '(' CapAtom ('+' CapAtom)* ')'
Gives   ::= '@gives' '(' CapAtom ('+' CapAtom)* ')'
```

### 2.5 Regions carrying capabilities

```loi
RegionStmt ::= 'region' Ident ('(' Expr ')')? ('with' CapAtom ('as' Ident)?)? Block
```

`region r(64) with Write as w { ... }` makes `Write` ambient (borrowed) inside
the region. It is released together with the region's memory.

## 3. Typing rules

Let `Δ` be the set of live capability bindings in scope and `E` / `N` the
ambient effects / needs of the enclosing function.

1. **Satisfaction (width):** `@cap(S)` satisfies `@cap(R)` iff `S ⊇ R`. A proof
   of more permissions can stand in for a proof of fewer.
2. **Owned transfer:** passing an owned binding to a `@cap(R)` parameter
   requires it to be live and `⊇ R`, then marks it **consumed**.
3. **Borrow:** passing to `@cap(borrow R)` requires a live binding `⊇ R` and
   does **not** consume it. A borrowed/ambient binding may never be consumed.
4. **Linearity:** an owned capability introduced locally (`@grant`, `@cap`
   declaration) must be consumed, returned, or `@drop`'ed before its scope ends.
   Capability **parameters** are the function's authority and are consumed by
   the body implicitly.
5. **Refinement:** `@refine(A -> B) x` requires `x` live and holding `A`; it
   removes `A` and adds `B` to `x`.
6. **Granting:** `@grant(...)` is allowed only at a root — see §4.
7. **Returning:** `return w;` where `w` is an owned capability and the function
   returns a `@cap(...)` moves `w` out (satisfying linearity).
8. **Effects bridge:** a call requiring `!X` is allowed if `X` is covered by the
   caller's declared effects **or** by a live `@cap(X)`.
9. **Needs:** a call to `f` with `@needs(R)` is allowed if the caller declares a
   superset of `R` (`@needs` propagation) **or** live capabilities cover `R`.
10. **Ambient borrowing:** `@with_cap(S) { ... }` requires some live capability
    binding to already cover `S`; it then introduces ambient (borrowed, non-owned)
    bindings for `S` scoped to the block. The source is not consumed, so it stays
    live after the block.
11. **Elision (inference):** a trailing `@cap(...)` argument may be omitted. It
    is inferred only when **exactly one** live capability binding in scope proves
    the required set — `poke(&x)` stands for `poke(&x, w)`. Zero candidates or
    several candidates is an error naming the count, and a missing argument that
    is *not* a capability parameter still reports an ordinary arity error.
12. **Ambient precedence:** ambient (non-owned) proofs shadow the owned binding
    they were borrowed from when a capability argument is inferred, so
    `poke(&x) with Write;` resolves to the ambient `Write` rather than becoming
    ambiguous with its source.

## 4. Roots

A **root** may create capabilities with `@grant(...)`. A function is a root if:

- it is `main`, **or**
- it is marked `@root` (sugar for `@attr(root)`), **or**
- it declares `@gives(...)` covering the granted set, **or**
- its **return type** is the capability (a *producer*).

This keeps authority explicit: ordinary code can only *use* permissions that
were handed to it, never conjure them.

```loi
@root
mk_write = (): @cap(Write) { @grant(Write) w; return w; }

/* equivalent: the return type authorises the grant */
mk_write2 = (): @cap(Write) { @grant(Write) w; return w; }
```

## 5. Zero-cost

Capabilities are erased:

- `@cap` parameters, returns and locals lower to a bare `i8` placeholder that the
  optimizer removes; they carry no meaningful data and no ABI significance;
- `@grant`, `@refine` and `@drop` emit **no** runtime work;
- calling a function with `@cap` parameters does not change its signature
  semantics.

So the capability discipline costs nothing at runtime.

## 6. Worked example (kernel flavour)

```loi
@package(kernel.mem)
@inc std
@capability(Read, Write)

/* receives a borrowed read proof: does not consume it */
peek = (buf: ptr, c: @cap(borrow Read)): int { return *buf; }

/* consumes a write proof */
poke = (buf: ptr, c: @cap(Write)): void { *buf = 42; }

/* a producer: its return type authorises @grant */
new_page = (buf: ptr): @cap(Write) {
    @grant(Write) w;
    return w;
}

main = (): int {
    int page = 0;
    @cap(Write) w = new_page(&page);
    poke(&page, w);          /* w consumed here */
    return page;             /* 42 */
}
```

## 7. Diagnostics

- `capability 'Write' (in @grant) is not declared; add it to @capability(...)`
- `'helper' cannot @grant('Write'): not @root and no matching @gives(...) declared`
- `capability 'w' was already consumed`
- `cannot consume 'c': it is a borrowed/ambient capability`
- `argument 2 of call to 'poke' requires @cap(Write), but 'r' proves @cap(Read)`
- `live capability 'w' (@cap(Write)) must be consumed, returned, or @drop'ed before the end of its scope`
- `@with_cap('Write'): no live capability provides it; @grant it first (at a root) or drop the block`
- `with('Write'): no live capability provides it; @grant it first (at a root) or drop the clause`
- `cannot infer capability argument 2 of call to 'poke': expected exactly one live @cap(borrow Write) in scope, found 2`
- `call to 'load' @needs('Filesystem'), which is not provided by 'main'`
- `cannot @refine('Write -> Exec') 'w': it does not hold 'Write'`

## 8. Status / roadmap

Implemented: capability kinds, `@cap` types (owned/borrow/sets), `@grant`,
`@refine`, `@drop`, `@with_cap` (ambient borrowing), `expr with Cap` (ambient
borrow for one statement), argument elision (inference), `@needs`/`@gives`,
linearity, refinement, effects bridge, regions-with-capabilities, zero-cost
erasure.

Planned: capabilities as struct fields (so `(Buffer, @cap)` pairs become
structs), fractional permissions (e.g. `@cap(Write[½])`) for shared access, and
typed dataflow (`@flow` / `@in` / `@out` / `@connect`).
