# Auditoria — ETAPA 4 (regressão e differential)

Data: 2026-11-24. Nenhum teste foi removido, relaxado ou marcado como ignorado; nenhum golden foi atualizado em massa.

## 1. Testes failing-first → passing

Os 13 testes registrados em `AUDIT_ETAPA2.md` e no apêndice daquele arquivo passaram após as correções da ETAPA 3. Os comandos individuais foram executados com `RUSTFLAGS="-C linker=/usr/bin/gcc"` e o log consolidado está em `/tmp/opencode/loi-audit-gate-after.log`.

| Teste | Resultado após correção |
|---|---|
| `checker::global_code_values_are_rejected` | ok |
| `checker::invalid_printf_length_modifiers_are_rejected` | ok |
| `checker::nested_generic_shape_literals_in_slices_are_checked` | ok |
| `guard::guard_accepts_a_generic_shape_type_after_monomorphization` | ok |
| `lexer::rejects_malformed_radix_literals` | ok |
| `lexer::string_escape_errors_point_at_the_backslash` | ok |
| `completion::completion_offers_every_canonical_keyword` | ok |
| `modules::guard_with_qualified_shape_resolves_its_release_flow` | ok |
| `order::len_keeps_logical_short_circuiting` | ok |
| `order::len_preserves_left_to_right_operand_order` | ok |
| `semantic_tokens::comptime_keywords_are_semantic_keywords` | ok |
| `diagnostic_render::sourced_rendering_summarizes_warnings_as_warnings` | ok |
| `llvm::llvm_matches_c11_arithmetic_traps` | ok |

`llvm_matches_c11_arithmetic_traps` confirma que C11 e LLVM imprimem `integer overflow` e saem com status 1 para o mesmo programa de `i8` overflow/soma/divisão/shift.

### Cobertura adicional de ownership (sem mudança de compilador)

`tests/ownership_edges.rs` cobre quatro contratos já documentados que não tinham teste dedicado: `give` de campo não inicializado, `give` antes de `continue` em `if` dentro de loop, junta conservadora de `if` sem `else` e fuga de `view` de dentro de um loop. Esses testes validam o comportamento existente; não foram usados para justificar mudança de semântica.

### Regressões adicionais (AUD-014…AUD-017)

| Teste | Resultado após correção |
|---|---|
| `checker::mark_variants_are_accepted_as_read_arguments` | ok |
| `checker::give_before_break_inside_an_if_poisons_the_value_after_the_loop` | ok |
| `checker::extern_read_argument_rejects_a_partially_moved_composite` | ok |
| `codegen::large_negative_i64_literal_keeps_its_value_in_c11` | ok — C gerado sem `loi_neg_i32` para i64::MIN; stdout `-9223372036854775808` |
| `llvm::llvm_matches_c11_for_large_negative_i64_literal` | ok — C11 e LLVM com mesmo stdout e exit 0 |

## 2. Suíte completa intermediária

Comando: `cargo test --all-targets` (log em `/tmp/opencode/loi-audit-gate-after.log`).

Resultado: **588 passed, 0 failed, 0 ignored, 0 measured, 0 filtered**, 35 suítes. Isso inclui os 13 testes de regressão acima e mantém os 575 testes do baseline. `sync_examples --check` e `git diff --check` também saíram com exit 0 nessa execução; `fmt` e `clippy` falharam apenas por formatação/imports introduzidos durante a auditoria e foram corrigidos antes do gate final (`src/format.rs` clippy `collapsible_if`; imports não usados em `tests/abi_golden.rs`, `tests/guard.rs`, `tests/codegen.rs` e `tests/comparison.rs`).

## 3. C11 estrito

Comando exato:

```sh
cargo build --quiet
work=$(mktemp -d)
./target/debug/loi gen examples/flow_syntax.loi -o "$work/flow_syntax.c"
cc -std=c11 -pedantic-errors -Wall -Wextra -fsyntax-only "$work/flow_syntax.c"
```

Resultado: **exit 0**. Houve dois warnings esperados e não suprimidos: `unused variable 'mass'` no Loi e `variable 'loi_v_mass' set but not used` no C gerado. Nenhum `-Werror` foi adicionado e nenhum warning foi escondido.

## 4. Differential e ordem

- `tests/order.rs::repro_prints_5007_with_gcc_and_clang`: GCC 15.2.0 e Clang 21.1.8 imprimem `5007` — ok.
- `tests/order.rs::binary_operands_evaluate_left_to_right`, `call_arguments_evaluate_left_to_right`, `printf_arguments_evaluate_left_to_right`, `logical_operators_keep_short_circuiting`, `len_keeps_logical_short_circuiting` e `len_preserves_left_to_right_operand_order`: ok nos dois compiladores.
- `tests/order.rs::three_hundred_generated_programs_match_the_left_to_right_oracle`: **320/320 casos gerados** comparados com o oráculo Rust — ok. O teste levou aproximadamente 315 s no baseline e permaneceu como o mais longo da suíte.
- `tests/llvm.rs::llvm_matches_c11_arithmetic_traps`: ok, com paridade de mensagem e status entre C11 e LLVM.
- Os demais testes de differential C11/LLVM da suíte (`llvm_backend_builds_and_runs_with_the_same_output_as_c11`, `llvm_example_matches_the_c11_backend` e `llvm_backend_matches_c11_for_slices_protect_and_alloc`): ok.

## 5. Gate final

O gate final usa `./tools/ci.sh`. O primeiro log `/tmp/opencode/loi-audit-final-gate.log` terminou com `gate_exit=0`; a reexecução final após `tests/ownership_edges.rs` em `/tmp/opencode/loi-audit-final-gate2.log` terminou com `gate_exit=0`, `selected CI steps passed` e **612 passed, 0 failed, 0 ignored** em 40 suítes. Resultado consolidado em `AUDIT_ETAPA6.md`, `AUDIT_FINAL.md` e no topo de `PROGRESS.md`.

## 6. Sanitizers

Comando: `./tools/check_sanitizers.sh` (log em `/tmp/opencode/loi-sanitizers.log`, exit 0 com `LOI_REQUIRE_ASAN=0`).

| Verificação | Resultado |
|---|---|
| UBSan GCC em `flow_syntax`, `marks`, `choices`, `language_basics` | 4/4 compilaram e executaram com exit 0 e stderr vazio |
| UBSan Clang nos mesmos quatro programas | 4/4 compilaram e executaram com exit 0 e stderr vazio |
| ASan+UBSan Clang nos mesmos quatro programas | 4/4 compilaram e executaram com exit 0 e stderr vazio |
| ASan+UBSan GCC nos mesmos quatro programas | compilou, mas **não executou** neste PRoot: `heap size 40000002000 exceeds max user virtual address 7fffffffff` |

O script instrumentou também um overflow deliberado e confirmou que UBSan (GCC e Clang) e ASan Clang detectam a falha. Portanto, **ASan é considerado validado apenas com Clang neste host**; GCC ASan é explicitamente reportado como não validado. O CI pode exigir ASan com `LOI_REQUIRE_ASAN=1`.

## 7. O que não foi declarado como validado

- Windows/MSYS e macOS: não executados nesta auditoria; a portabilidade dos fixtures foi corrigida, mas não há evidência de execução.
- Mutation testing: não executado; `development/MUTATION.md` documenta o procedimento manual e as mutações que devem ser effective.
- Fuzz extensivo: não executado; apenas os targets determinísticos de smoke e propriedade da ETAPA 5.
- LLVM pleno: o subset documentado continua build-only e de arquivo único; a paridade foi provada para os casos de aritmética checada e slices/protect/alloc, não para toda a linguagem.
