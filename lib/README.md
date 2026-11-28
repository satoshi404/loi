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

`build.loi` is a toolchain runner, not another stdlib package. `loi make`
compiles it from this directory and executes it with `LOI_SELF` and
`LOI_TOOLCHAIN_LIB`. A project script creates a `loi_build::BuildPlan` with
`new`, adds `source`, `lib` and `cc_flag` entries, selects `mode` / `optimize`
/ `target` / `profile`, then calls `run`. The runner honors
`LOI_BUILD_TARGET` / `LOI_BUILD_PROFILE` / `LOI_BUILD_REBUILD` over the plan and
caches a fingerprint of the script, the plan and every source byte in
`output/.loi-build-cache`. A scaffolded project imports it as
`import build as loi_build`; putting `lib/build.loi` in the project overrides it
because logical imports prefer the nearest library root. The runner needs
`process_shim.c` (linked explicitly) and the shims found by `--auto-c-source`.

`build/model.loi` and `build/validate.loi` are the opt-in FASE 2 model of the
same toolchain. A script imports them as `import build::model` and
`import build::validate`; `model::project(name)` creates a `Project`, mutators
return stable vector indices for targets/tasks/artifacts, and
`validate::validate_project(project)` checks ids, declared inputs, config and
dependency endpoints. The shapes are `Project`, `BuildTarget`, `BuildTask`,
`Artifact`, `BuildDependency`, `BuildConfig` and `BuildContext`; config is copied
to each target and the context carries root/toolchain/output_dir explicitly.

FASE 3 adds `build/graph.loi` and `build/order.loi` on top of that model.
`graph::build_graph(project)` returns a `BuildGraph` whose nodes follow
declaration order (targets, tasks, artifacts) and whose edges are deduplicated
by `(from, to, kind)`; `graph::target_sources` returns the deduplicated source
list of one target. `order::order_project(project)` runs a deterministic Kahn
sort: an edge `from -> to` means `from` depends on `to`, so `to` is emitted
first, ties are broken by declaration order, and a cycle panics with
`dependency cycle`. Edges of kind `input` are kept for future hashing but are
not graph nodes. None of these modules executes targets, writes manifests or
changes `loi init` / `loi make`: the legacy `BuildPlan` above remains the active
runner path.

FASE 4 adds `build/cache.loi`, also opt-in. `cache::artifact_key(project, id,
program)` returns a versioned FNV-1a key over the target identity, its config,
the effective compiler path and the bytes of every declared input: sources, C
sources, headers, assets, include directories, defines, libs, compiler flags,
link flags, dependency ids, task references and output path. FASE 7 extends the
key with the transitive contents of declared dependencies and writes manifests
through a temporary file plus rename (`loi_build_write_atomic` in `os_shim.c`),
so an interrupted write cannot leave a false success. `cache::needs_build` is
false only when the output and its manifest both exist and still carry that key;
`cache::write_manifest` commits the key after a successful build. The manifest
lives at the path in a declared `Artifact`, otherwise at
`output/.loi-build/<id>.cache`; its parent directory is created by
`loi_build_mkdir`. Headers and assets are declared with `model::add_header` and
`model::add_asset`. Nothing here reads the environment or scans directories, and
`loi make` still uses the legacy `output/.loi-build-cache` stamp.

FASE 5 adds `build/exec.loi`, also opt-in. `exec::run_project(project, program)`
validates the project, obtains the deterministic order from `order::order_project`
and visits one node at a time. For a target it checks `cache::needs_build`,
creates only the declared output/objects directories, calls the compiler through
`process_spawn_args` with an explicit argument vector, forwards stdout/stderr and
writes the manifest only after a zero exit status. Artifacts are descriptors and
are skipped. Tasks use a closed vocabulary: `operation` stays `noop` or
`generate`, while `set_task_action` selects `noop`, `write`, `copy` or `concat`,
`set_task_content` supplies literal bytes and `set_task_condition` selects
`changed` or `always`. `write` creates one declared output, `copy` reproduces a
declared input and `concat` joins declared inputs in order; no string is ever
interpreted as a shell command. Validation rejects duplicate output paths and
requires outputs under `output_dir`; the graph adds implicit `generate` edges
from a declared output's producer to its consumer. Task cache keys use the
`loi-task-v1` prefix, hash the task fields and declared input bytes, and commit
`output/.loi-build/<id>.cache` (or the declared `Artifact` manifest) only after a
successful action. The caller supplies the compiler path (typically `LOI_SELF`
read outside this module), so the executor itself reads no environment. Linking a
script that imports `build::exec` requires `os_shim.c` and `process_shim.c`;
`operation "run"`, parallelism and `loi make` remain unchanged. FASE 6 adds
`test`, `install` and `clean` below.

FASE 6 extends the same opt-in build modules. `model::asset_dir` (alias
`model::asset`) declares one source tree and destination; `cache::tree_paths`
uses `loi_build_list_files` in `os_shim.c` to return relative paths in lexical
order, skips symlinks and includes their names and bytes in the task key. The
executor copies that declared tree only when the key changes. `model::embed_file`
turns one declared file into a public Loi byte array named `_loi_embed_<id>`
(`open <symbol>: [u8] <- [u8: ...]`); the generated output must be declared as a
target input so a change invalidates the target. `model::test` declares a Loi
program compiled to `output/tests/<id>`; `exec::run_tests(project, program,
filter)` compiles through `process_spawn_args`, always executes the binary even
when compilation is cached, forwards output and commits the task manifest only
after a zero exit. An empty filter runs every test in declaration order; a
non-empty filter names exactly one. `model::install` copies one declared input
to a destination inside the project with permission bits preserved;
`exec::run_install(project, program)` runs install tasks in declaration order.
`model::clean` plus `model::add_clean_path` names exact paths removed by
`exec::run_clean(project)`. Validation rejects absolute paths and `..` in
install/clean paths before any file is touched, and recognizes outputs produced
by targets. `operation "run"` remains rejected: no string becomes a shell
command. Scripts still need `os_shim.c` and `process_shim.c`; `loi make`, the
CLI, the ABI and legacy `build.loi` are unaffected.

FASE 7 adds explicit toolchain/configuration support. `model::BuildToolchain`
separates host OS/arch/ABI, target OS/arch/ABI, C compiler, linker and archiver;
`model::toolchain()` returns conservative native defaults without reading the
environment. `model::shared_library(id)` declares an `output/lib<id>.so` target,
and validation accepts a non-native triple only when the build context names an
explicit toolchain — no compiler, linker or universal flag is guessed.
`build::config` documents named profiles (`debug`, `test`, `native`, `release`,
`speed`, `size`, `web`) with explicit optimize/mode rules and propagates them to
all targets. `config::apply_env(project, -> changes)` applies the documented
overrides `LOI_BUILD_PROFILE`, `LOI_BUILD_OPTIMIZE`, `LOI_BUILD_MODE`,
`LOI_BUILD_C_COMPILER` and `LOI_BUILD_ARCHIVER`, prints every override in the
form `[config] profile <- env LOI_BUILD_PROFILE=release`, and returns the same
strings. Precedence is defaults < profile < explicit `set_*` mutation <
environment; a caller that must beat the environment applies `apply_env` before
its final explicit mutation. The effective compiler path also participates in
the artifact key, so changing it rebuilds the target.

FASE 8 closes the migration contract without changing the active CLI path.
`loi init` and `loi make` still scaffold and run the legacy `BuildPlan` in
`lib/build.loi`; the graph model (`build::model`, `build::graph`, `build::order`,
`build::cache`, `build::exec`, `build::config`, `build::validate`) remains an
explicit opt-in surface for scripts that import it. No legacy API was removed or
silently rewritten as a wrapper, and no parallel scheduling, implicit source
discovery, shared library on non-native targets, cross toolchain download or
JSON build description was introduced. See `SINTAXE.md` §19.2 for the public
summary and `development/PROGRESS.md` for the phase evidence and limitations.

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
flows still need their package shim. `os_path_exists` / `path_exists` in
`os/file.loi` takes a runtime byte-slice path and returns `false` for a missing
entry through `loi_os_path_exists` in `os_shim.c`. No whole shim file has been
removed: the remaining functions perform ABI conversion, state ownership or
process work.

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
