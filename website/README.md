# Loi website

The Loi website is a static site plus a small local Node server that powers the
playground with the real compiler. There are no npm dependencies.

## Pages

| File | Purpose |
|------|---------|
| `index.html` | Landing page |
| `docs.html` | Complete documentation |
| `playground.html` | Editor, presets and compiler output |
| `404.html` | Not-found page |
| `assets/` | Shared CSS, syntax highlighter and page scripts |
| `examples/` | Mirrors of the repository examples, used by the playground and docs links |

## Run the site and playground

Build the compiler first:

```sh
cargo build
node website/server.mjs
```

Then open <http://localhost:4173/>. The playground uses
`/api/health`, `/api/run`, `/api/check` and `/api/format`.

The server looks for the compiler at `target/debug/loi`, then
`target/release/loi`, then `loi` on `PATH`. Set `LOI_BIN` to use a specific
binary:

```sh
LOI_BIN=./target/release/loi node website/server.mjs
```

`PORT` (default `4173`) and `HOST` (default `127.0.0.1`) control the listen
address. `CC` and `AR` are passed through to `loi build`.

## How execution works

Each API request:

1. creates a fresh temporary directory;
2. copies `lib/std.loi` to both `lib/std.loi` and `examples/lib/std.loi`;
3. writes the submitted program to `examples/main.loi`;
4. runs `loi check`, `loi fmt` or `loi build` there;
5. for `/api/run`, executes the resulting binary and captures its output;
6. deletes the temporary directory.

Programs are limited to 200 KB, output is capped at 128 KB, compilation has a
30-second timeout and execution has a 10-second timeout. At most four requests
compile at once.

## Security

The playground executes arbitrary Loi programs on the machine running the
server. Run it locally, keep the default loopback bind and never expose it as a
public service without adding real sandboxing.

## Notes

- The site is deliberately dependency-free: plain HTML, CSS and JavaScript.
- `examples/` contains copies of `../examples/*.loi`; update both when an example
  changes.
- The docs are hand-written HTML so they work without a build step. Keep them in
  sync with `development/LOI_SPEC.md`, `PROGRESS.md` and `DECISIONS.md`.
