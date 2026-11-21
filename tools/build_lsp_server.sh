#!/usr/bin/env sh
# Build the Loi language server directly, without the Cargo launcher.
#
# Normal installs only need `cargo build`, which produces both `loi` and the
# separate `loi-lsp` executable. This script is useful when you want a plain
# server binary (for example to embed in an editor config) from a source
# checkout.
#
# Usage (from the repository root):
#   sh tools/build_lsp_server.sh [OUTPUT]
#
# OUTPUT defaults to ./loi-lsp. LOI_BIN selects the compiler (default: target/debug/loi).
set -eu

OUT=${1:-loi-lsp}
LOI_BIN=${LOI_BIN:-target/debug/loi}
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if command -v "$LOI_BIN" >/dev/null 2>&1; then
    COMPILER=$LOI_BIN
else
    COMPILER="$ROOT/$LOI_BIN"
fi

cd "$ROOT"
exec "$COMPILER" build examples/lsp_server.loi -o "$OUT" \
    --c-source lib/stdio_shim.c \
    --c-source lib/process_shim.c \
    --c-source lib/os_shim.c \
    --c-source lib/lsp_shim.c
