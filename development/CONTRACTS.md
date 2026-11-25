# Matriz de contratos da Loi

Esta matriz é o índice verificável dos contratos da linguagem. Ela não cria regras novas: cada linha deriva de `SINTAXE.md` (superfície canônica), `development/LOI_SPEC.md` (regras) ou `development/DECISIONS.md` (decisões aceitas). Quando a regra não está definida, o campo de status é `OPEN` e a decisão permanece pendente — nunca é escolhida silenciosamente.

Legenda de status: **C11** = backend de produção; **LLVM** = subset opt-in; `n/a` = fora do contrato do backend; `parcial` = coberto apenas no subset documentado; `OPEN` = regra ainda não definida.

Formato de cada contrato: descrição, fase, pré-condição, permitido, proibido, testes positivos/negativos/runtime, C11, LLVM, referência.

---

## CONTRACT-LEX-001 — Tokens e spellings canônicos

- **Fase:** lexer.
- **Pré-condição:** bytes de um arquivo Loi com CRLF ou LF, com ou sem newline final.
- **Permitido:** setas `<-`, `->`, `<->`, `=>`; operadores compostos (`+=`, `..`, `>>`); identificadores qualificados `m::x`; keywords canônicas.
- **Proibido:** keyword antiga (`interface`, `fun`, `let`, `out`, `inout`, `move`, `copy`, `visible`, `hidden`), `[]T` como tipo, token vazio exceto EOF.
- **Testes:** `tests/lexer.rs` (`all_flow_keywords_are_recognized`, `qualified_names_lex_as_single_identifiers`, `longest_match_operators`, `comments_are_skipped`); unidades `src/lexer.rs`.
- **C11/LLVM:** n/a (frontend comum). **Spec:** `SINTAXE.md` §1–2; `LOI_SPEC.md` §4.1.

## CONTRACT-LEX-002 — Literais numéricos e escapes

- **Fase:** lexer / constant evaluation.
- **Pré-condição:** prefixo de base `0x`/`0b`, escape de string.
- **Permitido:** `0xFF`, `0b101`, `_` separador, escapes documentados (`\n`, `\t`, `\0`, `\"`, `\\`, `\xNN` ASCII).
- **Proibido:** `0x` ou `0b` sem dígito, `0b2`, `\q`, `\x` incompleto ou não-ASCII.
- **Testes:** `tests/lexer.rs::rejects_malformed_radix_literals`, `string_escape_errors_point_at_the_backslash`; unidades de `src/lexer.rs`.
- **C11/LLVM:** n/a. **Spec:** `SINTAXE.md` §2; `LOI_SPEC.md` §4.1.

## CONTRACT-PARSE-001 — Declarações e blocos

- **Fase:** parser.
- **Permitido:** `shape`, `flow`, `mark`, `choice`, `import`, `open`/`sealed`, assinaturas verticais, blocos aninhados, EOF sem newline.
- **Proibido:** declaração incompleta, bloco não fechado, token consumido duas vezes, recuperação em cascata.
- **Testes:** `tests/parser.rs`, `tests/pretty.rs`, `tests/fuzz_smoke.rs::parser_smoke_never_panics`.
- **C11/LLVM:** n/a. **Spec:** `SINTAXE.md` §3–5; `LOI_SPEC.md` §4.

## CONTRACT-PARSE-002 — Generics e retrocesso especulativo

- **Fase:** parser.
- **Permitido:** `Box<i32>`, `Vec<Vec<i32>>`, literais de slice com tipo explícito `[T: a, b]`.
- **Proibido:** aceitar `<` como tipo quando é comparação; corromper tokens `>>` após backtrack.
- **Testes:** `tests/checker.rs::nested_generic_shape_literals_in_slices_are_checked`, testes de parser/pretty existentes.
- **C11/LLVM:** n/a. **Spec:** `SINTAXE.md` §5; `LOI_SPEC.md` §5.1a.

## CONTRACT-PARSE-003 — Quote, unquote e splice

- **Fase:** parser + expansão.
- **Permitido:** `quote { ... }` dentro de `comptime flow`; `unquote(expr)` e `splice expr` apenas onde o template é expandido.
- **Proibido:** `Code` em global, runtime ou inicializador que chegue ao codegen.
- **Testes:** `tests/checker.rs::global_code_values_are_rejected`, `tests/parser.rs`, `tests/completion.rs`.
- **C11/LLVM:** `Code` removido antes dos backends. **Spec:** `SINTAXE.md` §5.5a; `LOI_SPEC.md` §5.1b.

## CONTRACT-OWN-001 — `give` transfere ownership

- **Fase:** checker.
- **Permitido:** `b <- give(a)` quando `a` estava disponível.
- **Proibido:** usar `a` depois; transferir valor global com vida estendida; `give` de caminho já consumido.
- **Testes:** `tests/checker.rs` (suíte de ownership), `tests/handles.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §4.2.

## CONTRACT-OWN-002 — `clone` duplica explicitamente

- **Fase:** checker + codegen.
- **Permitido:** `clone(x)` para valores que satisfazem `Clone`; deep clone de slices dentro de `protect`.
- **Proibido:** cópia implícita de composto com slice; `clone` de recurso linear sem contrato.
- **Testes:** `tests/checker.rs`, `tests/codegen.rs`, `tests/comparison.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §4.2, §4.9.

## CONTRACT-OWN-003 — Caminhos parciais e conservadorismo

- **Fase:** checker.
- **Permitido:** escrita de qualquer campo de uma saída `->` inicializa o valor inteiro; junção consumido+vivo mantém estado conservador; loops podem executar zero vezes.
- **Proibido:** uso de valor parcialmente movido em caminho que ainda depende do campo consumido; escape de partially-consumed value via `<-` com leitura sobre campo consumido.
- **Testes:** `tests/checker.rs` (path-sensitive consumption/read summaries), `tests/views.rs`, `src/checker.rs::flow_integration_tests::nested_consumption_is_applied_at_the_call_site`, `nested_read_summary_detects_a_moved_field`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `PROGRESS.md` (limitação documentada), `LOI_SPEC.md` §4.2. **Status:** cobertura existente ampliada para composição de caminhos aninhados (ver `AUDIT_DATA_FLOW.md`); moves, taints e loans do estado transiente usam `DataPath` e a mesma regra `may_overlap` (FASE 3, `DECISIONS.md` 2026-11-25).

## CONTRACT-DIR-001 — `<-` é leitura

- **Fase:** checker.
- **Permitido:** ler parâmetro; passar `<-` para flow que só lê.
- **Proibido:** escrever through `<-`; mover valor emprestado; aliases que permitam escrita.
- **Testes:** `tests/checker.rs::checks_parameter_directions_at_call_sites`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `SINTAXE.md` §3; `LOI_SPEC.md` §4.2.

## CONTRACT-DIR-002 — `->` inicializa saída do chamador

- **Fase:** checker.
- **Permitido:** inicializar exatamente uma vez; campos diferentes em caminhos disjuntos; `alloc(-> xs, T, n)` sob `protect`.
- **Proibido:** saída não inicializada em retorno; inicialização dupla em branches; escape de região.
- **Testes:** `tests/checker.rs`, `tests/codegen.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `SINTAXE.md` §3; `LOI_SPEC.md` §4.2, §4.9.

## CONTRACT-DIR-003 — `<->` é exclusivo

- **Fase:** checker.
- **Permitido:** `update(<-> world.physics, world.delta)` quando os caminhos são disjuntos.
- **Proibido:** `update(<-> world, world.physics)`; aliases indiretos sobrepostos; índice dinâmico que pode colidir com outro `<->`.
- **Testes:** `tests/checker.rs` (direções e aliases), `tests/views.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `SINTAXE.md` §3; `LOI_SPEC.md` §4.2.

## CONTRACT-DIR-004 — `=>` é resultado do flow

- **Fase:** checker + codegen.
- **Permitido:** um resultado por caminho de retorno; tipo compatível.
- **Proibido:** retornar valor com taint de `protect` que escapa; retornar slices locais; dois resultados em branches incompatíveis.
- **Testes:** `tests/checker.rs`, `tests/codegen.rs`, `tests/order.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `SINTAXE.md` §3; `LOI_SPEC.md` §4.2.

## CONTRACT-EFF-001 — Resumos de consumo e leitura

- **Fase:** checker / monomorphizer.
- **Permitido:** propagar caminhos consumidos e leituras por flow; fixpoint em ciclos; especialização genérica.
- **Proibido:** tratar callee externo desconhecido como leitura parcial; confundir índice dinâmico com caminho fixo.
- **Testes:** `tests/checker.rs`, `tests/data_path.rs` (álgebra tipada e accessores), `tests/comparison.rs`, `tests/guard.rs`, `src/checker.rs::flow_integration_tests::legacy_effects_are_exposed_by_one_flow_summary`, `one_visitor_produces_transitive_effects_for_independent_paths`, `nested_consumption_is_applied_at_the_call_site`, `nested_read_summary_detects_a_moved_field`, `produced_paths_record_the_destination_written_by_the_flow`, `produced_paths_propagate_through_calls`, `produced_paths_keep_the_call_site_nested_path`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §4.2, §5.1a. **Status:** IMPLEMENTADO — o fixpoint compõe `DataPath` tipado sem round-trip por string; strings restantes são fronteiras de leitura/mensagem (`DECISIONS.md` 2026-11-25).

## CONTRACT-EFF-002 — Efeitos desconhecidos são conservadores

- **Fase:** checker / análise de resumos.
- **Pré-condição:** `extern flow` com corpo vazio, ou chamada cujo callee não é intrínseco conhecido nem tem resumo.
- **Permitido:** marcar `unknown_effects` e registrar `READ`/`INIT`/`WRITE` no valor inteiro conforme a direção; propagar o flag por chamadas.
- **Proibido:** inferir "nenhum efeito" do silêncio; contar intrínseco conhecido como desconhecido; transformar isso em warning.
- **Testes:** `src/checker.rs::flow_integration_tests::extern_flow_marks_unknown_effects_and_propagates_them`, `call_without_summary_marks_unknown_effects`.
- **C11/LLVM:** n/a (frontend comum). **Spec:** `DATA_FLOW_DESIGN.md` §4; `DECISIONS.md` 2026-11-24.

## CONTRACT-VIEW-001 — View mantém a origem viva

- **Fase:** checker.
- **Permitido:** `view(xs, start, end)`, view de view, view em campo, loan de campos disjuntos.
- **Proibido:** mover/escrever/give a origem enquanto a view está viva; sobreposição parcial; retorno de view de `<-` sem summary.
- **Testes:** `tests/views.rs` (17), `tests/checker.rs`, `src/checker.rs::flow_integration_tests::escaped_source_keeps_its_relative_path`, `escaped_source_keeps_nested_relative_paths`, `escaped_loan_keeps_the_holder_slot`.
- **C11/LLVM:** ✓ / n/a. **Spec:** `LOI_SPEC.md` §4.6; `SINTAXE.md` §11. **Status:** IMPLEMENTADO — origem relativa e slot do holder preservados como `DataPath` (`view_returns_data_paths`, `view_out_data_paths`) no resumo e no estado transiente; typestate/split/pipeline são contratos separados e permanecem propostos em `DATA_FLOW_DESIGN.md`.

## CONTRACT-PROT-001 — `protect` delimita região

- **Fase:** checker + codegen.
- **Permitido:** `alloc`, slices e clones dentro da região; helpers com `protect` do chamador; regiões aninhadas.
- **Proibido:** escape por retorno, variável externa, campo ou `<-` com leitura total; saída antecipada sem restauração.
- **Testes:** `tests/checker.rs`, `tests/codegen.rs::compiled_protect_restores_the_arena`, `tests/llvm.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §4.9; `SINTAXE.md` §11.

## CONTRACT-PROT-002 — Taint através de `<->`

- **Fase:** checker.
- **Permitido:** propagar taint de `protect` para saídas que podem conter slice; conservadorismo em calls desconhecidos.
- **Proibido:** perder taint quando helper apenas muta armazenamento `<->` (limitação conhecida).
- **Testes:** `tests/checker.rs` (protect taint), `tests/codegen.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `PROGRESS.md` (limitação documentada). **Status:** OPEN — decidir se a aproximação é contrato ou bug.

## CONTRACT-GUARD-001 — Release exatamente uma vez

- **Fase:** checker + codegen.
- **Permitido:** normal, `=>`, `break`, `continue`, guards aninhados (LIFO), múltiplos escopos.
- **Proibido:** `release` explícito, mover o recurso, retornar recurso guardado.
- **Testes:** `tests/guard.rs`, `tests/handles.rs`, `tests/modules.rs`.
- **C11/LLVM:** ✓ / n/a. **Spec:** `SINTAXE.md` §11; `LOI_SPEC.md` §4.5.

## CONTRACT-GUARD-002 — Assinatura de release

- **Fase:** checker + monomorphizer.
- **Permitido:** `<Shape>_release` exato; nome manglado para instância genérica (`Box<i32>` → `box__i32_release`).
- **Proibido:** tipo qualificado resolvido para nome não manglado; assinatura incompatível.
- **Testes:** `tests/guard.rs::guard_accepts_a_generic_shape_type_after_monomorphization`, `tests/modules.rs::guard_with_qualified_shape_resolves_its_release_flow`.
- **C11/LLVM:** ✓ / n/a. **Spec:** `LOI_SPEC.md` §4.5; `DECISIONS.md`. **Status:** OPEN-07 (convenção de mangling para recursos genéricos).

## CONTRACT-TYPE-001 — Tipos, coercion e casts

- **Fase:** checker / const_eval / codegen.
- **Permitido:** literal coerced ao tipo esperado quando cabe; `as` truncante sem range check; literais não assinados padrão `i32`.
- **Proibido:** conversão implícita entre tipos nomeados; literal que não cabe; narrowing silencioso de literal negativo grande.
- **Testes:** `tests/checker.rs`, `tests/const_values.rs`, `tests/comparison.rs`.
- **C11/LLVM:** ✓ / ✓ (AUD-015). **Spec:** `SINTAXE.md` §6; `LOI_SPEC.md` §5. **Status:** OPEN-04 resolvido por paridade C11/LLVM (`AUDIT_ETAPA3.md`).

## CONTRACT-TYPE-002 — `Eq`/`Ord` estruturais e bounds

- **Fase:** monomorphizer / checker.
- **Permitido:** comparação estrutural de números, bool, marks, `cstr`, handles opacos, shapes/choices/slices cujas partes comparam; `Copy` inclui callbacks.
- **Proibido:** comparar callback com `==`; bound desconhecido tratado como `Clone`.
- **Testes:** `tests/comparison.rs`, `tests/checker.rs`, `tests/guard.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §5.1a; `SINTAXE.md` §6.

## CONTRACT-TYPE-003 — Mark e choice como valores

- **Fase:** parser + checker + codegen.
- **Permitido:** `Color.Red`, `Result.Ok(1)` como expressões e argumentos.
- **Proibido:** tratar construtor como lvalue ao passar diretamente como argumento.
- **Testes:** `tests/checker.rs::mark_variants_are_accepted_as_read_arguments`, `tests/codegen.rs`. **Status:** AUD-014 corrigido; choice construtor já funcionava.

## CONTRACT-COMP-001 — `comptime` e constant evaluation

- **Fase:** const_eval / checker.
- **Permitido:** dobras de literais, unário `-`, `+`, `-`, `*`, `/`, `%` para diagnósticos; comparações numéricas em compile-time.
- **Proibido:** comparar composite em comptime (limitação documentada); usar valor runtime em contexto de constante.
- **Testes:** `tests/const_values.rs`, `tests/checker.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §5.1b; `FRICTION.md`.

## CONTRACT-MOD-001 — Import, reexport e visibilidade

- **Fase:** modules.
- **Permitido:** `import`, `open import`, reexports transitives de `open`, aliases, paths relativos/absolutos, mesmo nome em módulos distintos.
- **Proibido:** `sealed import`; item `sealed` de outro módulo; ambiguidade sem qualificação; ciclos.
- **Testes:** `tests/modules.rs`, `tests/mangling.rs`, `tests/packages.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `SINTAXE.md` §13; `LOI_SPEC.md` §4.8.

## CONTRACT-FFI-001 — `opaque`, `cstr` e `extern flow`

- **Fase:** checker + codegen + linker.
- **Permitido:** handles opacos como `Copy`; `cstr` para FFI; `extern flow` com shim explícito (`--c-source`).
- **Proibido:** dereferenciar ponteiro em Loi; `nil`; shim ausente sem diagnóstico útil.
- **Testes:** `tests/checker.rs`, `tests/codegen.rs`, `tests/handles.rs`, `tests/process.rs`.
- **C11/LLVM:** ✓ / parcial. **Spec:** `SINTAXE.md` §12; `LOI_SPEC.md` §4.10; `DECISIONS.md`.

## CONTRACT-C11-001 — C11 como produção

- **Fase:** codegen + C compiler.
- **Permitido:** C11 válido com `-std=c11 -pedantic-errors -Wall -Wextra`; aritmética checada; ordem left-to-right; ABI estável.
- **Proibido:** overflow silencioso, avaliação de RHS quando LHS decide, símbolos sem mangling quando há colisão, warning escondido.
- **Testes:** `tests/codegen.rs`, `tests/order.rs` (320 gerados), `tests/abi_golden.rs`, `tools/ci.sh` (c11-strict).
- **C11/LLVM:** ✓ / ✓ subset. **Spec:** `LOI_SPEC.md` §4.3–4.4, §7.

## CONTRACT-LLVM-001 — Paridade no subset suportado

- **Fase:** LLVM backend.
- **Permitido:** mesmos resultados e traps do C11 para escalares/slices/protect/alloc suportados; diagnóstico claro para unsupported.
- **Proibido:** wrapping silencioso em overflow/div0/shift; resultado diferente sem diagnóstico.
- **Testes:** `tests/llvm.rs` (`llvm_matches_c11_arithmetic_traps`, `llvm_backend_matches_c11_for_slices_protect_and_alloc`).
- **C11/LLVM:** ✓ / parcial. **Spec:** `LOI_SPEC.md` §7; `DECISIONS.md` 2026-11-24.

## CONTRACT-DIAG-001 — Diagnósticos confiáveis

- **Fase:** diagnostic / CLI / LSP.
- **Permitido:** human/short/JSON; severity correta; cores com opt-out; spans 1-based; help e labels; exit 0 para warning.
- **Proibido:** panic, JSON inválido, coluna fora do arquivo, erro duplicado que esconda a causa.
- **Testes:** `tests/errors.rs` (goldens), `tests/cli.rs`, `tests/diagnostic_render.rs`, `tests/semantic_tokens.rs`, `tests/lsp.rs`.
- **C11/LLVM:** n/a. **Spec:** `SINTAXE.md` §16; `LOI_SPEC.md` §8.

## CONTRACT-FMT-001 — Formatação estável

- **Fase:** pretty-printer.
- **Permitido:** `loi fmt` produz apenas sintaxe canônica; formatar duas vezes é idempotente; braces para shape/mark/choice.
- **Proibido:** emitir `Name[ ... ]`, `interface`, `let`, `[]T`; mudar semântica ao formatar.
- **Testes:** `tests/pretty.rs`; propriedade de idempotência a adicionar em `tests/property_smoke.rs`.
- **C11/LLVM:** n/a. **Spec:** `SINTAXE.md`; `DECISIONS.md` 2026-11-22.

## CONTRACT-CI-001 — Gate reproduzível

- **Fase:** infraestrutura.
- **Permitido:** `./tools/ci.sh` com steps nomeados; inventário de ferramentas; skip explícito; goldens protegidos; MSRV exigido no CI.
- **Proibido:** skip silencioso, `LOI_BLESS=1` no CI, golden sujo, tool ausente tratado como sucesso.
- **Testes/scripts:** `.github/workflows/ci.yml`, `tools/ci.sh`, `tools/check_test_output.py`, `tools/check_goldens.py`, `tools/check_msrv.sh`.
- **C11/LLVM:** aplica a ambos. **Spec:** `development/AGENTS.md`.

---

## Contratos resolvidos nesta auditoria

- **AUD-014 / CONTRACT-TYPE-003:** mark/choice como argumento direto.
- **AUD-015 / OPEN-04:** literal negativo grande nos dois backends.
- **AUD-016 / CONTRACT-OWN-003:** move antes de `break`/`continue` em loop.
- **AUD-017 / CONTRACT-EFF-001:** `<-` de composto parcialmente movido para extern flow.

## Contratos ainda em aberto

- **CONTRACT-PROT-002:** taint de `protect` através de `<->` — aproximação conservadora atual é contrato ou lacuna a corrigir? (sem mudança nesta auditoria)
- **CONTRACT-TYPE-003:** resolvido — mark/choice como argumento direto passam a ser tratados como valores (AUD-014).
- **CONTRACT-TYPE-001 / OPEN-04:** resolvido — literal negativo é dobrado na largura esperada nos dois backends (AUD-015).
- **CONTRACT-GUARD-002 / OPEN-07:** convenção de release para recursos genéricos.
- **OPEN-02:** `xs.len` escrito além da capacidade (trap, unchecked ou type-level).
- **OPEN-05:** conversão slice ↔ `cstr` além de literais.
- **OPEN-06:** `<-` de composto (endereço vs valor no C11).
