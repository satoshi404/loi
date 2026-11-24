# Auditoria do compilador Loi — ETAPA 1 (somente leitura)

Data: 2026-11-24. Commit auditado: `8712bd4` (`Stdlib: modular packages with open import re-exports`). Nenhum arquivo de código, teste, spec ou CI foi alterado nesta etapa. Reproduções foram escritas apenas em `/tmp/opencode/loi-audit-repro/`.

## 1. Escopo e método

Leitura integral, nesta ordem, de `development/LOI_SPEC.md`, `SINTAXE.md`, `development/PROGRESS.md`, `development/DECISIONS.md` (obrigatório em `development/AGENTS.md`), seguida de `development/FRICTION.md`, `development/PROBES.md`, `development/fix_problems.md`, `development/ideia.md`, `README.md`, de todos os `src/*.rs` e da suíte em `tests/`. Três agentes de leitura independentes cobriram frontend, backend e infraestrutura. Os achados marcados como **confirmados** foram reproduzidos com o binário já construído em `target/debug/loi`; os demais estão marcados como **confirmados por leitura**.

Spec canônica: `SINTAXE.md`, complementada por `LOI_SPEC.md` e `DECISIONS.md`. Contradições foram registradas antes de qualquer decisão (§3). Nenhuma semântica de `<-`, `->`, `<->`, `=>`, ownership, `give`, `clone`, `protect`, `guard`, `view`, `comptime`, módulos, FFI ou backends foi alterada.

## 2. Baseline da suíte atual

Comando exato:

```sh
export PATH="/root/.cargo/bin:$PATH"
export RUSTFLAGS="-C linker=/usr/bin/gcc"
cargo fmt --check &&
cargo clippy --all-targets -- -D warnings &&
cargo test --all-targets &&
python3 tools/sync_examples.py --check &&
git diff --check
```

Resultado (log completo em `/tmp/opencode/loi-audit-baseline.log`):

| Etapa | Resultado |
|---|---|
| `rustc` / `cargo` | `1.100.0` (MSRV declarada: 1.88) |
| `cc` / `clang` / arquitetura | GCC 15.2.0 / Clang 21.1.8 / `aarch64` |
| `cargo fmt --check` | exit 0 |
| `cargo clippy --all-targets -- -D warnings` | exit 0, sem warnings |
| `cargo test --all-targets` | exit 0 — **575 passed, 0 failed, 0 ignored, 0 measured, 0 filtered**, 33 suítes |
| `python3 tools/sync_examples.py --check` | `website/examples is in sync`, exit 0 |
| `git diff --check` | exit 0 |

O teste mais longo é `tests/order.rs::three_hundred_generated_programs_match_the_left_to_right_oracle` (315 s, 320 programas gerados). Não há testes ignorados. A suíte é verde; os problemas abaixo são lacunas de contrato que a suíte atual não detecta.

## 3. Contradições documentais registradas (decisão pendente, nada foi escolhido silenciosamente)

| ID | Sev. | Evidência | Contradição | Correção proposta (sem mudar semântica) |
|---|---|---|---|---|
| DOC-01 | P2 | `LOI_SPEC.md:112,113,1032,1033` vs `SINTAXE.md:109`, `LOI_SPEC.md:972,982`, `DECISIONS.md` 2026-11-22 | Exemplos usam `Vec2[ x: ... ]`, mas a forma canônica é `Vec2 { x: ... }`. A própria spec diz que as seções 3 e 7 são canonical-only (`LOI_SPEC.md:1060`). | Substituir os quatro exemplos por braces. |
| DOC-02 | P2 | `AGENTS.md:39` vs `SINTAXE.md:109`, `DECISIONS.md` 2026-11-22 | `AGENTS.md` proíbe `Name { ... }`, hoje literal de shape canônico. Regra stale do período em que braces eram legados. | Remover `Name { ... }` da lista de palavras proibidas; manter proibição de `interface`, `let`, `[]T`, etc. |
| DOC-03 | P2 | `LOI_SPEC.md:428,915` vs `SINTAXE.md:936`, `DECISIONS.md` 2026-11-22, `PROGRESS.md:11` | Intrínseco `format` implementado e canônico, mas ausente nas duas listas de intrínsecos da spec. | Acrescentar `format(<- [u8], ...) => [u8]` às listas. |
| DOC-04 | P2 | `LOI_SPEC.md:631-639` vs `SINTAXE.md:80-83`, `DECISIONS.md` 2026-11-20, `src/monomorphize.rs:807-817` | Spec omite `callback` do conjunto `Copy`; implementação e `SINTAXE.md` dizem que callbacks são copiáveis. | Alinhar spec ao comportamento implementado, sem tocar em `is_copy_type`. |
| DOC-05 | P2 | `PROGRESS.md:1039-1041,1061` vs `LOI_SPEC.md:325`, `DECISIONS.md` 2026-11-18, `tests/checker.rs`/`tests/comparison.rs` | Duas limitações stale: (a) check de `alloc` trataria qualquer fluxo não-`sealed` como entry point; (b) igualdade de shapes não seria estrutural. Ambas já mudaram. | Atualizar/remover os dois bullets. |
| DOC-06 | P3 | `DECISIONS.md:134,135` vs `DECISIONS.md` 2026-11-22 | Decisões históricas de 2026-11-08 dizem que braces foram removidos e marcadas como `Done`; a decisão de 2026-11-22 reintroduz braces. | Marcar explicitamente como `Superseded by 2026-11-22`, preservando o registro histórico. |
| DOC-07 | P3 | `development/fix_problems.md` (inteiro) | Relatório de 2026-11-08 descreve 7 problemas, hoje corrigidos (escape de literal, overflow/divisão, arena que cresce, `--emit-asm` rejeitado, README, sync de exemplos). Pode induzir reimplementação. | Marcar como relatório histórico, não estado atual. |
| DOC-08 | P3 | `PROGRESS.md` (1254 linhas) vs `AGENTS.md:79` | Regra “Keep `PROGRESS.md` short; summarize old entries” violada. | Resumir entradas antigas na ETAPA 6. |

`AGENTS.md:17-19` exige corrigir divergências entre `LOI_SPEC.md` e `SINTAXE.md` na mesma mudança. Todas as correções DOC-* são documentais; nenhuma exige nova sintaxe ou semântica.

## 4. Bugs confirmados

Severidade: **P0** = corrompe memória/segurança ou impede o gate; **P1** = aceita programa inválido, produz resultado errado ou quebra contrato entre fases/backends; **P2** = diagnóstico/implementação incompleta em caso delimitado; **P3** = UX/API/consistência sem impacto em programas válidos.

### 4.1 Quadro-resumo

| ID | Sev. | Área | Sintoma confirmado | Status |
|---|---|---|---|---|
| AUD-001 | P1 | expand/codegen | `check` aceita `quote` em global; `gen` faz panic | Confirmado por execução |
| AUD-002 | P1 | modules/guard | `guard` com tipo qualificado procura `mod::http_client` em vez de `_release` | Confirmado por execução |
| AUD-003 | P1 | parser | Backtracking de `>>` corrompe slice literal com tipo genérico aninhado | Confirmado por execução |
| AUD-004 | P1 | format/codegen | `printf("%llf", 1.0)` aceito; imprime `0.000000` | Confirmado por execução |
| AUD-005 | P1 | codegen | `len()` em RHS quebra curto-circuito e ordem left-to-right | Confirmado por execução |
| AUD-006 | P1 | LLVM | Overflow assinado: C11 trap, LLVM imprime `-128` e sai 0 | Confirmado por execução |
| AUD-007 | P2 | monomorphize | Tipo explícito de `guard` não é monomorfizado | Confirmado por execução |
| AUD-008 | P2 | lexer | `0x` / `0b2` não produzem erro léxico | Confirmado por execução |
| AUD-009 | P3 | lexer/diagnóstico | Escape inválido em string é apontado na abertura `"` | Confirmado por execução |
| AUD-010 | P3 | completion | Faltam `clone`, `quote`, `splice`, `unquote` | Confirmado por execução |
| AUD-011 | P3 | LSP semanticTokens | `quote`/`unquote`/`splice` não são classificados como keyword | Confirmado por leitura + saída decodificada |
| AUD-012 | P3 | diagnostic API | `render_sourced` resume warnings como “errors” | Confirmado por leitura (sem exposição no CLI hoje) |
| AUD-013 | P3 | monomorphize/checker | Colisão de nome gerado é reportada, mas com mensagem enganosa | Confirmado por execução |

Nenhum P0 foi encontrado. Nenhum dos bugs corrompe a memória do programa gerado em C11 nos casos testados; AUD-004 e AUD-006 produzem saída silenciosamente errada.

### 4.2 Detalhes

**AUD-001 — `quote` em inicializador global escapa da validação (P1).**
`src/expand.rs:127-134` valida apenas `Item::Function`; `Item::Global` não passa por `validate_expr`. `src/checker.rs:2633-2636` confia na expansão e devolve `Ty::Code`; `src/codegen.rs:1638` não tem representação C.
Reprodução: `/tmp/opencode/loi-audit-repro/a1_global_quote.loi` — `G: Code <- quote { print_int(1) }`. `loi check` → exit 0; `loi gen` → `panicked at src/codegen.rs:1638:25: internal error: entered unreachable code: Code has no C representation` (exit 101).
Esperado (`SINTAXE.md:455-461`): `quote` apenas dentro de `comptime flow`. Correção mínima: validar também `Item::Global` em `validate_program` (e, opcionalmente, rejeitar `Ty::Code` como tipo de global no checker). Teste necessário: checker negativo + `gen` não deve panicar.

**AUD-002 — release convention perde `_release` para tipo qualificado (P1).**
`src/types.rs:384-389`: `release_flow_name_for("mod::HttpClient")` devolve `mod::http_client`, não `mod::http_client_release`. Usado por `src/modules.rs:350-380`, `src/checker.rs` e `src/codegen.rs`.
Reprodução: `/tmp/opencode/loi-audit-repro/a2_two_modules.loi` com dois módulos `a`/`b`, ambos com `HttpClient`, `http_client_release` e `make_client`. `loi check` → `module 'a' has no item 'http_client'`. Com um único módulo (`a2_main.loi`) funciona e gera `loi_fn_http_client_release`, o que explica a lacuna de teste.
Esperado (`SINTAXE.md:783-786,893-910`): `<snake_case_shape>_release`, preservando o módulo. Correção mínima: `format!("{module}::{}_release", snake_case(item))` e garantir resolução consistente do símbolo manglado. Teste: `guard` com tipo qualificado em dois módulos, positivo e negativo (`sealed` release de outro módulo).

**AUD-003 — backtracking de tipo em slice literal corrompe `>>` (P1).**
`src/parser.rs:1593-1616` restaura apenas `position`; `try_parse_type_args` (`:739-758`) também restaura `splits` e o token original.
Reprodução: `/tmp/opencode/loi-audit-repro/a7c_slice_split.loi`:
```loi
xs <- [Vec<Vec<i32>> { x: Vec<i32> { x: 1 } }, Vec<Vec<i32>> { x: Vec<i32> { x: 2 } }]
```
`loi check` → `unknown variable 'Vec'`, `field 'x' has type Vec<i32>, expected i32`, `slice literal elements have incompatible types bool and Vec<Vec<i32>>`. Controle: com `Vec<i32>` (sem `>>`), `/tmp/.../a7d_slice_split_single.loi` passa. Correção mínima: extrair helper de restauração de `try_parse_type_args` e usá-lo em `parse_slice_literal`.

**AUD-004 — conversões `printf` inválidas aceitas (P1).**
`src/format.rs:128-146` aceita qualquer repetição de `h|l|L|z|j|t`; `c_cast()` (`:48-70`) só reconhece alguns valores e cai em `double`/`int`.
Reprodução: `printf("%llf", 1.0)` (`/tmp/.../a6_printf.loi`). `loi check` → exit 0; C gerado contém `printf("...%llf", (double)1.0)`; `cc -std=c11 -pedantic-errors -Wall -Wextra -fsyntax-only` não acusa porque o formato é uma variável; execução imprime `0.000000` em vez de `1.000000`.
Esperado (`SINTAXE.md:970-982`): conversões bem definidas e casts exatos. Correção mínima: parser de comprimento com alternativas válidas (`""`, `hh`, `h`, `ll`, `l`, `L`, `z`, `j`, `t`) e checagem de compatibilidade (`L` só com float, `s`/`c` sem comprimento inteiro). Testes: negativos para `%llf`, `%lll`, `%Ls`, `%hf` e positivos para os válidos.

**AUD-005 — intrínsecos com `emit` quebram curto-circuito e ordem (P1).**
`src/codegen.rs:3397-3410` gera ambos os lados de `and`/`or` e confia em `&&`/`||` do C; porém `print`, `eprint`, `len` e `panic` chamam `emit` durante a geração (`:3711-3731`, `:3757-3766`), injetando instruções reais antes da condição. `ordered_expr`/`order_values`/`order_call_args` (`:1300-1445`) e snapshot de compound assignment (`:2792-2813`) também são afetados.
Reproduções (ambas em `/tmp/.../`):
- `b1_short_circuit.loi`: `no()` imprime `L` e devolve `false`; RHS `len(touched())` imprime `R`. Esperado `L`; obtido `RL`.
- `b1_order.loi`: `first()` imprime `L`; `len(touched())` imprime `R`. Esperado `LR`; obtido `RL`.
Esperado (`LOI_SPEC.md:567-585`, `SINTAXE.md:629-643`): LHS antes do RHS, RHS só quando necessário. Correção mínima: declarar temporário sem efeito (`declare_temp_typed`) e devolver a atribuição como `step` de `with_order`, em vez de `emit`. Impacto amplo no C gerado — exige differential gcc/clang e os 320 casos de `order.rs` como rede de segurança.

**AUD-006 — LLVM não implementa aritmética checada (P1).**
`src/llvm.rs:1280-1326` emite `add`/`sub`/`mul`/`sdiv`/`srem`/`shl`/`ashr` diretos; unário `-` em `:836-845` e compound assignment em `:609-617` também. `@loi_runtime_fail` (`:1797-1802`) não é usado nesses caminhos.
Reprodução: `/tmp/.../b2b_overflow.loi` com `x: i8 <- 127; x += 1`. C11 → `loi: integer overflow at 5:5`, exit 1. LLVM → imprime `-128`, exit 0. (`printf` não é suportado pelo LLVM, por isso a versão com `printf` foi substituída por `print_int`.)
Esperado (`LOI_SPEC.md:538-565,1124-1129`): mesmo valor, diagnóstico e status nos dois backends; C11 é o oráculo semântico. **Decisão em aberto (OPEN-01)**: implementar checagens no LLVM (`sadd.with.overflow`, zero/MIN, shifts) ou rejeitar explicitamente essas operações no subset LLVM. A segunda opção é menor, mas estreita o subset documentado; exige aprovação do autor antes de mudar spec/CLI. **Resolvido na ETAPA 6:** implementar checagens no LLVM para igualar o C11, sem mudar spec/CLI (`DECISIONS.md`, 2026-11-24).

**AUD-007 — tipo explícito de `guard` não é monomorfizado (P2).**
`src/monomorphize.rs:292-300` descarta o resultado de `rewrite_type`; `StmtKind::Let` (`:250-257`) faz `*ty = self.rewrite_type(ty, mapping);`.
Reprodução: `/tmp/.../a4_guard_generic.loi` → `generic type 'Box' must be instantiated before checking` em `guard b: Box<T>`. Correção mínima: uma linha, igual a `Let`. Teste positivo com `guard b: Box<i32>` em fluxo genérico.

**AUD-008 — literais de base malformados não são erros léxicos (P2).**
`src/lexer.rs:224-294,296-322` só verifica sufixo alfabético/`_` depois do prefixo. `0b2` vira `Integer("0b") + Integer("2")` → erro de parser em `a5_bad_literals.loi`; `0x` sozinho vira erro de constant folding (`integer literal is out of range`) em `a5_hex_only.loi`. Correção mínima: exigir ao menos um dígito válido da base e reportar `invalid number literal` no caractere ofensor. Testes lexer + checker + C gerado.

**AUD-009 — coluna de escape de string (P3).**
`src/lexer.rs:328-395` reusa `line`/`column` da abertura. Reprodução: `print("a\q")` → erro em `2:11` (o `"`), não no `\` (`2:13`). Correção: capturar posição do `\`.

**AUD-010 — completion omite keywords canônicas (P3).**
`src/complete.rs:19-24` lista 28 keywords; faltam `clone`, `quote`, `splice`, `unquote`. `loi complete p3_complete.loi --line 2 --column 10` com prefixo `clo` → `[]`. Correção: acrescentar as quatro em ordem alfabética e testar cobertura completa contra `SINTAXE.md:23-30`.

**AUD-011 — semanticTokens omite três keywords (P3).**
`src/lsp_info.rs:602-636` não inclui `Quote`, `Unquote`, `Splice` no braço de keyword. `loi lsp-info p3_semantic.loi --kind semanticTokens`, decodificado, não tem tokens nas posições de `quote` (5:8), `unquote` (6:27) e `splice` (13:5). Correção: incluir os três `TokenKind` e teste de decodificação.

**AUD-012 — `render_sourced` conta warnings como errors (P3).**
`src/diagnostic.rs:443-470` usa `sorted.len()` para o resumo, enquanto `render_human` (`:333-353`) usa `summary()`. Hoje o único chamador (`main.rs:1645-1673`) passa só errors; risco é de API pública/futuro chamador. Correção: usar `summary()`.

**AUD-013 — colisão de nome gerado reportada com mensagem enganosa (P3).**
Suspeita inicial de que `src/monomorphize.rs:482-501` não detectasse colisões com `mark`/`choice`/`opaque`/global **não se confirmou**. `shape Vec<T> { x: T }` + `mark Vec__i32 { A }` e também + `Vec__i32: i32 <- 7` produzem `duplicate type 'Vec<i32>'` e `unknown shape 'Vec<i32>'`. O contrato “reportar colisão” é preservado; falta clareza (nome legível vs manglado). Nenhuma correção de semântica necessária.

**AUD-014 — mark variant como argumento `<-` é tratado como lvalue (P2).**
`src/checker.rs::is_place` (5739-5746) classifica qualquer `ExprKind::Member` como place, sem consultar a tabela de marks. `check_read_argument` (3662-3670) chama `lvalue_info`, que recursa em `Ident("Color")` e emite `unknown variable 'Color'`. `src/codegen.rs::is_place` (1340-1348) já possui o guard equivalente.
Reprodução: `same(Color.Red, Color.Red)` com `flow same <- a: Color <- b: Color => bool`. `loi check` → 2 erros `unknown variable 'Color'`; bound via `red <- Color.Red` funciona. Choice construtor `Val.Num(7)` não reprodúz (parseia como `Call`).
Esperado (`SINTAXE.md:143,483`; `LOI_SPEC.md:198-205`): mark variant é valor. Correção mínima: guard em `check_read_argument` para `Member { base: Ident(name) }` com `marks`/`choices` contendo `name`, delegando a `check_expr`.

**AUD-015 — literal negativo grande perde valor e difere entre backends (P1).**
`src/codegen.rs::ty_of(Unary Neg)` (3231-3235) usa o tipo do operando; um `Int` nu vira `Ty::IntLiteral` → `i32`. `gen_expr` (3369-3395) emite `loi_neg_i32(<magnitude>)`; `c_int_literal` (4370-4390) acrescenta `ULL` quando a magnitude excede `u32`, e o C estreita para `int32_t` antes da negação checada. `src/llvm.rs` (847-868,1829-1840) emite a magnitude positiva crua, que envelopa em LLVM.
Reprodução: `x: i64 <- -9223372036854775808`. C11 gera `loi_neg_i32(9223372036854775808ULL, ...)` com `-Woverflow` e imprime `0`; LLVM imprime `0` e trap; `x: i32 <- -2147483648` trap no C11. Globais e casts com `as i64` já funcionam.
Esperado (`SINTAXE.md:600,612-613`; `LOI_SPEC.md:544-545,562-564`; `PROGRESS.md:1181-1186` OPEN-04): valor exato e paridade C11/LLVM, sem warning. Correção mínima: dobrar `-<literal>` em constante do tipo esperado em `gen_value`/`gen_coerced_expr` (C) e `gen_expr` (LLVM), mantendo `loi_neg_*` para expressões não-literais.

**AUD-016 — `give` antes de `break`/`continue` em `if` escapa da junção do loop (P1).**
`src/checker.rs::check_statement` (1944-1980) descarta `then_state` no caso `(false, true)` do `if`; `break`/`continue` (2176-2187) apenas marcam `reachable = false` sem registrar estado. Um `give` em um ramo que sai do loop não chega ao merge pós-loop.
Reprodução: `for i in 0..n { if i == 1 { _ <- give(p.left); break } }; => p.left.len` → check Limpo, aceito.
Esperado (`LOI_SPEC.md §4.2`, PROGRESS.md ownership por caminhos): uso após transferência deve ser erro. Correção mínima: coletar snapshots em `loop_exit_states` nos `break`/`continue` e fazer merge deles no estado pós-loop de `while`/`for`/`for in`.

**AUD-017 — extern flow com `<-` parcialmente movido é aceito (P1).**
`src/checker.rs::analyze_function_effects` (1517-1525) inicializa `param_reads` com conjuntos vazios para **toda** função, inclusive `extern flow` (sem corpo). `check_read_argument` (3677-3706) percorre um conjunto vazio e não reporta nada, embora `PROGRESS.md:1041-1043` afirme que callee desconhecido lê o valor inteiro.
Reprodução: `extern flow peek <- p: Pair`; `_ <- give(p.left)`; `peek(p)` → check limpo.
Esperado (`PROGRESS.md:1041-1043`, `CONTRACTS.md` CONTRACT-EFF-001): erro `use of moved value 'p'`. Correção mínima: para `signature.external`, inicializar cada parâmetro `<-` com `{""}` (caminho raiz) em `param_reads`.

## 5. Infraestrutura, testes e CI (achados para ETAPA 5)

Inventário: 30 arquivos de integração com 540 testes + 35 testes unitários = **575**; 40 pares de goldens; 320 programas gerados em `tests/order.rs`; 10 testes Node do playground. Cobertura densa de checker (158), codegen C11 (66), CLI (47), libs (47), parser (32). Lacunas:

| ID | Sev. | Achado | Evidência |
|---|---|---|---|
| INF-001 | P1 | Não existe configuração de CI versionada; gate é manual | sem `.github/`, `Makefile` ou `justfile`; `README.md:196-206` |
| INF-002 | P1 | ~23 helpers com `/tmp/opencode` fixo; 19 alvos falham em MSYS/Windows | `tests/cli.rs:13`, `tests/lib.rs:14`, `PROGRESS.md:16,22` |
| INF-003 | P1 | 69 `eprintln!("skipping ...")` passam silenciosamente sem `cc`/`clang`/`node` | `tests/cli.rs`, `llvm.rs`, `header.rs`, `bindgen.rs`, `order.rs` |
| INF-004 | P1 | `LOI_BLESS=1` reescreve goldens; sem teste de integridade do corpus | `tests/errors.rs:46,70-73`, `KNOWN_BAD.txt:3-5` |
| INF-005 | P1 | Sem goldens de ABI: C gerado, header, IR LLVM e source map usam só `contains(...)` | `tests/codegen.rs:7-27`, `tests/header.rs`, `tests/llvm.rs:82-98` |
| INF-006 | P1 | Lacunas de contrato já documentadas sem teste: literal `-9223372036854775808`, mark/choice como argumento direto, `xs.len` > capacidade — **AUD-014 e AUD-015 confirmados e corrigidos**; `xs.len` permanece OPEN-02 | `PROGRESS.md:1171-1176,1063-1067,1044-1047` |
| INF-007 | P1 | ASan nunca executado (apenas build); UBSanClean é executado | `tests/lib.rs:114-116,174-193`, `tests/codegen.rs:71-114`, `PROBES.md:402-417` |
| INF-008 | P2 | MSRV testada como `>= 1.85`, mas `Cargo.toml` declara 1.88; `check_msrv.sh` sai 0 quando falta rustup | `tests/msrv.rs:22-30`, `tools/check_msrv.sh:13-21` |
| INF-009 | P2 | Property-based restrito a `order.rs`; differential C11/LLVM cobre só 3 programas; fuzz e mutation inexistentes | `tests/llvm.rs:123,180,295`, `Cargo.toml:9` |
| INF-010 | P2 | `loi fmt` binário não testado diretamente; assets LSP de `build.rs` sem golden; vendor `skipped.txt` com paths absolutos | `tests/llvm.rs:450-469`, `build.rs:28-47`, `vendor/*/skipped.txt:2` |
| INF-011 | P3 | Diretórios temporários sem limpeza/PID em `order.rs:406-419`, `lsp.rs:18-23`, `guard.rs:148` | relatório do agente de infraestrutura |

Não há whitelist ativa em `KNOWN_BAD.txt` — aspecto positivo: nenhum teste foi transformado em ignorado para esconder falha.

## 6. Matriz de contratos (estado final após ETAPA 6)

Legenda: **C11** = backend de produção; **LLVM** = subset opt-in; `n/a` = fora do subset documentado; `OPEN` = regra não definida, não escolher silenciosamente.

| ID | Contrato | Pré-condição / fase | Permitido | Proibido | Testes positivos / negativos / runtime | C11 | LLVM | Spec |
|---|---|---|---|---|---|---|---|---|
| CTR-01 | Ownership único; `give` transfere, `clone` duplica | checker | cópia escalar implícita; `clone` explícito | uso após `give`; cópia implícita de composto | `tests/checker.rs` (muitos), `tests/codegen.rs` | ✓ | parcial | `LOI_SPEC` §4.2 |
| CTR-02 | Direções `<-`, `->`, `<->`, `=>` | assinatura + call site | leitura, init, inout, retorno | aliases em `<->`; escrita via `<-` | `tests/checker.rs::checks_parameter_directions_at_call_sites` | ✓ | parcial | `SINTAXE` §3 |
| CTR-03 | `protect` / `alloc`: nada escapa | região léxica | alloc em helper sob `protect` | escape para outer/retorno | `tests/checker.rs`, `tests/codegen.rs::compiled_protect_restores_the_arena` | ✓ | parcial | `LOI_SPEC` §4.9 |
| CTR-04 | `guard` libera uma vez, LIFO | shape + `_release` exato | return/break/continue dentro do bloco | `release` explícito; mover o recurso | `tests/guard.rs`, `tests/handles.rs`, `tests/modules.rs` | ✓ (com mangling de instância, OPEN-07) | n/a | `SINTAXE` §11 |
| CTR-05 | Aritmética inteira checada | runtime | wrap unsigned; trap signed/div0/shift | overflow silencioso | `tests/codegen.rs`, `tests/llvm.rs::llvm_matches_c11_arithmetic_traps` | ✓ | ✓ (OPEN-01 resolvido por paridade) | `LOI_SPEC` §4.3 |
| CTR-06 | Ordem left-to-right e curto-circuito | todas as expressões | avaliar LHS antes do RHS | RHS avaliado quando LHS decide | `tests/order.rs` (320 gerados + intrínsecos) | ✓ GCC + Clang | ✓ subset | `LOI_SPEC` §4.4 |
| CTR-07 | `printf` / `format` conversions | literal de formato | conversões válidas com cast exato | `%*`, `%llf`, `%Ls`, combinações inválidas | `tests/checker.rs`, unidades de `src/format.rs` | ✓ | `format` rejeitado (limitação documentada) | `SINTAXE` §14 |
| CTR-08 | `view` / loans | checker | view de view; loan em field | mover source com view viva | `tests/views.rs` (17) | ✓ | n/a | `LOI_SPEC` §4.6 |
| CTR-09 | `comptime`, `quote`, `unquote`, `splice` | expansão antes do checker | Code em `comptime flow` | Code em global/runtime | `tests/checker.rs`, `tests/parser.rs`, `tests/completion.rs` | ✓ | n/a | `SINTAXE` §5.5a |
| CTR-10 | Módulos, aliases, mangling | import/alias | mesmo nome em módulos distintos | item `sealed` de outro módulo | `tests/modules.rs`, `tests/mangling.rs` | ✓ | ✓ subset | `SINTAXE` §13 |
| CTR-11 | Diagnósticos: human/short/JSON, severidade, cores | CLI/API | warning com exit 0 | error com exit 0 | `tests/errors.rs`, `tests/cli.rs`, `tests/diagnostic_render.rs`, `tests/semantic_tokens.rs` | ✓ | parcial | `SINTAXE` §16 |
| CTR-12 | Generics, bounds, monomorphização | antes do checker | `Clone`/`Copy`/`Eq`/`Ord` | tipo genérico não instanciado | `tests/checker.rs`, `tests/comparison.rs`, `tests/guard.rs` | ✓ | ✓ subset | `LOI_SPEC` §5.1a |
| CTR-13 | Escape léxico de slice literal | checker | `[T:]` vazio pode escapar | `[a,b]` fora do bloco | `tests/checker.rs::slice_literal_cannot_escape_the_block_that_created_it` | ✓ | parcial | `LOI_SPEC` §4.7 |

| CTR-14 | Literais radix e escapes léxicos | lexer | `0x1`, `0b1`, escapes canônicos | `0x`, `0b2`, `\q` sem erro | `tests/lexer.rs` | ✓ | ✓ | `SINTAXE` §2 |
| CTR-15 | Completação e semantic tokens de keywords | API LSP | `clone`, `quote`, `splice`, `unquote` presentes | keyword classificada como identificador | `tests/completion.rs`, `tests/semantic_tokens.rs` | ✓ | n/a | `SINTAXE` §14 |
| CTR-16 | ABI C gerada estável | codegen | assinaturas/helpers versionados no golden | mudança silenciosa de layout/símbolo | `tests/abi_golden.rs` | ✓ | n/a | `LOI_SPEC` §7 |
| CTR-17 | Gate versionado e goldens protegidos | CI | `./tools/ci.sh` com steps nomeados | skip silencioso, `LOI_BLESS=1` no CI | `.github/workflows/ci.yml`, `tools/ci.sh`, `tools/check_goldens.py` | ✓ | ✓ | `AGENTS.md` |

**OPEN (sem escolha silenciosa):**
- **OPEN-01** — ~~Política LLVM para traps aritméticos~~ **Resolvido**: implementar checagens para igualar o C11, registrado em `DECISIONS.md` (2026-11-24).
- **OPEN-02** — Contrato de `xs.len` escrito além da capacidade (`PROGRESS.md:1044-1047`): trap, unchecked ou type-level capacity?
- **OPEN-03** — ~~`mark`/`choice` como argumento direto~~ **Resolvido (AUD-014)**: mark variant e choice construtor são valores; o checker não trata mais `Name.Variant` como lvalue.
- **OPEN-04** — ~~Largura de literais negativos grandes~~ **Resolvido (AUD-015)**: `-<literal>` é dobrado na largura esperada nos dois backends, com paridade C11/LLVM.
- **OPEN-05** — Conversão slice ↔ `cstr` além de literais (`DECISIONS.md`, `FRICTION.md`).
- **OPEN-06** — `<-` de composto: spec descreve acesso por endereço sem cópia; C11 passa por valor (`PROGRESS.md:1191-1193`). Hoje seguro porque o checker proíbe escape; revisar se houver concorrência.
- **OPEN-07** — Recursos genéricos com `guard`: a convenção de release usa o nome manglado da instância (`Box<i32>` → `box__i32_release`), não o flow genérico `box_release<T>`. Confirmado na ETAPA 3; corrigir a convenção exigiria decisão de semântica/mangling, portanto não foi alterado.

## 7. Limitações intencionais (não são bugs)

- Backend C11 é o caminho de produção; LLVM é opt-in, build-only, arquivo único, somente executável, subset escalar (`DECISIONS.md` 2026-11-21).
- `--emit-asm` rejeitado com exit 2; `ir.rs`/`asm.rs` são rascunhos (`AGENTS.md:51-53`, `PROGRESS.md:1194-1195`).
- ASan não roda neste PRoot/Termux (shadow memory); UBSan roda e está limpo nos probes; nenhuma alegação de ASan validado.
- `protect` usa arena em chunks crescentes; `--arena-size` controla o primeiro chunk, não um teto (`AGENTS.md:54-55`, `PROBES.md:315-336`).
- Parser aceita braces legados para `shape`/`mark`/`choice` apenas como migração; `loi fmt` emite somente forma canônica (`DECISIONS.md` 2026-11-22).
- Bounds, callbacks tipados, typed FFI e subconjuntos LLVM são decisões registradas, não falhas.

## 8. Plano para as próximas etapas

1. **ETAPA 2 — teste antes da correção:** escrever testes mínimos failing-first para AUD-001…008 e AUD-010, sem tocar na implementação. Usar repros de `/tmp/opencode/loi-audit-repro/` como base. Para AUD-005, adicionar casos ao oracle de ordem existente; para AUD-006, escrever teste differential C11/LLVM que hoje falha. Registrar as mensagens de falha antes de corrigir.
2. **ETAPA 3 — correções mínimas:** uma mudança por bug, sempre com o teste já vermelho. AUD-001, 002, 003, 007 e 010 são localizados; AUD-004 e 005 exigem cuidado com o C gerado; AUD-006 depende de OPEN-01.
3. **ETAPA 4 — regressão:** rodar gate completo (575 testes iniciais + novos), differential gcc/clang e C11/LLVM, `cc -std=c11 -pedantic-errors -Wall -Wextra` nos artefatos gerados.
4. **ETAPA 5 — infraestrutura:** INF-001…011; priorizar CI versionado, portabilidade de fixtures, fail-on-skip controlado, proteção de goldens, goldens de ABI e fuzz smoke sem dependências novas.
5. **ETAPA 6 — documentação:** aplicar DOC-01…08, atualizar `PROGRESS.md` com evidências exatas, expandir a matriz de contratos e registrar decisões (especialmente OPEN-01).

Todas as etapas foram executadas; ver `AUDIT_ETAPA2.md`…`AUDIT_ETAPA6.md`.
