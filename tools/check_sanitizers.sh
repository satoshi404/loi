#!/bin/sh
# Compile and run generated Loi programs under UBSan, and attempt ASan+UBSan.
#
# UBSan is required here. ASan is reported explicitly: GCC's ASan cannot map
# shadow memory in the PRoot/Termux development container, while clang's ASan
# works there. Set LOI_REQUIRE_ASAN=1 in CI to make any ASan failure fatal.
set -eu

cd "$(dirname "$0")/.."

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'MISSING REQUIRED TOOL: %s\n' "$1" >&2
        exit 1
    fi
}

require cargo
require cc
require clang

cargo build --quiet
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

examples="flow_syntax marks choices language_basics"
for example in $examples; do
    printf 'generating %s\n' "$example"
    ./target/debug/loi gen "examples/$example.loi" -o "$work/$example.c" >/dev/null
done

printf '\n== UBSan (gcc and clang) ==\n'
for compiler in cc clang; do
    for example in $examples; do
        "$compiler" -std=c11 -fsanitize=undefined -fno-sanitize-recover=all \
            -fno-omit-frame-pointer -g "$work/$example.c" -lm -o "$work/$example-ubsan-$compiler"
        "$work/$example-ubsan-$compiler" > "$work/$example-ubsan-$compiler.out" 2> "$work/$example-ubsan-$compiler.err"
        if [ -s "$work/$example-ubsan-$compiler.err" ]; then
            printf 'UBSan output for %s/%s:\n' "$compiler" "$example" >&2
            cat "$work/$example-ubsan-$compiler.err" >&2
            exit 1
        fi
        printf 'ubsan ok: %s/%s\n' "$compiler" "$example"
    done
done

printf '\n== ASan + UBSan ==\n'
asan_failed=0
for compiler in clang cc; do
    for example in $examples; do
        if ! "$compiler" -std=c11 -fsanitize=address,undefined -fno-sanitize-recover=all \
            -fno-omit-frame-pointer -g "$work/$example.c" -lm \
            -o "$work/$example-asan-$compiler" 2> "$work/$example-asan-$compiler.build.err"; then
            printf 'ASAN BUILD FAILED: %s/%s\n' "$compiler" "$example" >&2
            cat "$work/$example-asan-$compiler.build.err" >&2
            asan_failed=1
            continue
        fi
        if "$work/$example-asan-$compiler" > "$work/$example-asan-$compiler.out" 2> "$work/$example-asan-$compiler.err"; then
            if [ -s "$work/$example-asan-$compiler.err" ]; then
                printf 'ASAN OUTPUT: %s/%s\n' "$compiler" "$example" >&2
                cat "$work/$example-asan-$compiler.err" >&2
                asan_failed=1
            else
                printf 'asan+ubsan ok: %s/%s\n' "$compiler" "$example"
            fi
        else
            printf 'ASAN NOT VALIDATED: %s/%s (runtime failed)\n' "$compiler" "$example" >&2
            sed -n '1,6p' "$work/$example-asan-$compiler.err" >&2 || true
            asan_failed=1
        fi
    done
done

if [ "$asan_failed" -ne 0 ] && [ "${LOI_REQUIRE_ASAN:-0}" = "1" ]; then
    printf 'LOI_REQUIRE_ASAN=1 but ASan validation failed\n' >&2
    exit 1
fi

if [ "$asan_failed" -ne 0 ]; then
    printf 'ASan is not fully validated on this host; see the report above\n' >&2
else
    printf 'ASan+UBSan validated with both gcc and clang\n'
fi
