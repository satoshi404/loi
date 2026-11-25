# Data Flow em Loi — proposta da FASE 1

> **Estado:** FASE 1 aprovada; FASE 2 reaberta e estabilizada em 2026-11-24;
> FASE 3 (representação tipada no checker) concluída em 2026-11-25.
> A proposta não altera `SINTAXE.md`, `LOI_SPEC.md`, AST, backends, sintaxe,
> semântica ou ABI. `DataPath`/`FlowSummary`, o visitante único, a álgebra de
> sobreposição e os sete efeitos estão implementados. A reauditoria corrigiu
> `PRODUCE` como efeito de resultado, `ESCAPE` com origem/slot relativos,
> `unknown_effects` conservador e a composição de caminhos aninhados. A FASE 3
> removeu os round-trips por string no fixpoint e migrou moves, taints, loans e
> conjuntos de scan para `DataPath`; `LvalueInfo`/`place_path` seguem como
> fronteiras legadas. Recursão com limite/widening, split/join, typestate e
> pipeline continuam **PROPOSTO**.
> Evidências: `development/AUDIT_DATA_FLOW.md` e `development/PROGRESS.md`.
>
> Fontes canônicas: `SINTAXE.md`, `development/LOI_SPEC.md`,
> `development/DECISIONS.md`, `development/PROGRESS.md`,
> `development/CONTRACTS.md`. Este documento é apenas o desenho da evolução;
> quando for aceito, as mudanças de regra devem ir para os arquivos canônicos,
> e as razões para `DECISIONS.md`.

## Resumo executivo

A Loi já possui uma base real sobre a qual construir o Data Flow: direções
explícitas, ownership por caminhos, resumos de consumo/leitura, resumos de view,
efeitos de alocação, regiões léxicas e junção conservadora de estados. O que
falta não é um operador novo; é **unificar essas peças em um contrato de
efeitos verificável**, com caminhos estruturados e propagação interprocedural.

A proposta é:

1. tornar `DataPath` a representação única de caminhos;
2. tornar `FlowSummary` o resumo único de efeitos, substituindo sem duplicar os
   mapas atuais de consumo, leitura, views e alocação;
3. inferir READ/WRITE/INIT/MOVE/BORROW/PRODUCE/ESCAPE a partir do corpo;
4. propagar esses efeitos por fixpoint determinístico, incluindo genéricos e
   recursão;
5. usar o mesmo resumo para checar chamadas, aliases, regiões e loans;
6. só depois disso considerar `split`, typestate e pipeline como açúcar sobre
   construções já existentes;
7. manter C11 como semântica de referência e LLVM com diagnóstico explícito
   onde o subset não suporta a construção.

---

## 1. Arquitetura atual do fluxo de dados

Pipeline real do compilador:

```text
lexer/token
  -> parser (AST com Direction e CallArg.direction)
  -> expand_program (quote/unquote/splice)
  -> monomorphize_for_check
  -> checker:
       collect_names
       resolve_interfaces
       resolve_marks_and_choices
       check_interface_value_cycles
       resolve_functions
       analyze_flow_summaries
       check_interface_defaults
       declare_globals
       check_functions
  -> monomorphize (C/IR/LLVM)
  -> codegen C11 (produção) ou llvm.rs (subset opt-in)
```

Pontos importantes:

- `Direction` existe na AST (`src/ast.rs:66-73`), em `Param`
  (`src/ast.rs:377-383`) e em `CallArg` (`src/ast.rs:558-562`). No checker,
  vira `ParamSig`/`FunctionSig` (`src/checker.rs:190-203`).
- `=>` não é um efeito separado no AST: é `FunctionDecl.return_type`
  (`src/ast.rs:355-375`) e, semanticamente, uma saída implícita.
- A ownership viva está em `VarInfo`, `ControlState` e nas pilhas de regiões
  (`src/checker.rs:205-401`).
- Os efeitos interprocedurais são calculados por `analyze_flow_summaries`
  (`src/checker.rs`), que substitui os quatro fixpoints específicos e o
  fixpoint de views; os mapas duráveis antigos viraram acessores de
  compatibilidade no único `FlowSummary` (`AUDIT_DATA_FLOW.md`).
- `src/ir.rs` e `generate_ir` são um rascunho auxiliar: não carregam
  direção, ownership, efeitos, guard, protect, loans ou typestate
  (`src/ir.rs:259-272`, `src/codegen.rs:4473-4523`). O LLVM é gerado
  separadamente a partir do AST checado/monomorfizado.
- Os backends não decidem ownership. C11 usa `T` para `<-`, `T*` para
  `->`/`<->` (`src/codegen.rs:2548-2555`) e aplica limpeza de guard/protect;
  LLVM ignora `Direction` e passa todos os parâmetros por valor
  (`src/llvm.rs:504-531`, `src/llvm.rs:898-951`).

Conclusão: o lugar natural do Data Flow é **antes do codegen, dentro do
checker**, com uma estrutura compartilhada — não na IR e não nos backends.

---

## 2. O que já existe e pode ser reaproveitado

### Direções e chamadas

- `<-` = leitura; `->` = inicialização da saída do chamador; `<->` = leitura e
  escrita exclusiva; `=>` = resultado. Regras canônicas em
  `SINTAXE.md:481-501` e `development/LOI_SPEC.md:225-265`.
- `check_call` já coordena checagem de argumento, aliases, consumo e
  propagação de regiões/views (`src/checker.rs:3025-3174`).
- `check_call_aliasing` já permite duas leituras e rejeita sobreposição de um
  argumento `->`/`<->` com qualquer outro (`src/checker.rs:3633-3682`).

### Caminhos

- `VarInfo.moved` guarda `(caminho relativo, span)`, com `""` para o valor
  inteiro (`src/checker.rs:205-227`).
- `PathComponent::{Field, Index}` e `parse_path` já distinguem campo,
  índice constante e índice dinâmico (`src/checker.rs:5553-5588`).
- `append_path`, `append_relative_path`, `path_is_ancestor`, `paths_overlap`,
  `path_is_moved` e as funções de inserção de resumo são a base da álgebra de
  caminhos (`src/checker.rs:5530-5687`).
- Índices constants disjuntos já são tratados como independentes; índice
  dinâmico é wildcard (`src/checker.rs:5829-5834`).

### Resumos

- `FunctionScan` registra leituras diretas, `give`, chamadas, alocação, clone e
  retorno composto (`src/checker.rs:277-311`).
- `param_consumption` e `param_reads` são conjuntos de caminhos por parâmetro
  (`src/checker.rs:379-384`).
- `view_returns`/`view_out` propagam views por índice de parâmetro
  (`src/checker.rs:250-266`, `src/checker.rs:387-390`).
- `clone_allocators` e `unguarded_allocators` propagam efeito de alocação
  (`src/checker.rs:372-377`).
- `ControlState` já faz junção conservadora: inicialização com interseção,
  move/taints/views com união (`src/checker.rs:4963-5004`), com tratamento de
  `break`/`continue` (`src/checker.rs:2205-2219`, `src/checker.rs:4943-4956`).

### Typestate e regiões

- `mark` é um tipo escalar nomeado (`src/types.rs:22`), usável como argumento
  genérico; monomorfização gera `Connection__Created`
  (`src/monomorphize.rs:827-859`) com layout C válido
  (`src/codegen.rs:2117-2141`).
- `protect` é uma região léxica com pais (`RegionKind`, `region_stack`,
  `region_parents` em `src/checker.rs:271-401`) e taints por caminho.
- `view` não é nó AST nem keyword: é intrínseco reservado com summary de
  escape e loans léxicos (`src/checker.rs:3064-3069`,
  `src/checker.rs:3176-3217`, `src/checker.rs:4585-4786`).

Nada disso deve ser reescrito do zero. A evolução é uma **generalização
incremental**, não uma segunda análise.

---

## 3. Limitações reais encontradas

1. **Não havia um contrato de efeitos unificado (corrigido).** READ, MOVE, view
   e alocação viviam em mapas separados; WRITE, INIT, PRODUCE e ESCAPE não eram
   efeitos de primeira classe nos resumos. Hoje `FlowSummary` unifica todos;
   o histórico da correção está em `AUDIT_DATA_FLOW.md`.
2. **Caminhos root-relative no checker (parcial).** A FASE 3 removeu a segunda
   álgebra (`PathComponent`, `parse_path`, `render_path`,
   `components_match_prefix`) e migrou o estado transiente de moves, taints,
   loans e scan para `DataPath`, compartilhando `may_overlap` com os resumos.
   `LvalueInfo.path`, `place_path` e as mensagens ainda usam strings como
   fronteira legada; tornar `DataPath` a representação única de todas as fases
   continua **IMPLEMENTADO PARCIALMENTE**, com `OwnershipState` ainda **PROPOSTO**.
3. **Resumos de view eram por índice (corrigido).** `ViewSummary` foi removido;
   `ViewSource { index, path }` e `ViewOutLoan { slot, source }` registram a
   origem relativa e o slot do holder (`AUDIT_DATA_FLOW.md` §3.2).
4. **Inicialização é whole-variable.** `mark_written` marca o `VarInfo`
   inteiro como inicializado (`src/checker.rs:4300-4314`), apesar de moves
   serem path-sensitive.
5. **Exclusividade ainda é checada no call site (parcial).**
   `check_call_aliasing` compara lugares usados nos argumentos; `FlowSummary`
   já carrega `WRITE`/`INIT` pelo grafo de chamadas, mas a decisão de aliases
   ainda não consulta esses efeitos propagados.
6. **Externos eram conservadores sem contrato explícito (corrigido).** `<-`
   extern é tratado como leitura do valor inteiro; `unknown_effects` e
   `CONTRACT-EFF-002` agora explicitam a regra conservadora
   (`AUDIT_DATA_FLOW.md` §3.3). Declarar "este extern só lê `p.data`" continua
   **NÃO SUPORTADO**.
7. **Loops são aproximados por zero/uma iteração** no checker de corpo; a
   análise de efeitos percorre o corpo uma vez (`src/checker.rs:5369-5401`,
   `src/checker.rs:2005-2128`). Isso é seguro, mas não preciso.
8. **Coletores parcialmente unificados (parcial).**
   `analyze_flow_summaries`/`scan_expr_effects` eliminaram os fixpoints
   duplicados e o scanner de views; `collect_accesses`,
   `collect_direct_reads`, `collect_value_regions` e `collect_value_views`
   ainda percorrem o AST para decisões locais de ownership, região e loan.
   Unificar esses coletores continua **PROPOSTO**.
9. **Trait de região díspar.** Taints de `<->` são repassados
   conservadoramente a todas as saídas com slice
   (`src/checker.rs:3505-3557`); não se sabe qual output recebeu o dado.
10. **LLVM tem lacunas de paridade já existentes.** Direções `->`/`<->` são
    lowering por valor e `clone` profundo não é implementado
    (`src/llvm.rs:504-531`, `src/llvm.rs:953`). Isto não é causado pelo
    Data Flow, mas limita a propagação para esse backend.
11. **Documento obsoleto (corrigido).** `PROGRESS.md` negava retorno de view de
    `<-`, contrariando `SINTAXE.md` e `tests/views.rs`; a limitação stale foi
    substituída pelo contrato de loans duráveis.
12. **Havia uma pequena inconsistência de scanner (corrigida).**
    `StmtKind::Splice` chamava `ctx.expr` no scanner de views, mas não
    `record_reads` no scanner de efeitos; o visitante único passou a cobrir os
    dois caminhos.

---

## 4. Proposta formal de Flow Contracts

Nomes internos; nenhuma palavra reservada nova.

| Efeito | Significado | Origem no corpo | Regra no chamador |
|---|---|---|---|
| `READ` | ler sem modificar/consumir | identificador, campo, índice, argumento `<-` | múltiplos READ são compatíveis; conflita com `MOVE`/`WRITE`/`INIT`/`BORROW` exclusivo sobre o mesmo caminho |
| `WRITE` | modificar valor existente | atribuição, operador composto, `<->` | exclusivo; não consome ownership |
| `INIT` | inicializar valor ainda não inicializado | `->`, `alloc`, primeira escrita em destino não inicializado | destino deve estar disponível como não inicializado; exclusivo |
| `MOVE` | transferir ownership e invalidar origem | `give`, retorno de composto, `<->` que consome | origem passa a `Moved`; cheque de globais/guards/views continua |
| `BORROW` | criar/propagar empréstimo | `view`, forwarding de view, loan | origem fica protegida; escopo limitado; conflito por sobreposição |
| `PRODUCE` | criar novo valor e entregar ao chamador | `=>`, `clone`, construtor de shape/choice | tipo e região de destino devem ser definidos |
| `ESCAPE` | permitir valor/emprestimo atravessar o escopo | retorno, escrita em `->`, taint de região | exige que a região de origem sobreviva ao destino |

Dimensão separada, já existente: `ALLOC`/`RegionEffect`, ligada a `protect`.
Ela não é um novo efeito de dados, mas precisa entrar no mesmo resumo para
evitar um segundo fixpoint.

### Compatibilidade por direção

- `<-`: o callee pode produzir `READ`, `BORROW`, `PRODUCE` e `ESCAPE` de view;
  `WRITE`, `INIT` e `MOVE` sobre o parâmetro são erro de contrato.
- `->`: o callee pode produzir `INIT`, `WRITE` e `PRODUCE` no destino; ler
  antes de inicializar é erro.
- `<->`: o callee pode produzir `READ`, `WRITE`, `INIT` e `MOVE`; permanece
  exclusivo no call site.
- `=>`: `PRODUCE` com tipo declarado; `ESCAPE` sujeito a região/loan.

### Regra conservadora

Se o corpo não permite determinar um efeito com segurança:

- callee externo/desconhecido: `READ`/`WRITE`/`INIT`/`MOVE` no valor inteiro,
  conforme a direção;
- índice dinâmico: caminho com `IndexKey::Dynamic`, que sobrepõe qualquer
  índice constante;
- caminho opaco: `UnknownTail`, que sobrepõe qualquer extensão;
- genérico não monomorfizado: efeito no valor inteiro até a instância concreta.

Nunca se infere "nenhum efeito" de silêncio; `unknown_effects: true` é
propagado. **Implementado:** `extern flow` com corpo vazio e chamada sem resumo
não intrínseco marcam o flag e gravam efeitos inteiros por direção
(`AUDIT_DATA_FLOW.md`).

---

## 5. Modelo de `DataPath`

Substituir o par `(root: String, relative: String)` por uma estrutura única,
mantendo conversão para a string antiga durante a migração:

```rust
struct DataPath {
    root: String,
    segments: Vec<PathSegment>,
}

enum PathSegment {
    Field(String),
    Index(IndexKey),
    UnknownTail,
}

enum IndexKey {
    Const(i128),  // mantém literais negativos compatíveis com o checker
    Dynamic,      // wildcard conservador nesta fase
    Unknown,      // marca effects desconhecidos
}
```

`""` continua significando "o valor inteiro" e vira
`DataPath { root, segments: [] }`.

### Regras formais de sobreposição

Para `p` e `q` com a mesma raiz, compare os segmentos em ordem:

- `Field(a)` vs `Field(b)`: incompatíveis se `a != b`;
- `IndexKey::Const(a)` vs `IndexKey::Const(b)`: incompatíveis se `a != b`;
- `IndexKey::Const` vs `IndexKey::Dynamic`/`Unknown`: **pode sobrepor**;
- `IndexKey::Dynamic` vs `IndexKey::Dynamic`: **pode sobrepor** — `i` e `j` só
  seriam distintos com prova explícita, que não existe nesta fase;
- `UnknownTail` vs qualquer coisa: **pode sobrepor**.

Definições:

- `disjoint(p, q) = !may_overlap(p, q)`;
- `ancestor(p, q)`: prefixo compatível; o caminho mais curto subsume o mais
  longo na canonicalização;
- `definitely_same(p, q)`: raiz, campos e índices constantes iguais, sem
  `UnknownTail`. Usado apenas para duplicação/prova forte;
- `may_overlap` é a regra de segurança usada em.write/read conflicts.

Exemplos:

- `world.physics` vs `world.graphics` → disjuntos;
- `world.physics` vs `world.physics.velocity.x` → sobrepostos;
- `xs[0]` vs `xs[1]` → disjuntos;
- `xs[i]` vs `xs[j]` → sobrepostos;
- `xs[]` vs `xs[0]` → sobrepostos.

---

## 6. Modelo de `FlowSummary`

Um único resumo por instância de flow:

```rust
struct FlowSummary {
    params: Vec<ParamEffects>,
    result: ResultEffects,
    unknown_effects: bool,
    allocations: bool,
}

struct ParamEffects {
    reads: DataPathSet,
    writes: DataPathSet,
    inits: DataPathSet,
    moves: DataPathSet,
    borrows: BorrowSet,
    escapes: DataPathSet,
}

struct ResultEffects {
    produces: bool,
    paths: DataPathSet, // PRODUCE por lugar entregue ao chamador
    regions: RegionIdSet,
    loans: LoanIdSet,
}

struct Borrow {
    source: DataPath,
    holder: DataPath,
    mode: Shared | Exclusive,
    escaped: bool,
}
```

Na implementação atual, `borrows`/`result.loans` são representados por
`ViewSource`/`ViewOutLoan` em `src/flow.rs`; `regions`/`LoanIdSet` continuam
**PROPOSTO**.

Compatibilidade durante a migração:

- `param_consumption` vira `params[i].moves`;
- `param_reads` vira `params[i].reads`;
- `view_returns`/`view_out` viram `borrows`/`result.loans`/`escapes`;
- `clone_allocators`/`unguarded_allocators` viram `allocations` + região.

`DataPathSet` canonicaliza: remove filhos de caminhos já presentes, funde
`UnknownTail` e ordena a saída para determinismo.

---

## 7. Estratégia de análise interprocedural

1. Construir o conjunto de chaves `(modo::nome, instância_genérica)` e
   ordená-lo por nome/mangling.
2. Sementes:
   - assinaturas e direções locais;
   - `extern flow`: resumo conservador;
   - intrínsecos: resumo conhecido ou conservador.
3. Escanear cada corpo com um único visitante que produz `FlowSummary`.
4. Para cada chamada, instanciar o resumo do callee substituindo o root do
   parâmetro pelo `DataPath` do argumento.
5. Unir os efeitos no resumo do chamador. União é monotônica; `unknown_effects`
   e `allocations` só sobem de `false` para `true`.
6. Repetir até fixpoint. Recursão com limite/widening ainda não foi
   implementada; permanece **PROPOSTO** no status final deste documento.
7. Widen: se um conjunto crescer sem estabilizar em profundidade/path
   complexity, colapsar para `UnknownTail` nesse raiz.

Determinismo: iteração sobre chaves ordenadas e conjuntos canonicalizados; a
ordem de declaração não pode alterar o resultado. A atual implementação já é
estável no **conteúdo** dos resumos, mas alguns spans secundários de
diagnóstico dependem de `HashMap` — isso deve ser corrigido junto.

---

## 8. Estratégia de integração com ownership

`OwnershipState` deve ser uma visão tipada sobre `moved`, `initialized`,
`taints` e `views`, não um novo sistema:

```text
Available | Uninitialized | Moved | Borrowed(loan_id)
```

Regras no call site:

1. substituir parâmetro por caminho real do argumento;
2. checar leituras do callee contra caminhos movidos/apropriados;
3. checar escritas/inicializações contra loans e destinos já inicializados;
4. aplicar `MOVE` via `record_move` existente;
5. aplicar `BORROW` via loans existentes;
6. aplicar `PRODUCE`/`ESCAPE` via taints/region checks existentes;
7. conflitos entre argumentos usam `may_overlap` sobre
   `WRITE|INIT|MOVE|ExclusiveBorrow`; `READ`+`READ` continua permitido.

Junções continuam conservadoras: inicialização por interseção, move/loan/taint
por união. `break`/`continue` continuam usando `loop_exit_states`.

---

## 9. Integração com `protect`, `guard` e views

- `RegionInfo` reaproveita `region_parents`, `region_kinds`,
  `region_origins` e `region_stack`.
- `ESCAPE` carrega a região de origem e a região de destino; `check_region_escape`
  continua sendo a autoridade.
- `clone` = `READ(source)` + `ALLOC(current protect)` + `PRODUCE(fresh)`; o
  resultado não herda taints da origem, mas herda loans se você vier a
  representar clone de view — hoje `clone` quebra a cadeia de loan.
- `view` = `BORROW(source, Shared)` + `PRODUCE(slice)` + loan ligado à
  variável que guarda a view.
- `=> view` = `ESCAPE` do source; permitido se o loan puder ser instalado no
  chamador. Retorno de composto não-slice ainda é proibido.
- `guard` continua linear por nome; `MOVE`/`WRITE`/retorno do recurso
  guardado são erros antes de virar efeito.
- A aproximação atual de taint via `<->` permanece até que um resumo
  path-sensitive de output seja implementado; ela deve ser explícita no
  `FlowSummary` como conservadorismo, não escondida.

---

## 10. Proposta de Split/Join

**Recomendação:** implementar como açúcar, não como novo valor de runtime.

```loi
// conceitual — sintaxe não aceita nesta fase
split world {
    <-> physics: world.physics
    <-> graphics: world.graphics
    <-> audio: world.audio

    update_physics(<-> physics)
    update_graphics(<-> graphics)
    update_audio(<-> audio)
}
```

Semântica proposta:

- os nomes `physics`, `graphics` e `audio` são **aliases de caminho em tempo
  de compilação**, não variáveis com storage próprio;
- o bloco é reescrito para os caminhos originais antes da checagem final;
- o objeto original fica congelado enquanto o split está ativo; somente os
  aliases são usáveis;
- `join` é **implícito no fechamento do bloco**, coerente com `protect` e
  `guard`. `join` explícito só seria adicionado se houver necessidade
  semântica real, não por estética;
- `break`, `continue` e `=>` executam a restauração antes da saída;
- loops restauram a cada iteração;
- aliases não podem escapar; view criada a partir de alias só pode viver se a
  origem sobreviver segundo as regras normais.

Validação obrigatória:

- caminho duplicado;
- caminho sobreposto (inclusive ancestral/descendente e índice dinâmico);
- uso do objeto original durante o split;
- alias escondido via view/loan;
- empréstimo escapando;
- restauração em todos os caminhos de controle.

Custo/benefício: não há paralelismo automático nem ganho de runtime agora. O
benefício é tornar explícita e verificável uma divisão que o checker já
consegue expressar como caminhos independentes. Se esse benefício não se
confirmar nos testes, a feature deve ser adiada.

---

## 11. Proposta de State Flow / Typestate

**Recomendação:** usar `mark` como argumento de tipo genérico; não criar uma
máquina de estados paralela.

```loi
// conceitual — sintaxe já representável, regras ainda não aceitas
mark Created { }
mark Connected { }
mark Authenticated { }

shape Connection<S> {
    socket: i32
    state: S
}

flow connect
    <-> c: Connection<Created>
    => Connection<Connected>
{
    // ler c, construir novo valor com state = Connected
    => connected
}

flow authenticate
    <-> c: Connection<Connected>
    => Connection<Authenticated>
{
    => authenticated
}
```

Por que funciona com o que existe:

- `mark` já é tipo escalar (`src/types.rs:22`);
- parser aceita mark como argumento de tipo;
- monomorfização gera `Connection__Created`, `Connection__Connected`, etc.
  (`src/monomorphize.rs:827-859`);
- parâmetro genérico não usado é aceito, mas o argumento permanece no nome
  mangled — então estados são distintos mesmo sem campo runtime;
- se o estado for um campo real, C já emite mark como `uint32_t`
  (`src/codegen.rs:2117-2141`).

Transições são flows ordinários com efeitos normais: o valor antigo é
consumido por `<->`/`give` e o novo é `PRODUCE`. Estado inválido é erro de
tipo, não erro de um subsistema separado. `choice` modela branches com estados
alternativos; `mark` modela estados finitos sem payload.

Branches e loops:

- estados diferentes em branches só são aceitos se convergirem para o mesmo
  tipo ou se o programa usar `choice` explícito;
- loop exige estado estabilizado entre iterações;
- uso após `give`/`<->` é bloqueado pelo ownership existente;
- duplo fechamento é bloqueado porque o valor foi movido;
- não se introduzem tipos dependentes completos nem inference mágica.

Ponto a decidir: estado como **parâmetro fantasma** (sem layout) ou como
**campo mark real** (com layout e visibilidade em C). Recomendação inicial:
parâmetro fantasma quando possível, para não mexer em ABI.

---

## 12. Proposta de Pipeline

**Recomendação:** `|>` só como açúcar sintático, depois dos contratos sólidos.

Forma mínima:

```loi
// conceitual — não implementado nesta fase
result <- data |> validate |> transform |> serialize
```

Expansão:

```loi
result <- serialize(transform(validate(data)))
```

Regras:

1. cada estágio é uma chamada normal a um flow unário `<-` com resultado;
2. direções dos argumentos continuam explícitas nos flow declarations;
3. os efeitos do estágio são os do `FlowSummary` — pipeline não cria efeito
   novo;
4. composite não é copiado implicitamente; se o estágio consumir ownership,
   a forma permitida deve ser uma chamada explícita com `<->`/`give`, não
   pipeline oculto;
5. views continuam emprestadas e não podem escapar;
6. erro em qualquer estágio não pode deixar ownership inválido, porque a
   expansão gera temporários com as mesmas regras de `let`/`=>`;
7. avaliação é rigorosamente left-to-right; short-circuit de `and`/`or`
   permanece intacto;
8. chamadas genéricas usam os mesmos type args e instâncias.

Placeholder `_`: **não implementar agora**. Se for aceito depois, sua semântica
deve ser: marcador de pipeline ligado ao temporário anterior, usável uma vez,
com direção explícita obrigatória e sem escape. Sem isso, ele teria múltiplas
interpretações — algo que a Loi evita.

---

## 13. Exemplos de sintaxe

Contrato inferido, com sintaxe atual:

```loi
flow update_physics
    <-> physics: Physics
    <- dt: f64
{
    physics.velocity.x += physics.acceleration.x * dt
    physics.position.x += physics.velocity.x * dt
}
```

Resumo esperado, conceitual:

```text
READ:  physics.acceleration.x, physics.velocity.x, physics.position.x, dt
WRITE: physics.velocity.x, physics.position.x
MOVE:  nenhum
BORROW: nenhum
```

Propagação para chamador:

```loi
flow update_world
    <-> world: World
    <- dt: f64
{
    update_physics(<-> world.physics, dt)
}
```

`update_world` deve modificar `world.physics`, não `world` inteiro. Chamadas
a `update_physics(<-> world.physics)` e `update_graphics(<-> world.graphics)`
são independentes; duas chamadas que escrevem `world.physics.velocity.x` não
são.

Exemplo de conflito que deve ser rejeitado:

```loi
step(<-> world.physics, <- world.physics.velocity.x)
// WRITE em world.physics.velocity.x sobrepõe READ no mesmo caminho
```

Exemplos de `split`, typestate e pipeline estão nas seções 10–12 e são
**propostas**, não sintaxe aceita.

---

## 14. Alterações necessárias no AST, checker e IR

### AST

- Contratos de efeitos: **nenhuma mudança obrigatória** na FASE 2; os efeitos
  são inferidos e ficam só no checker. Se, no futuro, externos puderem
  declarar contratos, aí sim `FunctionDecl` precisa de um campo opcional.
- `split`: preferir desugaring no parser para `Block`, `Let`, `CallArg` e
  caminhos `Member`/`Index`. Se os aliases precisarem sobreviver até o
  checker para diagnósticos precisos, adicionar **no máximo** um nó
  `StmtKind::Split` com `bindings: Vec<(String, Expr)>` e removê-lo antes da
  IR.
- `pipeline`: desugaring no parser/postfix para chamadas aninhadas; sem nó AST
  novo.
- typestate: nenhuma mudança AST se usarmos generics + marks existentes.

### Checker

Criar um módulo novo, por exemplo `src/flow/`, com:

- `DataPath` e `DataPathSet`;
- `FlowEffect`, `FlowSummary`, `BorrowSet`;
- `OwnershipState` como facade sobre estado atual;
- `RegionInfo` como facade sobre regiões atuais;
- um único visitante de efeitos substituindo os scanners duplicados.

`Checker` passa a consultar `FlowSummary` via acessores compatíveis; os mapas
antigos podem permanecer como aliases temporários até os testes estabilizarem.
Objetivo: **reduzir** a lógica de efeitos espalhada em `checker.rs`, não
aumentá-la.

### IR/backends

- IR não muda: ela não carrega semântica de direção/ownership hoje.
- `split` e pipeline são expandidos antes do checker e do monomorphizer;
  backends veem chamadas e lugares comuns.
- typestate é resolvido na monomorfização; backends veem tipos concretos.
- C11 continua sem alteração de ABI, exceto se um estado fantasma for
  transformado em campo real — decisão explícita.
- LLVM deve rejeitar explicitamente qualquer construção cuja semântica não
  consiga representar; em especial, não pode aceitar `->`/`<->` por valor
  fingindo paridade.

---

## 15. Testes necessários

Todos failing-first, sem `#[ignore]`, sem atualização em massa de goldens.

### A. Flow contracts

- leitura simples: `READ` propagado;
- escrita simples: `WRITE` em campo específico;
- inicialização: `->` e `alloc`;
- consumo: `give` e `<->` que move;
- propagação: flow A chama B e herda efeitos;
- chamada externa: conservador, whole-value;
- chamadas aninhadas: composição de caminhos;
- efeitos desconhecidos: `unknown_effects` nunca vira "nenhum efeito".

### B. Path analysis

- campos independentes;
- campos sobrepostos;
- campos aninhados;
- índices constantes disjuntos;
- índices dinâmicos sobrepostos;
- arrays/slices;
- múltiplos parâmetros;
- `UnknownTail`.

### C. Split/Join

- split válido;
- caminho duplicado;
- caminho sobreposto;
- uso do original durante split;
- view escapando;
- restauração após bloco;
- `break`/`continue`/`=>` dentro do bloco;
- split dentro de loop.

### D. Pipeline

- tipos compatíveis/incompatíveis;
- transferência de ownership;
- views;
- resultados e regiões;
- mutação explícita;
- efeitos;
- ordem left-to-right;
- short-circuit;
- chamadas genéricas.

### E. State Flow

- transição válida;
- transição inválida;
- estado incorreto;
- uso após fechamento;
- branch com estados diferentes;
- loop;
- ownership;
- resultado.

### F. Data Flow Graph

- dependência de leitura;
- dependência de escrita;
- dependência de resultado;
- caminhos independentes;
- ciclos/recursão;
- ordem de avaliação;
- operações com efeito desconhecido.

### G. Regressões

Gate completo `./tools/ci.sh`; suítes atuais de ownership, views, guard,
regions, order, differential e LLVM continuam passando. Nenhum enfraquecimento
de diagnóstico existente.

---

## 16. Compatibilidade com C11 e LLVM

- C11 é a primeira implementação e o oráculo semântico.
- Contratos são resolvidos antes do codegen; nenhum efeito vira runtime token.
- `split`/pipeline são expandidos para AST comum antes da monomorfização.
- typestate é dissolvido pela monomorfização; se for campo real, C já tem
  representação para mark.
- LLVM deve:
  - aceitar apenas o que pode representar com a mesma semântica; ou
  - emitir diagnóstico explícito `LLVM backend does not support ...`.
- Não se aceita divergência silenciosa. Em particular, as lacunas atuais de
  `->`/`<->` e `clone` precisam ser tratadas como restrição explícita do
  subset LLVM antes de qualquer feature nova depender delas.
- Mangling existente `base__arg__arg` permanece; mudanças de ABI C exigem
  decisão separada.

---

## 17. Riscos de regressão

1. **Migração de resumos:** substituir mapas por `FlowSummary` pode mudar
   conservadorismo; fazer em etapas com acessores e gate completo.
2. **Path canonicalização:** pode alterar mensagens e spans; manter diagnósticos
   atuais como contrato até decisão explícita.
3. **Inicialização path-sensitive:** mudar `mark_written` é o risco maior;
   fazer só com testes failing-first e documentar mudança de semântica.
4. **Views/region:** generalize loans sem permitir escape novo; manter fallback
   conservador.
5. **Split:** alias de caminho pode confundir formatter, completion, LSP e
   diagnostic spans; definir esses contratos antes de aceitar sintaxe.
6. **Pipeline:** desugaring ingênuo pode alterar ordem de avaliação; preservar
   temporários e short-circuit.
7. **Typestate:** estado fantasma vs campo real muda ABI; decidir antes.
8. **LLVM:** aceitar uma construção sem suporte real criaria divergência
   silenciosa; preferir rejeição explícita.
9. **Performance:** fixpoint com `DataPath` estruturado pode crescer; widener
   para `UnknownTail` é obrigatório.
10. **Documentação:** `PROGRESS.md`, `DECISIONS.md` e `CONTRACTS.md` precisam
    refletir o estado aceito; nunca transformar proposta em regra silenciosa.

---

## 18. Plano de implementação por etapas

| Fase | Entrega | Gate |
|---|---|---|
| 1 — Design | Este documento; nenhuma mudança de compilador | revisão/aprovação |
| 2 — Flow Contracts | `DataPath`, `FlowEffect`, `FlowSummary`; unificar resumos existentes; inferir efeitos, sem sintaxe nova | ✅ primitivos + migração do checker + visitante único + sete efeitos + `PRODUCE`/`ESCAPE`/`unknown_effects` + 13 testes internos verdes; evidência em `AUDIT_DATA_FLOW.md` |
| 3 — Path Analysis | Generalizar sobreposição a todos os efeitos; tornar `DataPath` a representação tipada do checker; testes B | ✅ `may_overlap` compartilhado por `READ`/`WRITE`/`INIT`/`MOVE`/`BORROW`/`PRODUCE`/`ESCAPE`; composição aninhada corrigida; accessores tipados e estado transiente migrado; 3 testes failing-first (9 × E0599) e 4 testes de composição verdes; recursão/widening permanecem PROPOSTO |
| 4 — Split/Join | Só se houver benefício; desugaring + validação; testes C | gate completo + fmt/pretty/LSP |
| 5 — State Flow | mark como argumento genérico; transições via efeitos; testes E | gate completo + differential |
| 6 — Pipeline | Açúcar de chamadas unárias; sem `_`; testes D | gate completo + ordem/short-circuit |
| 7 — Data Flow Graph | Grafo somente para análise/consulta; sem otimização/reordenação | testes F |
| 8 — Integração | Atualizar docs canônicos, exemplos, contratos, CI | `./tools/ci.sh` |

Regras do plano:

- failing-first → correção mínima → regressão → documentação;
- uma mudança por vez;
- sem novas dependências;
- sem paralelização automática;
- sem "inferência mágica";
- sem mudança de ABI sem decisão explícita.

---

## Decisões aprovadas antes da FASE 2

1. Unificar os quatro fixpoints atuais em um único `FlowSummary` com
   acessores de compatibilidade — aprovado; a primeira etapa já aplicou a
   unificação durável no checker, preservando os contratos antigos.
2. `join` do split é implícito no fechamento do bloco (recomendado).
3. Typestate começa como argumento genérico `mark`, preferencialmente
   parâmetro fantasma; campo real só se houver necessidade semântica.
4. Pipeline começa sem placeholder `_` e apenas com estágios unários;
   estágios que consomem ownership exigem chamada explícita.
5. Índices dinâmicos permanecem conservadoramente sobrepostos; prova de
   `i != j` não entra nesta fase.
6. LLVM continua com rejeição explícita onde não há paridade; não será
   usado como backend de validação para as novas features.

---

## Status da FASE 2 (reabertura em 2026-11-24)

- **IMPLEMENTADO:** `DataPath`/`DataPathSet`, `FlowEffect`, `FlowSummary` e os
  acessores legados; um único mapa durável no checker; visitante único de scan;
  inferência e propagação de `READ`/`WRITE`/`INIT`/`MOVE`/`BORROW`/`PRODUCE`/
  `ESCAPE`; `unknown_effects` conservador; composição de caminhos aninhados via
  `append_relative_path`.
- **IMPLEMENTADO PARCIALMENTE:** integração com ownership usa o resumo em
  consumo/leitura, mas ainda não há uma facade `OwnershipState` nem uso pleno de
  `PRODUCE`/`ESCAPE` em todas as decisões de região; o flag `unknown_effects`
  ainda não altera diagnósticos.
- **DOCUMENTADO:** contratos `CONTRACT-EFF-001`, `CONTRACT-EFF-002`,
  `CONTRACT-OWN-003` e `CONTRACT-VIEW-001` em `development/CONTRACTS.md`.
- **PROPOSTO:** recursão com limite/widening, split/join, typestate e pipeline.
- **NÃO SUPORTADO:** nenhuma sintaxe nova foi aceita; `split`, `join`, `|>` e
  State Flow declarativo continuam fora da linguagem.
- **LIMITAÇÃO CONHECIDA:** índice dinâmico é wildcard; taint de `protect` via
  `<->` permanece conservador (`CONTRACT-PROT-002`); LLVM mantém as lacunas
  pré-existentes de `->`/`<->` e `clone` profundo.

## Status da FASE 3 (representação tipada, 2026-11-25)

- **IMPLEMENTADO:** `DataPath::append_relative`/`with_root` e accessores tipados
  de `FlowSummary` (`moves_mut`, `reads_mut`, `escapes_mut`, `move_paths`,
  `read_paths_typed`, `escape_paths`, `view_returns_data_paths`,
  `view_out_data_paths`); fixpoint sem round-trip `DataPath → String →
  DataPath`; `ViewSource.path`, `ViewOutLoan.slot`, `VarInfo.moved`,
  `RegionTaint.path`, `ViewLoan.path/slot`, `ControlState.moved` e os conjuntos
  de `FunctionScan` usam `DataPath`; `index_data_path` conserva `IndexKey`.
- **IMPLEMENTADO PARCIALMENTE:** a eliminação completa de strings no checker;
  `LvalueInfo.path`, `place_path`, `append_relative_path` e mensagens continuam
  em fronteiras legadas, sem mudar diagnósticos nem APIs públicas.
- **DOCUMENTADO:** `CONTRACT-EFF-001`, `CONTRACT-OWN-003` e
  `CONTRACT-VIEW-001` em `development/CONTRACTS.md`; decisão de 2026-11-25 em
  `development/DECISIONS.md` e evidências em `AUDIT_DATA_FLOW.md`.
- **PROPOSTO:** facade `OwnershipState`, recursão com widener para
  `UnknownTail`, split/join, typestate e pipeline.
- **NÃO SUPORTADO:** nenhuma sintaxe nova; nenhuma mudança de `<-`, `->`,
  `<->`, `=>`, `give`, `clone`, `view`, `protect` ou `guard`.
- **LIMITAÇÃO CONHECIDA:** `unknown_effects` ainda não muda diagnósticos;
  `CONTRACT-PROT-002` e as lacunas LLVM pré-existentes seguem em aberto.

Evidência completa, failing-first e gates em `development/AUDIT_DATA_FLOW.md`;
estado corrente em `development/PROGRESS.md`.
