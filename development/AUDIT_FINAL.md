# Auditoria completa do compilador Loi — relatório final

Data: 2026-11-24. Host: Linux/PRoot, `rustc`/`cargo` 1.100.0, GCC 15.2.0,
Clang 21.1.8, aarch64. Este relatório consolida as ETAPAS 1–6 descritas em
`AUDIT_ETAPA1.md` … `AUDIT_ETAPA6.md`. Nenhuma mudança foi commitada, enviada
para revisão ou mesclada; a árvore de trabalho está suja, como esperado para
uma auditoria em andamento.

## 1. Estado inicial (antes de qualquer alteração)

Gate de baseline executado com `RUSTFLAGS="-C linker=/usr/bin/gcc"`:

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check
```

Resultado: **575 testes passaram, 0 falharam, 0 ignorados, 33 suítes**, todos
os cinco comandos com exit 0. Log: `/tmp/opencode/loi-audit-baseline.log`.
Não havia CI versionado, os helpers de teste continham `/tmp/opencode` fixo,
69 skips silenciosos e nenhum golden de ABI.

## 2. Bugs confirmados

Dezessete bugs reais foram confirmados com reprodução mínima:

| ID | Prioridade | Área | Resumo |
|---|---|---|---|
| AUD-001 | P1 | expansão/codegen | `quote` em global passava no `check` e fazia panic no `gen` |
| AUD-002 | P1 | módulos/guard | tipo qualificado perdia `_release` na convenção de release |
| AUD-003 | P1 | parser | backtracking de `>>` corrompia slice com tipo genérico aninhado |
| AUD-004 | P1 | format/codegen | `%llf`, `%Ls`, `%hf` aceitos e impressos como `0.000000` |
| AUD-005 | P1 | codegen | `len()` em RHS quebrava curto-circuito e ordem left-to-right |
| AUD-006 | P1 | LLVM | overflow assinado trapava no C11, mas envelopava no LLVM |
| AUD-007 | P2 | monomorfize | tipo explícito de `guard` não era monomorfizado |
| AUD-008 | P2 | lexer | `0x`/`0b2` não geravam erro léxico |
| AUD-009 | P3 | lexer/diagnóstico | escape inválido apontava para a abertura da string |
| AUD-010 | P3 | completação | faltavam `clone`, `quote`, `splice`, `unquote` |
| AUD-011 | P3 | LSP | `quote`/`unquote`/`splice` não eram keywords em `semanticTokens` |
| AUD-012 | P3 | diagnostic API | `render_sourced` chamava warnings de “errors” |
| AUD-013 | P3 | monomorphize/checker | colisão de nome gerado era reportada com mensagem enganosa |
| AUD-014 | P2 | checker | `Color.Red` como `<-` argumento virava `unknown variable 'Color'` |
| AUD-015 | P1 | codegen/llvm | `-9223372036854775808` virava `loi_neg_i32(...)` e imprimia `0` |
| AUD-016 | P1 | checker | `give` antes de `break`/`continue` em `if` escapava do loop |
| AUD-017 | P1 | checker | `extern flow` aceitava `<-` de composto parcialmente movido |

Detalhes, trechos, comportamento atual/esperado, impacto e proposta de correção
estão em `AUDIT_ETAPA1.md` §4. Os testes failing-first estão em
`AUDIT_ETAPA2.md`. Os cinco testes do apêndice 2 foram reexecutados em um worktree isolado no `HEAD` original para provar o estado vermelho sem depender da árvore modificada (`/tmp/opencode/loi-baseline-red-evidence.log`).

## 3. Bugs corrigidos

Correções mínimas aplicadas **depois** dos testes vermelhos, registradas em
`AUDIT_ETAPA3.md`:

- AUD-001: `validate_program` passa a validar `Item::Global`.
- AUD-002: `release_flow_name_for` preserva `_release` em nomes qualificados.
- AUD-003: `restore_speculation` restaura posição **e** tokens `>>` divididos.
- AUD-004: parser de `printf` aceita apenas comprimentos válidos.
- AUD-005: intrínsecos com efeito devolvem a atribuição como passo de ordem.
- AUD-006: LLVM usa intrinsics `with.overflow`, checa zero, `MIN/-1`, unário `-` e shifts.
- AUD-007: `StmtKind::Guard` escreve o tipo monomorfizado de volta no AST.
- AUD-008: `0x`/`0b` exigem dígitos válidos.
- AUD-009: erro de escape aponta para a barra invertida.
- AUD-010/011: keywords canônicas completas na completação e nos semantic tokens.
- AUD-012: `render_sourced` usa `summary()`.
- AUD-014: `check_read_argument` trata `Name.Variant` de mark/choice como valor.
- AUD-015: C11 e LLVM dobram `-<literal>` na largura esperada; `loi_neg_*` permanece para não-literais.
- AUD-016: `loop_exit_states` + `join_loop_exits` mesclam saídas antecipadas ao estado pós-loop.
- AUD-017: `extern flow` recebe o caminho raiz `{""}` como leitura de cada parâmetro `<-`.

AUD-013 foi reclassificado: o diagnóstico de colisão já era emitido; apenas a
mensagem é pobre. Não foi alterado para não mudar o contrato observável.

## 4. Bugs e lacunas em aberto

- **OPEN-02:** contrato de `xs.len` escrito além da capacidade (trap, unchecked ou type-level).
- **OPEN-05:** conversão slice ↔ `cstr` além de literais.
- **OPEN-06:** `<-` de composto: a spec descreve acesso por endereço; o C11 passa por valor.
- **OPEN-07:** release de recurso genérico usa o nome manglado da instância; mudar a convenção exige decisão de semântica/mangling.
- **Residual INF-010:** assets LSP e `vendor/*/skipped.txt` ainda sem golden de integridade.
- **AUD-013:** mensagem pobre de colisão de nome gerado (não é mais classificada como falha de diagnóstico).
- **Ambiente:** GCC ASan não roda neste PRoot; MSRV 1.88 não está instalada localmente (o CI instala e exige).

## 5. Contratos adicionados ou explicitados

`development/CONTRACTS.md` define 23 contratos com ID estável, fase,
pré-condição, permitido/proibido, testes positivos/negativos/de runtime e status
C11/LLVM. Nenhum contrato foi enfraquecido para fazer teste passar. Quatro
contratos anteriormente em aberto foram resolvidos nesta auditoria:

- mark/choice como argumento direto (AUD-014);
- literal negativo grande nos dois backends (AUD-015, OPEN-04);
- `give` antes de saída antecipada de loop (AUD-016);
- `<-` de composto parcialmente movido para extern flow (AUD-017).

OPEN-01 também foi resolvido: aritmética checada no LLVM segue o mesmo contrato
de mensagem e status do C11, registrado em `DECISIONS.md`.

## 6. Testes failing-first e regressões

18 novos testes de regressão foram escritos antes das correções e observados
vermelhos: 13 originais (`AUDIT_ETAPA2.md`) e 5 do aprofundamento
(`AUDIT_ETAPA2.md` §Apêndice 2). Todos passaram após as correções. Além disso,
`tests/ownership_edges.rs` adiciona 4 testes de cobertura de contratos já
documentados: `give` de não inicializado, `give` antes de `continue`, junta
conservadora de `if` sem `else` e fuga de `view` de dentro de um loop.

Nenhum teste foi removido, marcado como ignorado ou relaxado. Nenhum golden foi
atualizado em massa; a regeneração explícita com `LOI_BLESS=1` é rejeitada pelo
gate.

## 7. Infraestrutura e CI

- `.github/workflows/ci.yml` executa `./tools/ci.sh` em `push`/`pull_request`.
- `tools/ci.sh` tem steps nomeados: inventory, fmt, clippy, tests, goldens, msrv, examples, sync, diff-check, c11-strict, sanitizers, runtime, contracts, fuzz-smoke, differential.
- `tools/check_test_output.py` falha em qualquer skip, salvo `LOI_ALLOW_SKIPS=1`.
- `tools/check_goldens.py` falha com `LOI_BLESS=1` ou golden sujo.
- `tools/check_msrv.sh` falha com `LOI_REQUIRE_MSRV=1` se a toolchain declarada estiver ausente; o CI define essa variável.
- `tests/abi_golden.rs` compara a saída completa do C gerado com `tests/abi/scalars.expected.c`.
- Nenhuma dependência de crate foi adicionada.

## 8. Resultado C11

`cc -std=c11 -pedantic-errors -Wall -Wextra -fsyntax-only` passou para os
exemplos `flow_syntax`, `marks`, `choices` e `language_basics` no step
`c11-strict` do gate final. A verificação anterior de `flow_syntax.loi` também
exibiu dois warnings conhecidos (`unused variable 'mass'` no Loi e
`variable 'loi_v_mass' set but not used` no C gerado); nenhum foi suprimido e
`-Werror` não foi adicionado ao gate.

## 9. Resultado LLVM e paridade de backends

- `llvm_matches_c11_arithmetic_traps`: mesma mensagem `integer overflow` e status 1 em C11 e LLVM.
- `llvm_matches_c11_for_large_negative_i64_literal`: stdout `-9223372036854775808` e exit 0 nos dois backends.
- Differential GCC 15.2.0 / Clang 21.1.8: `repro_prints_5007_with_gcc_and_clang` e os testes de ordem passaram; 320/320 casos gerados casaram com o oráculo Rust.
- O subset LLVM continua build-only e de arquivo único; a paridade foi provada para os casos testados, não para toda a linguagem.

## 10. Sanitizers

`./tools/check_sanitizers.sh` terminou com exit 0 (com `LOI_REQUIRE_ASAN=0`):
UBSan GCC 8/8 e UBSan Clang 8/8 executaram sem stderr; ASan+UBSan Clang 4/4
executou sem stderr. ASan+UBSan GCC compilou, mas não executou neste PRoot:
`heap size 40000002000 exceeds max user virtual address 7fffffffff`. O script
também confirmou que as configurações executadas detectam um overflow
deliberado. Não se afirma que GCC ASan foi validado.

## 11. Fuzz, propriedades e mutation

- `tests/fuzz_smoke.rs`: 3/3 — lexer/parser/checker com entradas determinísticas.
- `tests/fuzz_stages.rs`: 4/4 — formatter, módulos/imports, expansão e const evaluator.
- `tests/property_smoke.rs`: 7/7 — determinismo, spans, idempotência de `fmt`, const-eval vs runtime, paths com espaço/Unicode e uso após `give`.
- `tests/ownership_edges.rs`: 4/4 — cobertura adicional de ownership/views.
- `development/MUTATION.md` documenta 10 mutações manuais effective. **Mutation testing não foi executado**; nenhuma ferramenta pesada foi adicionada.

## 12. Portabilidade

Todos os helpers de diretório temporário passaram a usar
`std::env::temp_dir()` com PID; os novos testes limpam explicitamente seus
diretórios. `tests/msrv.rs` exige `>= 1.88` e o README declara essa versão. O CI
roda em Linux com toolchain 1.88 instalada explicitamente. Windows/MSYS e
macOS **não foram executados**; apenas a portabilidade dos fixtures foi
corrigida.

## 13. Documentação e contradições

DOC-01…DOC-08 foram aplicadas e registradas antes das escolhas em
`AUDIT_ETAPA1.md`: sintaxe de braces em `LOI_SPEC.md`, `development/AGENTS.md`,
intrínseco `format`, conjunto `Copy` com `callback`, remoção de limitações
stale, marcação das decisões de 2026-11-08 como superadas, identificação de
`fix_problems.md` como relatório histórico e resumo cronológico no topo de
`PROGRESS.md`. `README.md`, `PROGRESS.md`, `DECISIONS.md`, `LOI_SPEC.md`,
`fix_problems.md` e os seis relatórios de etapa foram atualizados.

## 14. Arquivos alterados

- **Compilador:** `src/expand.rs`, `src/types.rs`, `src/monomorphize.rs`, `src/parser.rs`, `src/format.rs`, `src/lexer.rs`, `src/checker.rs`, `src/complete.rs`, `src/lsp_info.rs`, `src/diagnostic.rs`, `src/codegen.rs`, `src/llvm.rs`.
- **Testes existentes ajustados:** `tests/bindgen.rs`, `checker.rs`, `cli.rs`, `codegen.rs`, `comparison.rs`, `completion.rs`, `const_values.rs`, `guard.rs`, `handles.rs`, `header.rs`, `json.rs`, `json_builder.rs`, `jsonrpc.rs`, `lexer.rs`, `lib.rs`, `llvm.rs`, `mangling.rs`, `modules.rs`, `msrv.rs`, `order.rs`, `packages.rs`, `process.rs`, `stdio.rs`.
- **Novos testes/ferramentas:** `tests/abi/`, `tests/abi_golden.rs`, `tests/diagnostic_render.rs`, `tests/fuzz_smoke.rs`, `tests/fuzz_stages.rs`, `tests/property_smoke.rs`, `tests/ownership_edges.rs`, `tests/semantic_tokens.rs`, `tools/ci.sh`, `tools/check_goldens.py`, `tools/check_test_output.py`, `tools/check_sanitizers.sh`, `.github/workflows/ci.yml`.
- **Documentação:** `README.md`, `development/AGENTS.md`, `DECISIONS.md`, `LOI_SPEC.md`, `PROGRESS.md`, `fix_problems.md`, `AUDIT_ETAPA1.md` … `AUDIT_ETAPA6.md`, `CONTRACTS.md`, `MUTATION.md`, este `AUDIT_FINAL.md`.

## 15. Estado final e como reproduzir

Gate final: `./tools/ci.sh` → **exit 0 (`gate_exit=0`)**, **612 testes em
40 suítes**, 0 falhas, 0 ignorados no step `tests`. Log:
`/tmp/opencode/loi-audit-final-gate2.log`. Todos os steps nomeados passaram;
o step `msrv` local apenas emitiu `SKIP: Rust 1.88 toolchain is not installed`
sem falhar, e o mesmo step falha no CI porque `LOI_REQUIRE_MSRV=1` está definido.

```sh
./tools/ci.sh                  # gate completo
./tools/ci.sh tests            # apenas a suíte Rust
./tools/check_sanitizers.sh    # UBSan + ASan/Clang
```

Conclusão: a linguagem permanece a mesma — `shape`, `flow`, `<-`/`->`/`<->`/`=>`,
ownership único, `give`, `clone`, `protect`, `guard`, `view`, `open`/`sealed`,
`mark`, `choice`, generics/bounds, `comptime`, `quote`/`unquote`/`splice`,
módulos e FFI não foram alterados. As mudanças eliminam aceitações indevidas,
valores silenciosamente incorretos e lacunas de verificação. As decisões
pendentes listadas no item 4 exigem aprovação explícita antes de qualquer
mudança de sintaxe, semântica, ABI ou contrato.
