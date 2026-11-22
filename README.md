<p align="center">
  <img src="website/assets/logo.svg" alt="Loi" width="72" height="72">
</p>

<h1 align="center">Loi</h1>

<p align="center">
  <em>A compiled systems language with single ownership, explicit directions,
  and lexical memory regions.</em>
</p>

<p align="center">
  Rust 1.88+ &middot; C11 backend &middot; MIT OR Apache-2.0
</p>

---

## Contents

- [Why Loi](#why-loi)
- [Quick start](#quick-start)
- [Command-line interface](#command-line-interface)
- [Language at a glance](#language-at-a-glance)
- [Repository layout](#repository-layout)
- [Documentation](#documentation)
- [Testing](#testing)
- [License](#license)

## Why Loi

Loi is built around three ideas:

- **Single ownership** — one live name per value, with `give` for transfer and
  `clone` for an explicit duplicate.
- **Explicit directions** — `<-` reads, `->` initializes, `<->` reads and
  writes, `=>` returns. Effects are visible at the call site.
- **Lexical memory regions** — `protect { ... }` owns everything allocated
  inside it; nothing escapes.

The compiler is written in Rust (edition 2024, standard library only) and emits
C11, so the normal toolchain is Cargo plus a C compiler. Ambiguous expressions
are not rejected: Loi defines strict left-to-right evaluation for operands,
arguments and assignment, while `and`/`or` keep their short-circuit behavior.

**Requirements:** Rust 1.88 or newer (the verified minimum supported version)
and a C11 compiler.

```loi
shape Vec2 { x: f64, y: f64 }

open flow main
    => i32
{
    mut total: f64 <- 0
    protect {
        mut points: [Vec2]
        alloc(-> points, Vec2, 3)
        for i in 0..3 {
            points[i] = Vec2 { x: i as f64, y: 2 * i as f64 }
            total += points[i].x + points[i].y
        }
    }
    if total != 9 {
        => 1
    }
    => 0
}
```

## Quick start

```sh
cargo build                         # builds both loi and the separate loi-lsp
./target/debug/loi check examples/flow_syntax.loi
./target/debug/loi build examples/flow_syntax.loi -o flow_demo
./flow_demo
```

## Command-line interface

```text
loi <lex|parse|fmt|check> FILE
loi complete FILE --line N --column N
loi lsp-info FILE --kind KIND [--line N --column N] [--uri URI] [--new-name NAME]
loi gen FILE [-o out.c] [--emit-h header.h] [--emit-ir ir] [--source-map map.json]
loi build FILE... [-o out] [--emit exe|obj|lib] [--debug|--release] [--rebuild]
          [--emit-c c] [--emit-h header.h] [--emit-ir ir]
          [--arena-size BYTES] [--source-map map.json]
          [--cc-flag F]... [--c-source FILE]...
loi clean FILE [-o out]
```

`cargo build` also produces `target/debug/loi-lsp`, a separate stdio language
server like `clangd`. Editors start it directly; it compiles and caches the
server written in `examples/lsp_server.loi` on first launch and uses the
compiler next to it for `loi check`. Set `LOI_COMPILER` to override that
compiler, or `LOI_LSP_CACHE_DIR` to move the cached server. `initialize`
advertises full-document sync, diagnostics, completion, hover, definition,
references, rename, document symbols, semantic tokens, inlay hints, folding
ranges, code actions and signature help; warnings arrive as severity 2,
compiler suggestions are appended to diagnostic messages, and editor requests
are answered from a cache of the editor buffer. Editor requests are forwarded to
`loi lsp-info FILE --kind ... [--line N --column N] [--uri URI] [--new-name NAME]`.
`loi complete FILE --line N --column N` is the same source-level completion for
direct editor use. File URIs are normalized to forward slashes, percent escapes
are decoded and the extra slash before a Windows drive is removed before the
compiler opens the path.

For VS Code, install the packaged `editors/vscode-loi` extension. It registers
the `.loi` language with syntax highlighting, snippets and bracket matching,
ships a `vs-seti`-based file icon theme with the Loi icon for `.loi` files, and
uses a real PNG icon in the Extensions view. It discovers `loi-lsp` on PATH and
in the workspace's `target/debug` / `target/release` folders, pairs it with the
`loi` binary next to it, and reports a specific error if neither is found. Set
`loi.lsp.command`, `loi.compiler` or `loi.lsp.cacheDir` when those defaults do
not fit your setup.

Every mode accepts `--error-format human|short|json` (default `human`) and
`--color auto|always|never` (default `auto`). Human diagnostics show the source
line with labelled underlines and a final error count; `short` keeps the
historical `path:line:column: message` form for tools and editors; `json`
emits one machine-readable object per diagnostic. `auto` colors only when
standard error is a terminal and honors `NO_COLOR`; `CLICOLOR_FORCE` overrides
that detection.

`gen` writes C11; `build` invokes `cc` (or `$CC`) and `ar` (or `$AR`) and
supports incremental builds. `--emit-asm` is deliberately rejected: the x86-64
assembly backend is an unfinished draft, and the C backend is the production
path.

On the PRoot/Termux development environment, use the host toolchain:

```sh
export PATH=/root/.cargo/bin:$PATH
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo test --all-targets
```

## Language at a glance

- `shape` declares data; `flow` declares functions. Bodies use braces, data
  uses brackets, movement uses arrows.
- Slices are `[T]`, strings are `[u8]`, and indexing is bounds-checked.
- `protect` is a growable bump arena. `--arena-size` sets the initial chunk.
- Signed integer arithmetic traps on overflow; unsigned arithmetic wraps at
  its declared width; division and remainder by zero trap with a source
  position.
- Bitwise and shift operators (`&`, `|`, `^`, `~`, `<<`, `>>`) and compound
  forms are available for masks, flags and hashing; shift counts are checked.
- Operands, arguments and assignment are evaluated strictly left to right;
  `and` and `or` short-circuit. See `SINTAXE.md` under "Evaluation order".
- `read_file`, `write_file`, `read_stdin` and `read_line` cover whole-file and
  stdin I/O. `print_int`, `print_float` and `printf` print numbers and formatted
  text without building strings by hand; `printf` checks its varargs against a
  literal format string. Reads allocate in the current `protect` region.
- `loi fmt` preserves `//` comments and is idempotent.
- Foreign C is available through `opaque` handles, `cstr`, `extern flow` and
  vendored shims.
- The explicit library is modular: `lib/std.loi`, `lib/math.loi` and
  `lib/ctype.loi` are pure Loi; `lib/os.loi` and `lib/net.loi` are thin bindings
  with POSIX shims.

Start with [`SINTAXE.md`](SINTAXE.md) for the canonical surface and
[`examples/flow_syntax.loi`](examples/flow_syntax.loi) for a complete tour.
[`examples/marks.loi`](examples/marks.loi) and
[`examples/choices.loi`](examples/choices.loi) show finite tag sets and checked
tagged payloads. The raylib examples include an automatic sound synth and a
C3-inspired audio visualizer; see [`website/docs.html`](website/docs.html) for
their build commands.

## Repository layout

| Path | Contents |
|---|---|
| `src/` | Compiler binaries (`loi` and the `loi-lsp` launcher), lexer, parser, type/ownership checker, C backend, draft IR/assembly |
| `tests/` | Lexer/parser/checker/codegen/CLI/bindgen integration tests |
| `lib/*.loi`, `lib/*_shim.c` | Modular library: pure Loi helpers plus POSIX shims for `os`/`net` |
| `examples/` | Reference programs, including `flow_syntax.loi`, `marks.loi`, `choices.loi`, `systems.loi` and `tictactoe.loi` |
| `vendor/` | Generated Loi bindings for SDL2 and raylib 5.5, with shims and skip reports |
| `tools/bindgen/` | Header-to-Loi binding generator |
| `website/` | Static site, documentation and local playground |
| `development/` | Language spec, progress, decisions and agent instructions |

## Documentation

- [`SINTAXE.md`](SINTAXE.md) — canonical syntax reference.
- [`development/LOI_SPEC.md`](development/LOI_SPEC.md) — language design and
  rationale.
- [`development/PROGRESS.md`](development/PROGRESS.md) — current compiler
  state and limitations.
- [`development/DECISIONS.md`](development/DECISIONS.md) — decision log.
- [`website/docs.html`](website/docs.html) — public documentation.
- [`website/README.md`](website/README.md) — how to run the site and
  playground.

## Testing

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Compile-and-run tests are skipped when `cc` is unavailable. The playground and
the binding generator have their own prerequisites (`node` and `clang`
respectively); see `website/README.md` and `vendor/<library>/README.md`.

## License

MIT OR Apache-2.0.
