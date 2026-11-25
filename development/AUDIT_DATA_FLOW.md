# Auditoria de Data Flow — reabertura da FASE 2

Data: 2026-11-24. Escopo: infraestrutura de Data Flow já existente no checker
(`FlowSummary`, `DataPath`, fixpoint, resumos de view e efeitos), sem mudança de
linguagem. Nenhuma sintaxe, direção, regra de ownership, ABI, backend ou nova
dependência foi introduzida. Este relatório complementa `AUDIT_FINAL.md`, que
cobre a auditoria anterior do compilador.

## 1. Diagnóstico entregue antes de qualquer mudança

A auditoria foi feita somente leitura; o diagnóstico foi apresentado antes da
primeira edição. Achados reais, com os pontos de código então observados:

| ID | Achado | Evidência no código antes da correção | Classificação na época |
|---|---|---|---|
| DF-01 | `PRODUCE` existia como variante, mas nunca era construído; `result.paths` nunca era preenchido. | `src/flow.rs`: `ParamEffects::record` não tratava `Produce`; `src/checker.rs`: só `result.produces = true`. | BUG CONFIRMADO |
| DF-02 | `unknown_effects` era campo morto; `extern flow` com corpo vazio era silencioso e chamada sem resumo era tratada como neutra. | `src/checker.rs`: campo lido apenas em acessores; nenhum `= true`. | BUG CONFIRMADO |
| DF-03 | `ESCAPE`/loans achatavam a origem para o parâmetro inteiro e perdiam o slot do holder. | `ViewScanState` usava `HashSet<usize>`/`HashMap<String, HashSet<usize>>`; `Borrow` gravava `DataPath::new(param)`. | BUG CONFIRMADO |
| DF-04 | Duas álgebras de caminho e travessias duplicadas; a composição de caminhos relativos concatenava prefixo já separado com componente cru. | `join_paths(".inner", "leaf")` → `.innerleaf`; usado em consumo, leitura, escrita, produção e escape. | BUG CONFIRMADO (agravado na reabertura) |
| DF-05 | Fixpoint sem limite/widening e sem testes de recursão. | `analyze_flow_summaries` itera até não haver mudança. | LIMITAÇÃO CONHECIDA |
| DF-06 | Documentação contraditória: `DATA_FLOW_DESIGN.md` prometia `produced_paths`, `unknown_effects` e widening; `DECISIONS.md`/`PROGRESS.md` classificavam `PRODUCE` como implementado; `PROGRESS.md` negava retorno de view, contrariando `SINTAXE.md` e `tests/views.rs`. | Arquivos canônicos citados. | DOCUMENTADO / INCONSISTÊNCIA |

Plano aprovado pelo usuário: iniciar a FASE 2 com failing-first, uma mudança por
vez, nas frentes `PRODUCE`, `ESCAPE` e `unknown_effects`, seguidas de regressão e
documentação.

## 2. Classificação final

| Item | Classificação | Onde |
|---|---|---|
| `PRODUCE` com caminho e propagação interprocedural | **IMPLEMENTADO** | `FlowSummary::record_result_effect`, `ResultEffects::paths`, fixpoint em `src/checker.rs` |
| `ESCAPE` com origem relativa e slot do holder | **IMPLEMENTADO** | `ViewSource`, `ViewOutLoan`, `view_returns_sources`, `view_out_sources`, `escaped_loans` |
| `unknown_effects` conservador | **IMPLEMENTADO** | `extern flow` e chamada sem resumo não intrínseco em `analyze_flow_summaries` |
| Álgebra de sobreposição unificada | **IMPLEMENTADO** | `DataPath::may_overlap` usada por todos os efeitos |
| Composição de caminhos aninhados | **IMPLEMENTADO** | `append_relative_path` substitui `join_paths` em sete pontos do checker |
| Integração total com ownership/regiões | **IMPLEMENTADO PARCIALMENTE** | consumo/leitura usam o resumo; `PRODUCE`/`ESCAPE` ainda não decidem sozinhos todas as regiões; `unknown_effects` não gera diagnóstico novo |
| Contratos e decisões publicados | **DOCUMENTADO** | `CONTRACTS.md`, `DECISIONS.md`, `DATA_FLOW_DESIGN.md`, `PROGRESS.md` |
| Recursão com limite/widening | **PROPOSTO** | `DATA_FLOW_DESIGN.md` §7; sem implementação nesta fase |
| `split`, `join`, `|>`, State Flow declarativo | **NÃO SUPORTADO** | nenhuma sintaxe nova aceita |
| Taint de `protect` via `<->`, índice dinâmico, lacunas LLVM | **LIMITAÇÃO CONHECIDA** | `CONTRACT-PROT-002`, `DATA_FLOW_DESIGN.md` §3 e §16 |

## 3. Failing-first e correções mínimas

### 3.1 `PRODUCE`

Testes escritos antes da implementação:

- `produced_paths_record_the_destination_written_by_the_flow`: falhou com
  `expected out.left in produced paths, got []`; depois de gravar somente o
  ponto de retorno, falhou com
  `expected dst.left, got DataPathSet { paths: {} }`.
- `produced_paths_propagate_through_calls`: exigiu que `make_left(-> dst)`
  propagasse `dst.left`, não apenas `produces = true`.

Correção: `FlowEffect::Produce` roteado para `FlowSummary::record_result_effect`;
`produced_places` no `FunctionScan` registra atribuições/`alloc` em `->` e o
retorno `=>`; a propagação re-ancora o caminho do callee no argumento do
chamador. `ParamEffects::record` recusa `Produce` para impedir volta ao mapa de
parâmetros.

### 3.2 `ESCAPE`

- `escaped_source_keeps_its_relative_path` falhou primeiro com
  `left: [""]`, `right: ["left"]`.
- `escaped_loan_keeps_the_holder_slot` cobriu `out.a`, não apenas `out`.

Correção: `ViewSource { index, path }` e `ViewOutLoan { slot, source }` no scan;
`Borrow::source` e `Borrow::holder` passam a carregar caminhos relativos; novos
acessores preservam a tupla `(índice, caminho)` e o slot. A asserção antiga que
esperava escape `whole` foi atualizada porque documentava a limitação, não a
regra da linguagem.

### 3.3 `unknown_effects`

- `extern_flow_marks_unknown_effects_and_propagates_them`: falhou com
  `assertion failed: foreign.unknown_effects`.
- `call_without_summary_marks_unknown_effects`: falhou com
  `assertion failed: summary.unknown_effects`.

Correção: `extern flow` marca `unknown_effects` e grava `Read`/`Init`/`Write`
inteiros por direção; chamada sem resumo e sem nome intrínseco conhecido faz o
mesmo no chamador para argumentos `->`/`<->`; callee com flag propaga o flag.

### 3.4 Composição aninhada (achado durante a reabertura)

Quatro testes foram adicionados e executados antes da correção final; log:
`/tmp/opencode/loi-fase2-nested-final-red.log`.

- `escaped_source_keeps_nested_relative_paths`:
  `left: ["innerleaf"]`, `right: ["inner.leaf"]`.
- `produced_paths_keep_the_call_site_nested_path`:
  `expected dst.nested.left, got DataPathSet { paths: {DataPath { root: "dst", segments: [Field("nestedleft")] }}}`.
- `nested_consumption_is_applied_at_the_call_site`:
  `expected use of moved value, got []`.
- `nested_read_summary_detects_a_moved_field`:
  `expected use of moved value, got []`.

Correção: `append_relative_path` anexa um componente por vez (`.campo`, `[n]`).
A antiga `join_paths` esperava que o relativo já trouxesse separador, mas os
resumos expõem `relative_string()` sem separador. A função foi removida e todas
as composições de caminho do checker passaram a usar a mesma regra.

Verde: `/tmp/opencode/loi-fase2-nested-green.log` — 4/4; suíte interna completa
`checker::flow_integration_tests` — 13/13 em
`/tmp/opencode/loi-fase2-flow-integration-final.log`.

## 4. Arquivos alterados

- `src/flow.rs`: roteamento de `PRODUCE`, acessores de escape/loan e comentários
  de contrato.
- `src/checker.rs`: scan de produção/escape, `unknown_effects`, propagação e
  composição de caminhos; 13 testes internos de integração.
- `development/PROGRESS.md`, `DECISIONS.md`, `CONTRACTS.md`,
  `DATA_FLOW_DESIGN.md`, `AUDIT_FINAL.md` e este relatório.

## 5. Gates e não-evidências

Gate final executado após as correções e a limpeza dos comentários que ainda
tratavam `join_paths` como função corrente:

```sh
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
./tools/ci.sh
```

Resultado: `exit=0` e `selected CI steps passed`; log
`/tmp/opencode/loi-fase2-final-ci.log`. O step `tests` reportou **636 passed,
0 failed, 0 ignored, 41 suítes** e `no skipped tests detected`; o agregado dos
steps foi **950 passed, 0 failed, 0 ignored, 54 suítes**. Passaram fmt, clippy,
goldens, exemplos, sync, diff-check, C11 estrito, UBSan GCC/Clang e ASan+UBSan
Clang. Os comandos exatos e os resultados por step estão em
`development/PROGRESS.md`.

Este relatório não afirma MSRV local 1.88 validado nem execução de GCC ASan
neste PRoot; o CI instala 1.88 com `LOI_REQUIRE_MSRV=1`, e GCC ASan falha com
`heap size 40000002000 exceeds max user virtual address 7fffffffff`. UBSan
GCC/Clang e ASan+UBSan Clang permanecem cobertos pelo gate versionado.

Nenhum golden foi regenerado, nenhum teste foi marcado como ignorado, nenhuma
verificação foi removida ou relaxada, e nenhum warning foi escondido. C11
continua sendo o backend de produção; LLVM continua rejeitando explicitamente o
que não suporta.

## 6. FASE 3 — `DataPath` como representação do checker (2026-11-25)

Diagnóstico que abriu a etapa: os resumos já eram `DataPath`, mas o fixpoint
convertia caminhos para `String` e os reconstruía; o estado transiente mantinha
uma segunda álgebra (`PathComponent`/`parse_path`/`render_path`) com regras
próximas, porém separadas, de `DataPath::may_overlap`. `ViewSource.path` e
`ViewOutLoan.slot` também eram strings, e `call_sources` usava accessores que
devolviam pares `(índice, String)`.

Failing-first: três testes adicionados em `tests/data_path.rs` exigiam
`moves_mut`, `reads_mut`, `move_paths`, `read_paths_typed`, `append_relative`,
`view_returns_data_paths` e `view_out_data_paths`. Antes da implementação o
comando terminou com `exit=101` e 9 × E0599; log
`/tmp/opencode/loi-fase3-typed-red.log`. Depois dos accessores tipados, a suíte
ficou em **11 passed, 0 failed**, `/tmp/opencode/loi-fase3-typed-green.log`.

Correção aplicada em etapas pequenas, com `cargo check --all-targets` após cada
uma (logs `/tmp/opencode/loi-fase3-{viewsource,moved,taints,loans,scans}-check.log`,
todos `exit=0`):

1. `ViewSource { index, path: DataPath }` e `ViewOutLoan { slot: DataPath, … }`
   passaram a compor com `append_relative` e a re-ancorar com `with_root`;
   `call_sources` passou a usar `view_returns_data_paths`/
   `view_out_data_paths`.
2. `VarInfo.moved`, `ControlState.moved` e `RegionTaint.path` passaram a usar
   `DataPath`; `moved_span`, `path_is_moved`, `paths_overlap` e
   `path_is_ancestor` delegam a `may_overlap`/`is_ancestor_of`. Os helpers
   `PathComponent`, `parse_path`, `render_path` e `components_match_prefix`
   foram removidos.
3. `ViewLoan.path/slot` e `FunctionScan` (`moved_places`, `written_places`,
   `initialized_places`, `produced_places`, `direct_reads`, argumentos de
   `CallSiteScan`) passaram a carregar `DataPath`; `index_data_path` preserva
   `IndexKey::Const`/`Dynamic` e a regra conservadora atual.
4. `move_consumes_place`/`type_at_path` percorrem `PathSegment`; o fixpoint
   re-ancora com `with_root` em vez de reconstruir a partir de string. Os
   accessores antigos que devolvem `String` permanecem como wrappers.

Regressão: `checker::flow_integration_tests` = **13 passed, 0 failed** em
`/tmp/opencode/loi-fase3-flow-integration.log`; suítes focadas `checker`,
`data_path`, `ownership_edges` e `views` = **196 passed, 0 failed** em
`/tmp/opencode/loi-fase3-focused-suites.log`. `cargo fmt --check` e
`cargo clippy --all-targets -- -D warnings` terminaram em `exit=0`
(`/tmp/opencode/loi-fase3-fmt.log`, `/tmp/opencode/loi-fase3-clippy.log`).

Gate final versionado:

```sh
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
./tools/ci.sh
```

Resultado: `exit=0`, `selected CI steps passed`, log
`/tmp/opencode/loi-fase3-ci.log`. Step `tests`: **639 passed, 0 failed,
0 ignored, 41 suítes**; agregado de todos os steps: **953 passed, 0 failed,
0 ignored, 54 suítes**; `no skipped tests detected`. Passaram goldens,
exemplos, sync, diff-check, C11 estrito, UBSan GCC/Clang e ASan+UBSan Clang.
MSRV local segue sem validação (Rust 1.88 não instalado) e GCC ASan continua
inviável neste PRoot; nenhuma dessas limitações é nova.

Classificação da FASE 3: **IMPLEMENTADO** para accessores tipados, composição
relativa e estado transiente migrado; **IMPLEMENTADO PARCIALMENTE** para a
representação única (restam `LvalueInfo.path`, `place_path` e mensagens como
fronteiras de string); **DOCUMENTADO** em `PROGRESS.md`, `DECISIONS.md`,
`CONTRACTS.md` e `DATA_FLOW_DESIGN.md`; **PROPOSTO** para `OwnershipState` e
widener de recursão; **NÃO SUPPORTADO** (nenhuma sintaxe nova); **LIMITAÇÃO
CONHECIDA** para `unknown_effects` sem diagnóstico próprio, `CONTRACT-PROT-002`
e as lacunas LLVM preexistentes. Nenhuma sintaxe, mensagem de diagnóstico,
regra de ownership, ABI, C11/LLVM lowering ou dependência mudou; nenhum golden
foi regenerado e nenhuma verificação foi removida ou relaxada. Nada foi
commitado, enviado para revisão ou mesclado.
