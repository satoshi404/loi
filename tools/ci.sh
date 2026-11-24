#!/bin/sh
# Reproducible gate for the Loi compiler. Each step is separate so CI logs show
# exactly which contract failed. Missing required tools are reported by name
# instead of being silently turned into skipped tests.
#
# Usage: tools/ci.sh [all|<step>]   (default: all)
set -eu

cd "$(dirname "$0")/.."
TARGET=${1:-all}

step() {
    printf '\n== %s ==\n' "$1"
}

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'MISSING REQUIRED TOOL: %s\n' "$1" >&2
        exit 1
    fi
}

report_tool() {
    if command -v "$1" >/dev/null 2>&1; then
        printf '%s: %s\n' "$1" "$($1 --version 2>/dev/null | head -1)"
    else
        printf 'OPTIONAL TOOL ABSENT: %s\n' "$1" >&2
    fi
}

run_step() {
    if [ "$TARGET" = all ] || [ "$TARGET" = "$1" ]; then
        step "$1"
        shift
        "$@"
    fi
}

do_tool_inventory() {
    for tool in rustc cargo cc clang node python3 ar; do
        report_tool "$tool"
    done
}

do_fmt() {
    require cargo
    cargo fmt --check
}

do_clippy() {
    cargo clippy --all-targets -- -D warnings
}

do_tests() {
    require python3
    log=$(mktemp)
    if cargo test --all-targets > "$log" 2>&1; then
        status=0
    else
        status=1
    fi
    cat "$log"
    if [ "$status" -eq 0 ]; then
        python3 tools/check_test_output.py "$log" || status=1
    fi
    rm -f "$log"
    return "$status"
}

do_goldens() {
    require python3
    python3 tools/check_goldens.py
}

do_msrv() {
    sh tools/check_msrv.sh
}

do_examples() {
    cargo build --quiet
    for example in \
        examples/flow_syntax.loi \
        examples/language_basics.loi \
        examples/marks.loi \
        examples/choices.loi \
        examples/recent_features.loi \
        examples/guard_resources.loi \
        examples/llvm_scalars.loi
    do
        printf 'checking %s\n' "$example"
        ./target/debug/loi check "$example"
    done
}

do_sync() {
    require python3
    python3 tools/sync_examples.py --check
}

do_diff_check() {
    require git
    git diff --check
}

do_c11_strict() {
    require cc
    cargo build --quiet
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    for example in flow_syntax marks choices recent_features language_basics; do
        ./target/debug/loi gen "examples/$example.loi" -o "$work/$example.c"
        cc -std=c11 -pedantic-errors -Wall -Wextra -fsyntax-only "$work/$example.c"
    done
}

do_runtime() {
    cargo test --test codegen --test stdio --test guard
}

do_contracts() {
    cargo test --test checker --test modules --test views --test errors \
        --test diagnostic_render --test semantic_tokens
}

do_sanitizers() {
    sh tools/check_sanitizers.sh
}

do_fuzz_smoke() {
    cargo test --test fuzz_smoke --test fuzz_stages --test property_smoke
}

do_differential() {
    # The long GCC/Clang order differential already ran in the `tests` step.
    # This step adds the C11/LLVM backend differential without repeating it.
    require clang
    cargo test --test llvm
}

case "$TARGET" in
    all | tool-inventory | fmt | clippy | tests | goldens | msrv | examples | sync | diff-check | c11-strict | sanitizers | runtime | contracts | fuzz-smoke | differential) ;;
    *)
        printf 'unknown step: %s\n' "$TARGET" >&2
        exit 2
        ;;
esac

run_step tool-inventory do_tool_inventory
run_step fmt do_fmt
run_step clippy do_clippy
run_step tests do_tests
run_step goldens do_goldens
run_step msrv do_msrv
run_step examples do_examples
run_step sync do_sync
run_step diff-check do_diff_check
run_step c11-strict do_c11_strict
run_step sanitizers do_sanitizers
run_step runtime do_runtime
run_step contracts do_contracts
run_step fuzz-smoke do_fuzz_smoke
run_step differential do_differential

step done
printf 'selected CI steps passed\n'
