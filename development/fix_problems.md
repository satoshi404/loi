# Loi — análise do repositório: problemas encontrados

Análise feita em 2026-11-08 sobre o zip `loi-main` (Rust, edition 2024, sem dependências externas).

## 0. Resumo

| # | Problema | Gravidade | Onde mexer |
|---|----------|-----------|------------|
| 1 | Literal de slice escapa do bloco e vira ponteiro pendurado | **Alta** (fura a garantia de memória) | `src/checker.rs` |
| 2 | Overflow de inteiro com sinal não tem semântica definida (UB em C) | Média/alta | `src/codegen.rs` + decisão de design |
| 3 | Divisão por zero não constante derruba o programa com SIGFPE | Média | `src/codegen.rs` + decisão de design |
| 4 | Backend ASM gera funções vazias | Média (é um rascunho, mas parece pronto) | `src/asm.rs`, `src/ir.rs` |
| 5 | Arena fixa de 64 MiB que não cresce | Baixa/média | `src/codegen.rs` (runtime em C emitido) |
| 6 | Documentação desatualizada ou contraditória | Média (atrapalha quem vai aprender o projeto) | raiz, `development/`, `PROGRESS.md` |
| 7 | `checker.rs` monolítico (3.046 linhas) | Baixa, mas pesa na hora de aprender | `src/checker.rs` |

O que **funcionou** bem nos testes: uso depois de `give`, escape de `alloc` para fora do `protect`, mesmo valor passado duas vezes como `<->`, cópia implícita de composto (`ys = xs`), e índice fora do limite (aborta com linha e coluna). Isso está na seção 8.

---

## 1. Como reproduzi

- `cargo` do apt (1.75) não compila o projeto: precisa de Rust 1.85 ou mais novo por causa da edition 2024. Usei o 1.89 (`cargo-1.89` / `rustc-1.89`).
- `cargo build` compila sem warnings relevantes.
- `cargo test --all-targets`: **165 testes passam, 0 falham** (bate com o `PROGRESS.md`).
- Todos os exemplos de `examples/` passam em `loi check`, e `loi build` gera executável. `tictactoe` joga a partida inteira e termina com "X wins!".
- Para cada problema abaixo coloquei o programa `.loi` mínimo que usei. Para reproduzir: `loi check arquivo.loi`, e quando indicado `loi build arquivo.loi -o saida && ./saida`.

---

## 2. Problema 1 — literal de slice escapa do bloco (memória)

### O que acontece

Um literal de slice (`[7, 8, 9]`) é alocado em **armazenamento automático** do bloco onde aparece. Se você atribui esse literal a uma variável declarada fora do bloco, a variável passa a apontar para memória que já saiu de escopo.

O checker só rejeita o caso em que o literal é **retornado diretamente** (`=> [1, 2, 3]`). Ele não rejeita o caso de guardar o literal numa variável de escopo mais externo.

### Reprodução (checker aceita)

```loi
open flow main => i32 {
    mut outer: [i32]
    if true {
        outer = [7, 8, 9]
    } else {
        outer = [1, 2, 3]
    }
    mut i: u64 <- 1
    => outer[i]
}
```

- `loi check` → **aceita**.
- `loi build` e executar → retornou `8`. Parece correto, mas é sorte: o C gerado usa um compound literal que morre no fim do bloco, então ler depois é comportamento indefinido. Com outro compilador, outro nível de otimização ou mais chamadas no meio, pode dar outro valor ou lixo.

### Por que importa

A promessa central da linguagem é memória segura sem borrow checker e sem GC. Este caso é justamente um ponteiro que sobrevive ao dono. O `PROGRESS.md` já lista essa limitação em "Known Bugs", mas ela fura a garantia que o resto do checker se esforça para dar.

### Detalhe que atrapalha o teste

Sem o `else`, o checker recusa com *"use of uninitialized variable 'outer'"*. Isso acontece pela análise de atribuição definida, não pelo escape do literal. Ou seja, o buraco só aparece quando todos os caminhos atribuem. Um bloco solto `{ ... }` também não parseia (`expected expression, found '{'`), então não dá para testar escopo com bloco avulso.

### Para você investigar (sem solução pronta)

- No `checker.rs` já existem peças parecidas: a função `contains_slice_literal` e o rastreamento de "taint" por região (`RegionTaint`), usado para `alloc` dentro de `protect`. A pergunta é: o literal de slice poderia ser tratado como uma "região" cujo tempo de vida é o bloco léxico onde nasceu?
- Perguntas de design para decidir antes de codar:
  1. Atribuir um literal a uma variável de escopo mais externo deve ser erro, ou o compilador deve promover o literal a armazenamento mais longo?
  2. Passar o literal como argumento `<-` para uma função é seguro (ele vive durante a chamada)? E guardar o literal dentro de um `shape` local?
  3. A regra vale para string literal? (Não deveria: strings são estáticas.)

---

## 3. Problema 2 — overflow de inteiro com sinal

### O que acontece

O C gerado usa aritmética direta sobre `int32_t` (vi `loi_v_a += 1;` no `--emit-c`). Em C, overflow de inteiro **com sinal** é comportamento indefinido: o otimizador pode assumir que nunca acontece.

```loi
open flow main => i32 {
    mut a: i32 <- 2147483647
    a += 1
    => a
}
```

- `loi check` → aceita. O gerado contém `int32_t loi_v_a = 2147483647; loi_v_a += 1;`. Não executei este programa; o ponto é que a linguagem não diz o que `a` vale depois.

### Agravantes já documentados

- O `PROGRESS.md` reconhece que o C aplica promoção de inteiros: `i8 + i8` é avaliado como `int` em C, enquanto o checker enxerga `i8`. Assim, o checker e o C podem discordar sobre o resultado de uma subexpressão que não é armazenada.
- A constant folding só serve para diagnósticos. O C gerado continua com a expressão original.
- O cast `as` trunca sem checagem de faixa.

### Decisão de design que falta

A spec não define a semântica de overflow. Opções comuns, cada uma com um custo:

| Opção | Efeito |
|-------|--------|
| Wrap definido (complemento de dois) | Previsível, barato; esconde bugs |
| Trap (aborta com posição, como já faz o índice) | Seguro; custa uma checagem por operação |
| Operadores explícitos (`+%` wrap, `+?` checado) | Máximo controle; mais sintaxe |

Como a linguagem já aborta com posição no índice fora do limite, trap em debug e wrap em release é um caminho que combina com `--debug` / `--release` já existentes. Mas isso é uma escolha sua, e deve ir para o `LOI_SPEC.md`.

---

## 4. Problema 3 — divisão por zero em tempo de execução

### O que acontece

O checker rejeita divisão por zero **constante**, mas não tem como saber se uma variável é zero. Aí a verificação sobra para o C, que em x86 gera uma exceção de hardware.

```loi
open flow main => i32 {
    mut z: i32 <- 0
    => 10 / z
}
```

- `loi check` → aceita.
- Executável sai com código **136** e a mensagem `Floating point exception` (SIGFPE). Sem posição de código-fonte, ao contrário do erro de índice.

### Inconsistência de comportamento

| Erro de runtime | O que acontece |
|-----------------|----------------|
| Índice fora do limite | Mensagem `loi: slice index out of bounds at 6:9` e `exit 1` |
| Arena esgotada | Mensagem com posição e `exit 1` |
| Divisão por zero | SIGFPE, sem posição, `exit 136` |
| Overflow | Indefinido |

Se a linguagem se compromete com "erros de runtime reportam a posição", a divisão por zero e o overflow precisam seguir o mesmo padrão. Também `%` com zero e a divisão `INT_MIN / -1` (que também é UB em C) entram nessa conta.

---

## 5. Problema 4 — backend ASM gera funções vazias

### O que acontece

`loi gen examples/particles.loi --emit-asm saida.s` produz um arquivo cujas funções não têm corpo. As primeiras linhas:

```asm
.globl	_world_new
	.type	world_new, @function
world_new:
	pushq	%rbp
	movq	%rsp, %rbp
	subq	$256, %rsp
entry:
		ret
	movq	%rbp, %rsp
	popq	%rbp
	ret
	.size	world_new, .-_world_new
```

Problemas visíveis:

1. O corpo de `world_new` (que aloca e preenche partículas) virou só um `ret`. O mesmo vale para `clamp`.
2. O `ret` fica **antes** do epílogo, então ele sai da função sem restaurar o frame.
3. Os nomes não batem: a label é `world_new`, mas `.globl` e `.size` usam `_world_new`. Isso é convenção de macOS misturada com `@function` de ELF/Linux.
4. A reserva fixa `subq $256, %rsp` não vem de nenhuma análise.

Não tentei montar e linkar esse `.s`. Pelo que está no arquivo, não é um backend funcional.

### Contexto

O `PROGRESS.md` classifica o ASM como "Working (draft)" e diz que usa "lowerings simplificados". Isso é honesto, mas a linha de verificação (`writes ... x86-64 assembly (81 lines)`) dá a impressão de que a etapa é validada. Gerar 81 linhas não é a mesma coisa que gerar código que roda.

### Sugestão

Enquanto o backend não funciona, é melhor a flag `--emit-asm` falhar com um erro claro (*"backend ASM ainda não suporta ..."*) do que escrever um arquivo que parece válido. O próprio `PROGRESS.md` já tem esse próximo passo ("or mark those paths explicitly unsupported in the CLI").

---

## 6. Problema 5 — arena fixa de 64 MiB

### O que acontece

`protect` vira uma arena bump global de 64 MiB que não cresce. Se o total alocado passa disso (sem sair do `protect` e liberar), o programa aborta.

```loi
open flow main => i32 {
    protect {
        for i in 0..100000 {
            mut ys: [i64]
            alloc(-> ys, i64, 1000)
            ys[0] = 1
        }
    }
    => 0
}
```

- Aloca ~800 MB no total. Resultado: `loi: protect arena exhausted at 9:13`, `exit 1`. A mensagem é boa; o limite é que é fixo.
- O mesmo programa com o `protect` **dentro** do laço (alocar 8 KB por iteração e liberar a cada volta) rodou 1 milhão de iterações sem problema. Ou seja, o reset por bloco funciona; o problema é só o teto.

### Detalhes a considerar

- A flag `--arena-size BYTES` existe no `build`, então o teto é configurável em compilação, mas não cresce em execução.
- A arena é global e não é thread-safe (documentado).
- Para jogos (os projetos que você quer fazer), um teto fixo por frame costuma ser aceitável; o importante é decidir e documentar o padrão.

---

## 7. Problema 6 — documentação e processo

### 7.1 Não há README na raiz

O zip só tem `website/README.md`. Quem abre o repositório no GitHub não encontra uma página inicial com "o que é, como compilar, exemplo".

### 7.2 `AGENTS.md` aponta para o lugar errado

`development/AGENTS.md` manda ler `LOI_SPEC.md` primeiro, mas o arquivo está em `development/LOI_SPEC.md`. Fora desse diretório o caminho não existe. Além disso, a regra 4 ainda cita o vocabulário antigo (`interface` no lugar de `struct`, `visible` / `hidden`), que a seção 5.10 da spec declara removido.

### 7.3 Vocabulário antigo ainda espalhado no `PROGRESS.md`

Em "Known Bugs / Limitations" aparecem `move`, `interface`, `hidden`, `let x = <constant>`, `[]u8`, `inout`/`out`. Tudo isso foi trocado por `give`, `shape`, `sealed`, `<-`/`->`/`<->`. Quem lê o arquivo para entender o estado atual precisa traduzir mentalmente.

### 7.4 Tamanho dos arquivos contra as próprias regras

| Arquivo | Tamanho | Regra violada |
|---------|---------|---------------|
| `PROGRESS.md` | ~60 KB | "Keep `PROGRESS.md` short. Summarize old entries" |
| `DECISIONS.md` | ~40 KB | — |

A seção "Verification" do PROGRESS tem mais de 120 linhas de comandos já executados, incluindo passos de quando a sintaxe era a antiga ("the retained legacy-surface reference still checks"). Isso é histórico, não estado. A regra "uma informação, um lugar" também é quebrada: os limites do backend aparecem em "Component Status", "Known Bugs" e "Open Design Questions".

### 7.5 Roadmap da spec todo desmarcado

Na seção 6 do `LOI_SPEC.md` **todos** os itens estão como `- [ ]`, inclusive os das Fases 0, 1 e 2 que estão implementadas e testadas. Quem lê a spec acha que nada foi feito.

### 7.6 Spec e implementação divergem em `<-`

A spec diz que `<-` dá acesso de leitura "pelo próprio endereço, sem cópia". O backend C passa composto **por valor** (o `PROGRESS.md` documenta isso como decisão temporária). Funciona hoje porque o checker proíbe guardar ou devolver emprestados e o programa é single-thread. Mas a spec ainda descreve outra coisa, e para tipos grandes é uma cópia real.

### 7.7 Duplicação dos exemplos

`website/examples/` é cópia de `examples/` e precisa ser mantida igual à mão (`diff -r examples website/examples`). Um script ou um symlink resolve.

### 7.8 Playground

`website/server.mjs` executa programas Loi arbitrários com as permissões do processo. Está documentado como ferramenta local (`127.0.0.1`), e é o que deve continuar sendo.

---

## 8. Problema 7 — `checker.rs` monolítico

- `src/checker.rs`: **3.046 linhas, 114 KB**, com um único `impl Checker` de ~2.500 linhas.
- Ele concentra: resolução de nomes, tipos, direções, liveness de `give`, atribuição definida, retorno definido, escape de `protect`, exclusividade de `<->`, constant folding e vários passes auxiliares.
- Funciona e tem 71 testes de integração, mas para entender, depurar ou estender (por exemplo, o Problema 1) você precisa navegar um arquivo enorme.

### Sugestão de organização

Sem mudar comportamento, separar por responsabilidade: `names/types`, `ownership` (give/aliasing), `regions` (protect/alloc), `consts` (folding). Os 71 testes servem de rede de segurança para a refatoração. Isso também te obriga a ler cada parte, o que combina com seu objetivo de entender o compilador.

---

## 9. O que funcionou (testado)

Todos rejeitados corretamente pelo `loi check`, com posição de linha e coluna:

| Caso | Mensagem |
|------|----------|
| Escrever em `a` depois de `b <- give(a)` | `cannot write to moved value 'a'` |
| `keep = give(a)` com `a` alocado dentro de `protect` | `value allocated inside 'protect' escapes the block` |
| `f(<-> v, <-> v)` | `cannot pass 'v' as '<->' ... because it is also used by another argument` |
| `f(<-> xs, <-> xs)` com slices | idem |
| `ys = xs` (dois nomes para o mesmo slice) | `cannot copy [i32] implicitly; use give(...) or clone(...)` |

Em tempo de execução:

| Caso | Resultado |
|------|-----------|
| `xs[10]` com `len = 4` | `loi: slice index out of bounds at 6:9`, `exit 1` |
| 1 milhão de `protect` com `alloc` dentro de laço | roda sem crescer a memória |

Ou seja: a parte de ownership e `protect` está bem implementada para os caminhos comuns. Os furos estão nas bordas (literais, aritmética) e no acabamento (docs, ASM).

---

## 10. Ordem sugerida para atacar

1. **Problema 1** (literal escapa) — pequeno, delimitado, e te ensina como o checker modela escopo e região.
2. **Decidir overflow e divisão por zero** (Problemas 2 e 3) — primeiro uma decisão escrita no `LOI_SPEC.md`, depois o ajuste no `codegen.rs`. Os dois podem usar o mesmo mecanismo de "abortar com posição" que já existe para índice.
3. **Fazer `--emit-asm` falhar com mensagem honesta** (Problema 4), até o backend ser real.
4. **Limpar a documentação** (seção 7): README na raiz, caminho do `AGENTS.md`, roadmap marcado, `PROGRESS.md` reduzido a estado atual.
5. **Dividir o `checker.rs`** (Problema 7), quando já tiver entendido bem as partes.

Para cada item, escreva primeiro o teste que falha (os programas desta análise servem de base), depois mexa no compilador.
