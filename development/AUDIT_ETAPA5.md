# Auditoria — ETAPA 5 (infraestrutura)

Data: 2026-11-24. Nenhuma dependência de crate foi adicionada. As mudanças abaixo tornam explícito o que antes era manual, silencioso ou dependente de um host específico.

| Item | Ação |
|---|---|
| INF-001 | `.github/workflows/ci.yml` executa `./tools/ci.sh` em `push`/`pull_request`, com `rustfmt`, `clippy`, `clang` e a toolchain 1.88 instalados explicitamente. `tools/ci.sh` é o único ponto de entrada e aceita um step opcional para reprodução local. |
| INF-002 | Todos os helpers de diretório temporário em `tests/*.rs` passaram a usar `std::env::temp_dir()`; os três paths literais restantes em `tests/codegen.rs` e `tests/handles.rs` também foram convertidos. |
| INF-003 | `tools/check_test_output.py` falha quando a saída do `cargo test --all-targets` contém skip, a menos que `LOI_ALLOW_SKIPS=1` seja definido. `tools/ci.sh` captura a saída, mostra o log completo e aplica o gate. |
| INF-004 | `tools/check_goldens.py` rejeita `LOI_BLESS=1` e qualquer `.expected`/`.golden` sujo no working tree. `tests/abi_golden.rs` usa o mesmo mecanismo explícito de `LOI_BLESS=1` para regeneração. |
| INF-005 | `tests/abi_golden.rs` compara a saída completa de `loi gen tests/abi/scalars.loi` com `tests/abi/scalars.expected.c`, cobrindo runtime, ABI de slice/callback e assinaturas C geradas. |
| INF-006 | `development/CONTRACTS.md` cria 23 contratos com ID estável, fase, pré-condição, permitido/proibido, testes e status C11/LLVM. Os pontos não definidos continuam marcados como OPEN. `-9223372036854775808` (OPEN-04) e mark/choice como argumento direto (OPEN-03) foram confirmados como AUD-015 e AUD-014 e corrigidos na ETAPA 3; `xs.len` além da capacidade permanece OPEN-02. |
| INF-007 | `tools/check_sanitizers.sh` executa UBSan com GCC e Clang (8/8 programas) e ASan+UBSan com Clang (4/4). GCC ASan falha no PRoot com erro de mapeamento de shadow memory e é explicitamente reportado como não validado; `LOI_REQUIRE_ASAN=1` torna isso fatal no CI. |
| INF-008 | `tests/msrv.rs` agora exige `rust-version >= 1.88` e a menção correspondente no README. `tools/check_msrv.sh` falha com `LOI_REQUIRE_MSRV=1` se `rustup` ou a toolchain declarada estiverem ausentes; o CI usa essa configuração. |
| INF-009 | `tests/fuzz_smoke.rs` (lexer/parser/checker), `tests/fuzz_stages.rs` (formatter, módulos/imports, expansão de templates, constant evaluator), `tests/property_smoke.rs` (determinismo, spans, idempotência de `fmt`, const-eval vs runtime, paths Unicode/com espaços, uso após `give`) e `tests/ownership_edges.rs` (init conservadora, `continue`, junta de `if`, fuga de `view`) adicionam 18 testes determinísticos sem dependências. Não é fuzz extensivo nem property-based completo, mas cobre as propriedades exigidas com geradores locais. |
| INF-010 | `tests/abi_golden.rs` cobre o C gerado; `development/MUTATION.md` documenta 10 mutações manuais effective, sem framework pesado. Assets LSP e `vendor/*/skipped.txt` permanecem sem golden e são registrados como lacuna residual. |
| INF-011 | Diretórios temporários agora usam `std::env::temp_dir()` com PID e são removidos explicitamente nos novos testes; os helpers antigos continuam a limpeza no início de cada execução. |

## Mudanças de comportamento do CI

- `./tools/ci.sh` executa, em ordem: inventário de ferramentas, `fmt`, `clippy`, `cargo test --all-targets`, integridade de goldens, MSRV, checagem de exemplos, `sync_examples --check`, `git diff --check`, C11 estrito, suítes de runtime, suítes de contrato, fuzz smoke e differential.
- O inventário imprime versões de `rustc`, `cargo`, `cc`, `clang`, `node`, `python3` e `ar`; ferramentas opcionais ausentes são visíveis, não disfarçadas.
- Um step pode ser executado isoladamente, por exemplo `./tools/ci.sh tests`, para reproduzir falhas sem repetir o gate inteiro.

## Verificação

O resultado exato do gate pós-correção foi consolidado na ETAPA 6 e no relatório final `AUDIT_FINAL.md`, junto com as evidências de differential C11/LLVM e dos 320 casos de ordem.
