# Mutation smoke (manual, no new tooling)

Mutation testing is intentionally **not automated** with a heavy framework: the crate has no dependencies and the MSRV is 1.88. Instead, use this manual procedure in a disposable worktree/copy of the repository. Each mutation must be small, reversible and checked by at least one existing regression test. Never run these mutations on the user's working tree.

## Procedure

```sh
git worktree add /tmp/loi-mutation -b mutation-check HEAD
cd /tmp/loi-mutation
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
# apply ONE mutation below
cargo test --all-targets
git checkout -- <mutated file>
```

A useful mutation is **effective** when the named test fails. If it passes, the suite has a coverage gap and a new regression test is needed.

## Effective mutations to verify

| # | File / function | Mutation | Expected failing test |
|---|---|---|---|
| 1 | `src/types.rs::release_flow_name_for` | remove `_release` from the qualified branch | `tests/modules.rs::guard_with_qualified_shape_resolves_its_release_flow` |
| 2 | `src/monomorphize.rs::StmtKind::Guard` | change `*ty = self.rewrite_type(ty, mapping)` to `self.rewrite_type(ty, mapping);` | `tests/guard.rs::guard_accepts_a_generic_shape_type_after_monomorphization` |
| 3 | `src/codegen.rs` intrinsic `len` branch | restore eager `gen_expr` before emitting the temporary | `tests/order.rs::len_keeps_logical_short_circuiting` |
| 4 | `src/format.rs::length_is_valid` | accept `"ll"` for float conversions | `tests/checker.rs::invalid_printf_length_modifiers_are_rejected` |
| 5 | `src/lexer.rs` radix check | remove the `digits == 0` guard | `tests/lexer.rs::rejects_malformed_radix_literals` |
| 6 | `src/expand.rs::validate_program` | skip `Item::Global` | `tests/checker.rs::global_code_values_are_rejected` |
| 7 | `src/llvm.rs::checked_fail` | make the trap function return instead of failing | `tests/llvm.rs::llvm_matches_c11_arithmetic_traps` |
| 8 | `src/diagnostic.rs::render_sourced` | restore `sorted.len()` as the error count | `tests/diagnostic_render.rs::sourced_rendering_summarizes_warnings_as_warnings` |
| 9 | `src/lsp_info.rs::semantic_token_type` | remove `TokenKind::Quote` from keyword branch | `tests/semantic_tokens.rs::comptime_keywords_are_semantic_keywords` |
| 10 | `src/parser.rs::restore_speculation` | restore only `position`, not split tokens | `tests/checker.rs::nested_generic_shape_literals_in_slices_are_checked` |

## Current status

This is a documented procedure, not a claim that mutations were executed. No mutation results are asserted in `PROGRESS.md` unless a log is recorded.
