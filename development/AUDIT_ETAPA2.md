# Auditoria — ETAPA 2 (testes failing-first)

Data: 2026-11-24. Antes de qualquer alteração em `src/`, foram adicionados 11 testes de regressão que capturam os contratos violados na ETAPA 1. Comando de verificação: `cargo test --all-targets --no-run` (exit 0) seguido de `cargo test --test <suíte> <nome> -- --nocapture` para cada teste.

| Bug | Teste | Suíte | Falha observada antes da correção |
|---|---|---|---|
| AUD-001 | `global_code_values_are_rejected` | `tests/checker.rs` | `expected a global quote to be rejected, got []` (`check` aceita `G: Code <- quote {...}`) |
| AUD-004 | `invalid_printf_length_modifiers_are_rejected` | `tests/checker.rs` | `%llf`, `%lll`, `%Ls`, `%hf` produzem zero diagnósticos |
| AUD-003 | `nested_generic_shape_literals_in_slices_are_checked` | `tests/checker.rs` | `field 'x' has type Vec<i32>, expected i32` (tokens `>>` corrompidos) |
| AUD-007 | `guard_accepts_a_generic_shape_type_after_monomorphization` | `tests/guard.rs` | `guard requires a named shape with a release flow` + `generic type 'Box' must be instantiated before checking` |
| AUD-008 | `rejects_malformed_radix_literals` | `tests/lexer.rs` | `tokenize("0x")` e `tokenize("0b2")` retornam `Ok` |
| AUD-009 | `string_escape_errors_point_at_the_backslash` | `tests/lexer.rs` | `LexError { column: 1, end_column: 6 }` (aponta a abertura, esperado `column: 3, end_column: 6`) |
| AUD-010 | `completion_offers_every_canonical_keyword` | `tests/completion.rs` | `clone`, `quote`, `splice`, `unquote` ausentes |
| AUD-002 | `guard_with_qualified_shape_resolves_its_release_flow` | `tests/modules.rs` | `module 'a' has no item 'http_client'` |
| AUD-005 | `len_keeps_logical_short_circuiting` | `tests/order.rs` | gcc imprime `RL` em vez de `L` |
| AUD-005 | `len_preserves_left_to_right_operand_order` | `tests/order.rs` | gcc imprime `RL` em vez de `LR` |
| AUD-006 | `llvm_matches_c11_arithmetic_traps` | `tests/llvm.rs` | LLVM imprime `-128`, status 0; C11 trap com `integer overflow`, status 1 |

Resultado: **11 novos testes, 11 falhas, 0 falsos positivos**. Nenhum teste existente foi removido, relaxado ou marcado como ignorado; nenhum golden foi atualizado. As correções da ETAPA 3 devem fazer todos passarem sem alterar a semântica das direções, ownership ou backends.

## Apêndice 2 — bugs descobertos na auditoria aprofundada (2026-11-24)

Após o gate de 603 testes, a auditoria dos subagentes encontrou mais quatro bugs reais. Os testes abaixo foram escritos **antes** de qualquer alteração em `src/` e confirmados como vermelhos. Para eliminar qualquer dúvida sobre o estado pré-correção, os cinco testes também foram executados em um worktree isolado no `HEAD` original (`/tmp/opencode/loi-baseline-audit`), com os mesmos arquivos de teste; log em `/tmp/opencode/loi-baseline-red-evidence.log`.

| Teste | Bug | Comando | Resultado antes da correção |
|---|---|---|---|
| `tests/checker.rs::mark_variants_are_accepted_as_read_arguments` | AUD-014 | `cargo test --test checker mark_variants_are_accepted_as_read_arguments` | FAILED — `unknown variable 'Color'` |
| `tests/codegen.rs::large_negative_i64_literal_keeps_its_value_in_c11` | AUD-015 | `cargo test --test codegen large_negative_i64_literal_keeps_its_value_in_c11` | FAILED — C emite `loi_neg_i32(9223372036854775808ULL)` e imprime `0` |
| `tests/llvm.rs::llvm_matches_c11_for_large_negative_i64_literal` | AUD-015 | `cargo test --test llvm llvm_matches_c11_for_large_negative_i64_literal` | FAILED — LLVM imprime `0`, C11 imprime `-9223372036854775808` |
| `tests/checker.rs::give_before_break_inside_an_if_poisons_the_value_after_the_loop` | AUD-016 | `cargo test --test checker give_before_break_inside_an_if_poisons_the_value_after_the_loop` | FAILED no `HEAD` original — uso após `give` é aceito |
| `tests/checker.rs::extern_read_argument_rejects_a_partially_moved_composite` | AUD-017 | `cargo test --test checker extern_read_argument_rejects_a_partially_moved_composite` | FAILED — `peek(p)` com `p.left` movido é aceito |

Nenhum desses testes foi marcado como ignorado nem relaxado após a correção.
