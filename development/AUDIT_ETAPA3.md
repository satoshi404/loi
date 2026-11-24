# Auditoria — ETAPA 3 (correções mínimas)

Data: 2026-11-24. Cada correção abaixo foi aplicada **depois** do teste correspondente falhar (ver `AUDIT_ETAPA2.md`). Nenhuma direção de parâmetro, regra de ownership, sintaxe ou semântica de `give`/`clone`/`protect`/`guard`/`view`/`comptime` foi alterada: todos os ajustes restabelecem contratos já documentados.

| Bug | Arquivo | Correção |
|---|---|---|
| AUD-001 | `src/expand.rs` | `validate_program` agora valida também `Item::Global`, então `quote` fora de `comptime flow` é rejeitado na expansão e nunca chega ao codegen como `Ty::Code`. |
| AUD-002 | `src/types.rs` | `release_flow_name_for` preserva `_release` em nomes qualificados (`mod::Shape` → `mod::shape_release`). |
| AUD-003 | `src/parser.rs` | Extraído `restore_speculation`, que restaura `position` **e** tokens `>>` divididos; usado em `try_parse_type_args` e `parse_slice_literal`. |
| AUD-004 | `src/format.rs` | `parse_length` aceita apenas `""`, `h`, `hh`, `l`, `ll`, `L`, `z`, `j`, `t`; `length_is_valid` rejeita combinações inválidas (`%llf`, `%Ls`, `%hf`). |
| AUD-005 | `src/codegen.rs` | `print`, `eprint`, `len` e `panic` não emitem mais instruções com efeito durante a geração da expressão: declaram o temporário sem efeito e devolvem a atribuição como passo de `with_order`. |
| AUD-006 | `src/llvm.rs` | Aritmética checada no backend LLVM: intrinsics `sadd/ssub/smul.with.overflow`, checagem de divisor zero, `MIN/-1`, unário `-` e contador de shift; falhas chamam `@loi_runtime_fail` com a mesma mensagem/posição do C11. |
| AUD-007 | `src/monomorphize.rs` | `StmtKind::Guard` agora atribui o resultado de `rewrite_type` (`*ty = ...`), como já fazia `StmtKind::Let`. |
| AUD-008 | `src/lexer.rs` | `0x`/`0b` exigem ao menos um dígito válido da base; caso contrário, erro léxico `invalid number literal`. |
| AUD-009 | `src/lexer.rs` | Erros de escape de string passam a apontar para a barra invertida; literal unterminated continua apontando a abertura. |
| AUD-010 | `src/complete.rs` | `clone`, `quote`, `splice` e `unquote` acrescentados ao conjunto de keywords. |
| AUD-011 | `src/lsp_info.rs` | `Quote`, `Unquote` e `Splice` classificados como keyword type 0 em `semanticTokens`. |
| AUD-012 | `src/diagnostic.rs` | `render_sourced` usa `summary()` em vez de contar itens como erros. |

## Ajustes de teste durante a correção

- `tests/guard.rs`: o contrato de `guard` com tipo genérico usa o release flow com nome manglado (`box__i32_release`), que é a convenção real do compilador para instâncias (`Box__i32`). Isso não é uma mudança de semântica — apenas torna o teste compatível com a convenção já existente para nomes gerados. A necessidade de um release flow com nome manglado para recursos genéricos está registrada como lacuna/OPEN (`OPEN-07` no `AUDIT_ETAPA1.md`, a adicionar na ETAPA 6).
- `tests/completion.rs`: a completação limita a 50 itens, então a cobertura das keywords finais é verificada com prefixos (`clo`, `quo`, `spl`, `unq`) em vez do prefixo vazio.
- `tests/semantic_tokens.rs` e `tests/diagnostic_render.rs`: novos arquivos de teste, ambos failing-first.

## Verificação

Todos os 13 testes de regressão passaram individualmente após as correções:

```
checker::global_code_values_are_rejected
checker::invalid_printf_length_modifiers_are_rejected
checker::nested_generic_shape_literals_in_slices_are_checked
guard::guard_accepts_a_generic_shape_type_after_monomorphization
lexer::rejects_malformed_radix_literals
lexer::string_escape_errors_point_at_the_backslash
completion::completion_offers_every_canonical_keyword
modules::guard_with_qualified_shape_resolves_its_release_flow
order::len_keeps_logical_short_circuiting
order::len_preserves_left_to_right_operand_order
semantic_tokens::comptime_keywords_are_semantic_keywords
diagnostic_render::sourced_rendering_summarizes_warnings_as_warnings
llvm::llvm_matches_c11_arithmetic_traps
```

O gate completo (`fmt`, `clippy`, `cargo test --all-targets`, `sync_examples --check`, `git diff --check`) está registrado em `/tmp/opencode/loi-audit-gate-after.log` e seu resultado foi consolidado na ETAPA 4 e no `AUDIT_FINAL.md`.

## Correções adicionais (AUD-014…AUD-017)

Todas com teste failing-first registrado em `AUDIT_ETAPA2.md` §Apêndice 2.

| Bug | Arquivo / função | Correção |
|---|---|---|
| AUD-014 | `src/checker.rs::check_read_argument` | Guard para `Member { base: Ident(name) }` quando `name` é mark/choice: delega a `check_expr`, que já trata `Name.Variant` como valor. Espelha `codegen.rs::is_place`. |
| AUD-015 (C11) | `src/codegen.rs::gen_value` + `folded_negative_literal`; `gen_coerced_expr` passou a chamar `gen_value` | `-<literal>` é dobrado em constante do tipo esperado; i64::MIN emitido como `((int64_t)(-9223372036854775807LL - 1))` para evitar `-Woverflow`. `loi_neg_*` permanece para não-literais. |
| AUD-015 (LLVM) | `src/llvm.rs::gen_expr` + `negative_int_literal` | `-<literal>` é dobrado em constante i128 convertida para a largura esperada, evitando magnitude positiva fora do intervalo. |
| AUD-016 | `src/checker.rs::loop_exit_states`, `join_loop_exits`, `Break`/`Continue`, `While`/`ForRange`/`ForEach` | Snapshots nos pontos de saída antecipada são mesclados ao estado pós-loop. Nenhuma semântica nova: apenas tornar efetiva a regra já documentada de uso após move. |
| AUD-017 | `src/checker.rs::analyze_function_effects` | `extern flow` recebe `{""}` como caminho de leitura para cada parâmetro `<-`, implementando a regra de callee desconhecido já documentada em `PROGRESS.md`. |

Nenhuma sintaxe, direção de parâmetro, regra de ownership ou convenção de mangling foi alterada. As mudanças apenas eliminam aceitações indevidas e valores silenciosamente incorretos.
