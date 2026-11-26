# Loi standard library

The library is a set of small packages. Each package has a root file that
re-exports its public declarations and internal files that can be imported
directly when a narrower dependency is preferred.

```loi
import "lib/std.loi"        // broad: re-exports the whole std package
import "lib/std/fmt.loi"    // narrow: only the formatting helpers
```

Path imports are explicit and resolve from the importing file (or from the
filesystem root when absolute). Logical imports such as `std::fmt` and groups
such as `std::{fmt, vec}` resolve to `lib/std/fmt.loi` and `lib/std/vec.loi`
through the nearest `lib/` directory found by walking ancestors of the importing
file. There is no global search path, manifest, cwd lookup or download. Logical
paths may contain several segments, but they name files, not nested declaration
namespaces: `open import` remains a declaration-level re-export.

For a project outside this repository, pass this directory explicitly:
`loi check main.loi --lib-dir /path/to/loi/lib` (also accepted by `gen` and
`build`, repeatable). A nearer project `lib/` still wins; `--auto-c-source`
discovers shims from these roots too.

| Package | Root | Internal files | C shim |
|---|---|---|---|
| Foundation | `core.loi` | `core/option.loi`, `core/result.loi`, `core/cmp.loi`, `core/mem.loi`, `core/ops.loi` | none |
| Everyday utilities | `std.loi` | `std/num.loi`, `std/convert.loi`, `std/str.loi`, `std/vec.loi`, `std/slice.loi`, `std/map.loi`, `std/hash.loi`, `std/crc32.loi`, `std/random.loi`, `std/ctype.loi`, `std/fmt.loi`, `std/debug.loi`, `std/iter.loi` | none |
| Runtime concepts | `runtime.loi` | `runtime/arena.loi`, `runtime/panic.loi`, `runtime/hooks.loi` | none today |
| Pure math | `math.loi` | `math/const.loi`, `math/generic.loi`, `math/float.loi`, `math/integer.loi` | none |
| Operating system | `os.loi` | `os/env.loi`, `os/time.loi`, `os/file.loi`, `os/path.loi` | `os_shim.c` |
| Networking | `net.loi` | `net/address.loi`, `net/socket.loi`, `net/tcp.loi`, `net/udp.loi` | `net_shim.c` |
| Encodings | `encoding.loi` | `encoding/json.loi`, `encoding/jsonrpc.loi` | `stdio_shim.c` only for JSON-RPC I/O |
| LSP helpers | `lsp.loi` | `lsp/protocol.loi`, `lsp/shim.loi` | `lsp_shim.c`, plus stdio via JSON-RPC |

`std/fmt.loi` exposes short aliases `label`, `line` and `parens` next to the
original `fmt_*` flows, so `import std::fmt` reads `fmt::line(...)` instead of
`fmt::fmt_line(...)`. The convention now covers narrow modules across the
packages: `std::vec` has `new`, `size`, `push`, `get` and `sort`, `core::option`
has `some`/`unwrap`, `math::generic` has `min`/`sum`, and so on. Each alias is a
thin wrapper; prefixed flows, generated symbols and broad package imports are
unchanged. A few names are intentionally absent: `len` is an intrinsic (so
`vec::size`), `view`/`clone` are reserved, and `bytes`, `copy`, `root`, `state`,
`crc32::bytes` and `map::str_len`/`str_contains` collide in a broad root or
with locals in another module. Imported aliases are program-wide declarations,
so narrow imports remain the recommended style.

`os_pid`, `os_exit`, `io_close` and `net_close` bind directly to libc
(`getpid`, `exit`, `close`) through `extern flow ... as`; the other file-backed
flows still need their package shim. No whole shim file has been removed: the
remaining functions perform ABI conversion, state ownership or process work.

Dependencies point one way: `core` imports none of the other packages; `std`
may use `core`; `runtime` is independent; `os` may use `core`/`runtime`; `net`
may use `core`/`os`; `encoding` uses `std` and `os/stdio`; `lsp` uses `encoding`
and `std`. Vendored bindings under `../vendor/` are separate packages.

Flat paths such as `lib/vec.loi` and `lib/json.loi` remain as compatibility
facades. They use `open import` with an explicit alias and resolve to the same
declarations and C symbols as the new internal files.

`open` declarations are public; `sealed` declarations stay private to their own
file and are never re-exported. Flows that allocate still require the caller to
write `protect`, and flows that own non-memory resources use `guard`; the
library never hides `alloc`, `give`, `clone`, `view` or `guard` ownership.

Realistic programs: `../examples/package_core.loi`,
`../examples/package_std.loi`, `../examples/package_runtime.loi`,
`../examples/package_math.loi`, `../examples/package_os.loi`,
`../examples/package_net.loi` and `../examples/package_encoding.loi`.
