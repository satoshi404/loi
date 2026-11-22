# Loi standard library

The library is a set of small packages. Each package has a root file that
re-exports its public declarations and internal files that can be imported
directly when a narrower dependency is preferred.

```loi
import "lib/std.loi"        // broad: re-exports the whole std package
import "lib/std/fmt.loi"    // narrow: only the formatting helpers
```

Paths are explicit and resolve from the importing file (or from the filesystem
root when absolute). There is no global search path and no nested `a::b::c`
module syntax: `open import` is a declaration-level re-export, not a namespace.

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
