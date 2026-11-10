# Loi

Loi is a small compiled systems language built around three ideas:

- **single ownership** — one live name per value, with `give` for transfer and
  `clone` for an explicit duplicate;
- **explicit directions** — `<-` reads, `->` initializes, `<->` reads and
  writes, `=>` returns;
- **lexical memory regions** — `protect { ... }` owns everything allocated
  inside it; nothing escapes.

The compiler is written in Rust (edition 2024, standard library only) and emits
C11, so the normal toolchain is Cargo plus a C compiler.

```loi
shape Vec2 [ x: f64, y: f64 ]

open flow main
    => i32
{
    mut total: f64 <- 0
    protect {
        mut points: [Vec2]
        alloc(-> points, Vec2, 3)
        for i in 0..3 {
            points[i] = Vec2[ x: i as f64, y: 2 * i as f64 ]
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
cargo build
./target/debug/loi check examples/flow_syntax.loi
./target/debug/loi build examples/flow_syntax.loi -o flow_demo
./flow_demo
```

The CLI is:

```text
loi <lex|parse|fmt|check> FILE
loi gen FILE [-o out.c] [--emit-h header.h] [--emit-ir ir] [--source-map map.json]
loi build FILE... [-o out] [--emit exe|obj|lib] [--debug|--release] [--rebuild]
          [--emit-c c] [--emit-h header.h] [--emit-ir ir]
          [--arena-size BYTES] [--source-map map.json]
          [--cc-flag F]... [--c-source FILE]...
loi clean FILE [-o out]
```

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
  position. See `SINTAXE.md` for the full rules.
- Foreign C is available through `opaque` handles, `cstr`, `extern flow` and
  vendored shims.

Start with [`SINTAXE.md`](SINTAXE.md) for the canonical surface and
[`examples/flow_syntax.loi`](examples/flow_syntax.loi) for a complete tour.

## Repository layout

| Path | Contents |
|---|---|
| `src/` | Lexer, parser, type/ownership checker, C backend, draft IR/assembly |
| `tests/` | Lexer/parser/checker/codegen/CLI/bindgen integration tests |
| `lib/std.loi` | Minimal standard library written in Loi |
| `examples/` | Reference programs, including `flow_syntax.loi` and `tictactoe.loi` |
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
