# Auditoria — ETAPA 6 (documentação e fechamento)

Data: 2026-11-24.

## 1. Contradições documentais aplicadas

Todas as contradições foram registradas em `AUDIT_ETAPA1.md` **antes** de qualquer escolha. Nenhuma delas exigiu nova sintaxe ou semântica.

| ID | Aplicação |
|---|---|
| DOC-01 | Quatro exemplos em `LOI_SPEC.md` passaram de `Vec2[ x: ..., y: ... ]` para `Vec2 { x: ..., y: ... }`. |
| DOC-02 | `development/AGENTS.md` não proíbe mais `Name { ... }`; continua proibindo `interface`, `enum`, `union`, `fun`, `let`, `var`, `visible`, `hidden`, `out`, `inout`, `move`, `copy`, `return` e `[]T`. |
| DOC-03 | `format(<- [u8], ...) => [u8]` acrescentado às duas listas de intrínsecos em `LOI_SPEC.md`. |
| DOC-04 | `callback` acrescentado ao conjunto `Copy` em `LOI_SPEC.md`, alinhado a `SINTAXE.md` e à implementação; `is_copy_type` não foi alterado. |
| DOC-05 | Limitações stale sobre `alloc`/entry points e igualdade não estrutural removidas de `PROGRESS.md`. |
| DOC-06 | Decisões de 2026-11-08 sobre braces marcadas como superadas pela decisão de 2026-11-22, preservando o registro histórico. |
| DOC-07 | `fix_problems.md` identificado como relatório histórico de 2026-11-08, não estado atual. |
| DOC-08 | `PROGRESS.md` ganhou um resumo cronológico no topo; o histórico detalhado permanece abaixo para auditoria. |

## 2. Decisões registradas

- **OPEN-01 resolvido**: o backend LLVM passa a trapar com a mesma mensagem/posição do C11 (`sadd/ssub/smul.with.overflow`, zero, `MIN/-1`, unário `-` e shifts), sem rejeitar operações documentadas. Registro em `DECISIONS.md` (2026-11-24).
- **CI versionado**: `tools/ci.sh` + `.github/workflows/ci.yml` com steps nomeados, inventário de ferramentas, fail-on-skip controlado, proteção de goldens e MSRV exigido no CI. Registro em `DECISIONS.md` (2026-11-24).
- **OPEN-07** permanece aberto: nomes manglados de release para recursos genéticos são a convenção atual; mudá-los exigiria decisão de semântica/mangling.

## 3. Matriz de contratos

A matriz em `AUDIT_ETAPA1.md` §6 foi atualizada com o estado final de CTR-01…CTR-13 e ampliada com CTR-14 (literais/escapes), CTR-15 (completação/semantic tokens), CTR-16 (ABI golden) e CTR-17 (gate/CI). `development/CONTRACTS.md` detalha 23 contratos com ID estável, pré-condição, permitido/proibido, testes positivos/negativos/runtime e status por backend. Nenhum contrato foi enfraquecido para fazer testes passarem.

## 4. Gates e evidências

### Baseline (antes de qualquer mudança)

```sh
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && python3 tools/sync_examples.py --check && git diff --check
```

Resultado: exit 0; **575 passed, 0 failed, 0 ignored**, 33 suítes; log em `/tmp/opencode/loi-audit-baseline.log`.

### Failing-first

`AUDIT_ETAPA2.md` registra 13 testes falhando antes da correção (11 testes principais mais `semantic_tokens` e `diagnostic_render`) e o apêndice 2 registra mais 5 testes. Total: 18 testes de regressão failing-first. Os 5 testes do apêndice foram reexecutados em um worktree isolado no `HEAD` original; log `/tmp/opencode/loi-baseline-red-evidence.log`.

### Pós-correção

- `cargo test --all-targets`: **588 passed, 0 failed, 0 ignored**, 35 suítes (log `/tmp/opencode/loi-audit-gate-after.log`).
- `cc -std=c11 -pedantic-errors -Wall -Wextra -fsyntax-only` no C gerado de `examples/flow_syntax.loi`: exit 0, sem supressão de warnings.
- GCC 15.2.0 e Clang 21.1.8: todos os testes de ordem e os 320 casos gerados passaram.
- C11 vs LLVM: `llvm_matches_c11_arithmetic_traps` passou com mensagem e status idênticos.
- Gate final `./tools/ci.sh`: primeiro resultado em `/tmp/opencode/loi-audit-final-gate.log` (`gate_exit=0`); reexecução final após `tests/ownership_edges.rs` em `/tmp/opencode/loi-audit-final-gate2.log` — **`gate_exit=0`**, `selected CI steps passed`, **612 passed, 0 failed, 0 ignored** em 40 suítes no step `tests`. Todos os steps passaram; o step `msrv` local emitiu `SKIP: Rust 1.88 toolchain is not installed`, e o CI impõe `LOI_REQUIRE_MSRV=1`.

## 5. Estado final dos bugs

AUD-001…AUD-013: corrigidos ou reclassificados conforme `AUDIT_ETAPA3.md`. AUD-013 era apenas mensagem pobre de colisão de nome; o diagnóstico continua sendo emitido e foi mantido como está para não alterar contrato.

AUD-014…AUD-017: confirmados na auditoria aprofundada com testes failing-first, corrigidos e validados individualmente. AUD-014 (mark variant como `<-` argumento), AUD-015 (literal negativo grande nos dois backends), AUD-016 (`give` antes de `break`/`continue` em loop) e AUD-017 (`extern flow` com composto parcialmente movido) são detalhados em `AUDIT_ETAPA1.md`, `AUDIT_ETAPA2.md`, `AUDIT_ETAPA3.md` e `AUDIT_ETAPA4.md`. OPEN-01 e OPEN-04 foram resolvidos; OPEN-02, OPEN-03 (parcialmente), OPEN-05, OPEN-06 e OPEN-07 permanecem registrados.

## 6. Limitações e não-evidências

- ASan **com Clang** foi executado e validado; ASan com GCC falha neste PRoot e não é afirmado. Fuzz extensivo, mutation testing automatizado, Windows/MSYS e validação LLVM plena não foram executados; nenhuma afirmação indevida foi adicionada.
- INF-006 (contratos de `xs.len`, mark/choice como argumento e literais negativos grandes), INF-007 (ASan), INF-010 parcial (assets LSP/vendor) e OPEN-02…OPEN-06 permanecem como lacunas explícitas.
- Nenhuma sintaxe ou semântica nova foi introduzida; nenhuma direção de parâmetro, ownership, `give`, `clone`, `protect`, `guard`, `view`, `comptime` ou módulos foi alterada.

## 7. Arquivos de auditoria

- `development/AUDIT_ETAPA1.md` — descobertas, baseline, contradições, matriz.
- `development/AUDIT_ETAPA2.md` — evidências failing-first.
- `development/AUDIT_ETAPA3.md` — correções mínimas.
- `development/AUDIT_ETAPA4.md` — regressão e differential.
- `development/AUDIT_ETAPA5.md` — infraestrutura.
- `development/AUDIT_ETAPA6.md` — este documento.
- `development/CONTRACTS.md` — matriz detalhada de contratos (23 contratos, 4 resolvidos nesta auditoria).
- `development/MUTATION.md` — procedimento manual de mutation smoke.
