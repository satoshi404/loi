# AGENTS.md — Loi compiler

Loi is a compiled systems language with single ownership, explicit parameter directions (`in` / `out` / `inout`) and `protect` memory regions. This repository contains its compiler.

## Read first (mandatory, in this order)

1. `LOI_SPEC.md` — the language design. Single source of truth.
2. `PROGRESS.md` — current state of the compiler, known bugs, next steps.
3. `DECISIONS.md` — why each design or implementation choice was made.

Do not start work before reading all three. If one is missing, create it following the template in `LOI_SPEC.md`.

## Project facts

- Implementation language: Rust (edition 2024), Cargo, standard library only; tests use Rust's built-in test framework.
- Build: `cargo build`.
- Run tests: `cargo test --all-targets`.
- Reference program: `examples/particles.loi` (the spec text also calls it `particle.loi`). The compiler must be able to lex, parse and type-check it.
- Source layout: `src/` — `token.rs`, `lexer.rs`, `ast.rs`, `parser.rs`, `pretty.rs`, `types.rs`, `checker.rs`, `codegen.rs`; integration tests in `tests/`.
- CLI: `cargo run -- <lex|parse|fmt|check|gen> <file.loi>`; `gen [-o out.c] [--emit-h header.h]` emits C11 and `build <file.loi>... [-o out] [--emit exe|obj|lib] [--debug|--release] [--rebuild] [--emit-c c] [--emit-h header.h] [--cc-flag F]...` compiles it with `cc`/`$CC` and `ar`/`$AR` (defaults `./a.out`, `<stem>.o`, `lib<stem>.a`; accepts multiple input files for multi-file builds; objects/headers placed next to each source), skipping unchanged builds via `<output>.loi-stamp` unless `--rebuild` is given. `CC`/`AR` values may include shell-like arguments (quotes and escapes). `clean <file.loi> [-o out]` removes artifacts/stamps and any retained `--emit-c` file recorded in the stamp, without reading the source. `--help`/`--version` are also supported.

## Rules

1. **Verify before claiming.** A task is done only after you ran the build and the tests. Write the exact command and its result in `PROGRESS.md`. Never write "done" from memory or assumption.
2. **Small steps.** Prefer a small correct change over a large incomplete one. Write the test together with the feature.
3. **The spec is not yours to change silently.** Do not alter language rules, syntax or keywords on your own. If something is ambiguous or wrong, add it to "Open Design Questions" in `PROGRESS.md`, pick a reasonable temporary behavior, mark it clearly, and keep going.
4. **Respect decisions.** Before "fixing" something that looks odd (for example no `defer`, `interface` instead of `struct`, `visible` / `hidden`), check `DECISIONS.md`. It may be intentional.
5. **Provisional items** in the spec may change. Do not build large features on top of them without saying so in `PROGRESS.md`.
6. **No large rewrites.** Never delete or rewrite big parts of working code without recording why in `DECISIONS.md`.
7. **Keep it modular.** Separate lexer, parser, AST, types and backend.

## After every meaningful task

1. Update `PROGRESS.md`: what changed, component status, bugs, next steps.
2. Add an entry to `DECISIONS.md` if you made a design or architecture choice (one line: decision, reason, date).
3. Keep `PROGRESS.md` short. Summarize old entries instead of piling them up.

## One fact, one place

- Language rules live only in `LOI_SPEC.md`.
- Current status lives only in `PROGRESS.md`.
- Reasons live only in `DECISIONS.md`.

Do not copy a fact into a second file. Link to it instead.
