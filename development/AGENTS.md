# AGENTS.md — Loi compiler

Loi is a compiled systems language with single ownership, explicit parameter
directions (`<-` / `->` / `<->` / `=>`) and lexical `protect` memory regions.
This repository contains its Rust compiler, its C11 backend and its standard
library.

## Read first (mandatory, in this order)

1. `development/LOI_SPEC.md` — the language design and rationale.
2. `SINTAXE.md` — the canonical surface reference. Every example, test and
   generated binding must use exactly these forms.
3. `development/PROGRESS.md` — current state, known limitations, next steps.
4. `development/DECISIONS.md` — why each design or implementation choice was
   made.

Do not start work before reading all four. If `development/LOI_SPEC.md` and `SINTAXE.md`
disagree, fix one of them in the same change. If the compiler and the canonical
surface disagree, fix the compiler or record the change explicitly.

## Project facts

- Implementation language: Rust (edition 2024), Cargo, standard library only;
  tests use Rust's built-in test framework.
- Build: `cargo build`. Tests: `cargo test --all-targets`.
- On the PRoot/Termux environment used for development, build and test with
  `RUSTFLAGS="-C linker=/usr/bin/gcc"` and put the host Cargo first:
  `export PATH=/root/.cargo/bin:$PATH`.
- Reference program: `examples/flow_syntax.loi`; `examples/particles.loi` is a
  byte-identical historical mirror. The compiler must lex, parse, check, build
  and run the reference.
- Source layout: `src/` — `token.rs`, `lexer.rs`, `ast.rs`, `parser.rs`,
  `pretty.rs`, `types.rs`, `checker.rs`, `codegen.rs`, plus the draft `ir.rs`
  and `asm.rs`. Integration tests live in `tests/`.
- Runtime library: `lib/std.loi` — canonical Loi, explicitly imported.
- Canonical words: `shape`, `flow`, `open`, `sealed`, `<-`, `->`, `<->`, `=>`,
  `mut`, `give`, `clone`. Do not reintroduce `interface`, `fun`, `let`, `var`,
  `visible`, `hidden`, `out`, `inout`, `move`, `copy`, `return`, `[]T` or
  `Name { ... }`.

### CLI

- `loi <lex|parse|fmt|check> FILE`
- `loi gen FILE [-o out.c] [--emit-h header.h] [--emit-ir ir] [--source-map map.json]`
- `loi build FILE... [-o out] [--emit exe|obj|lib] [--debug|--release] [--rebuild]`
  `[--emit-c c] [--emit-h header.h] [--emit-ir ir] [--arena-size BYTES]`
  `[--source-map map.json] [--cc-flag F]... [--c-source FILE]...`
- `loi clean FILE [-o out]`
- `--help` / `--version`
- `--emit-asm` is **disabled**: the x86-64 assembly backend is an unfinished
  draft and the CLI rejects the flag with exit 2. The C backend is the
  production path.
- `--arena-size BYTES` sets the first `protect` arena chunk (default 64 MiB);
  the arena grows in chunks and is not capped by this value.

## Rules

1. **Verify before claiming.** A task is done only after you ran the build and
   the tests. Write the exact command and its result in `development/PROGRESS.md`. Never
   write "done" from memory or assumption.
2. **Small steps.** Prefer a small correct change over a large incomplete one.
   Write the test together with the feature.
3. **The spec is not yours to change silently.** Do not alter language rules,
   syntax or keywords on your own. If something is ambiguous or wrong, add it
   to "Open Design Questions" in `development/PROGRESS.md`, pick a reasonable temporary
   behavior, mark it clearly, and keep going.
4. **Respect decisions.** Before "fixing" something that looks odd (for example
   no `defer`, `shape` instead of `struct`, `open` / `sealed`), check
   `development/DECISIONS.md`. It may be intentional.
5. **Provisional items** in the spec may change. Do not build large features on
   top of them without saying so in `development/PROGRESS.md`.
6. **No large rewrites.** Never delete or rewrite big parts of working code
   without recording why in `development/DECISIONS.md`.
7. **Keep it modular.** Separate lexer, parser, AST, types and backend.

## After every meaningful task

1. Update `development/PROGRESS.md`: what changed, component status, bugs, next
   steps. Keep it short; summarize old entries instead of piling them up.
2. Add an entry to `development/DECISIONS.md` if you made a design or
   architecture choice (decision, reason, date).
3. If the public surface changed, update `SINTAXE.md`, `website/docs.html` and
   the affected examples in the same change.
4. Run `python3 tools/sync_examples.py` after editing `examples/` so that
   `website/examples/` stays identical.
5. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and
   `cargo test --all-targets` before declaring the task complete.

## One fact, one place

- Language rules live in `development/LOI_SPEC.md`; the canonical spelling of
  every construct lives in `SINTAXE.md`.
- Current status lives only in `development/PROGRESS.md`.
- Reasons live only in `development/DECISIONS.md`.
- Generated binding provenance lives in `vendor/<library>/README.md`.

Do not copy a fact into a second file. Link to it instead.
