#!/bin/sh
# Check the crate against the declared minimum supported Rust version.
set -eu

cd "$(dirname "$0")/.."

MSRV=$(sed -n 's/^rust-version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
if [ -z "$MSRV" ]; then
    echo "Cargo.toml does not declare rust-version" >&2
    exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
    if [ "${LOI_REQUIRE_MSRV:-0}" = "1" ]; then
        echo "MISSING REQUIRED TOOL: rustup" >&2
        exit 1
    fi
    echo "SKIP: rustup is not installed; cannot check Rust $MSRV" >&2
    exit 0
fi

if ! rustup toolchain list | awk '{print $1}' | grep -Eq "^${MSRV}(-|\.)"; then
    if [ "${LOI_REQUIRE_MSRV:-0}" = "1" ]; then
        echo "MISSING REQUIRED TOOL: rustup toolchain ${MSRV}" >&2
        exit 1
    fi
    echo "SKIP: Rust $MSRV toolchain is not installed; run 'rustup toolchain install $MSRV' to check it" >&2
    exit 0
fi

exec cargo "+${MSRV}" check --all-targets
