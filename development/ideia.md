# Loi — ideia de otimização: paralelismo automático de laços

Documento de design. Nada aqui está implementado. Os números citados vêm do benchmark que rodei (`benchmark/`); onde algo é estimativa, está marcado.

## 1. Resumo

**Ideia:** quando o compilador provar que cada iteração de um laço `for <-> p in xs` só mexe no seu próprio elemento, ele gera código que executa as iterações em várias threads, sem o programador pedir e sem anotação.

**Por que no Loi:** a linguagem já impede, por construção, os dois problemas que travam essa otimização em outras linguagens: dois nomes para o mesmo dado (aliasing) e ponteiros livres (`&`, `*T`). O checker já prova exclusividade; falta usar essa prova para ganhar velocidade.

**Por que essa e não SIMD ou layout:** no benchmark, trocar o layout para SoA deu 1,1× a 1,2×, e a versão vetorizada com SIMD chegou a 1,3×. Paralelismo tem teto proporcional ao número de núcleos. Esse teto é teórico: **não consegui medir paralelismo**, porque a máquina do teste tem 1 vCPU.

## 2. A ideia em um exemplo

O laço do `step` do benchmark:

```loi
sealed flow step
    <-> w: World
    <- dt: f64
{
    for <-> p in w.particles {
        if not p.alive { continue }
        p.vel.y += w.gravity.y * dt
        p.pos.y += p.vel.y * dt
        if p.pos.y < 0 or p.pos.y > w.bounds.y {
            p.pos.y = clamp(p.pos.y, 0, w.bounds.y)
            p.vel.y = -p.vel.y
        }
    }
}
```

Cada partícula é atualizada sem olhar para as outras. O corpo lê `w.gravity` e `w.bounds` (só leitura) e escreve apenas em `p`. Logo, a iteração 5 e a iteração 900.000 podem rodar ao mesmo tempo sem que uma enxergue a outra.

O C que o compilador gera hoje para esse laço tem esta forma (visto no `--emit-c`): um `for` com contador `size_t` sobre `slice.len`, que pega `&slice.data[i]` e usa o ponteiro dentro do corpo. Essa forma já é a que o OpenMP aceita como laço paralelizável.

## 3. Por que o Loi consegue e o C não

Para paralelizar com segurança, o compilador precisa saber que nenhuma iteração escreve em algo que outra iteração lê ou escreve. Em C isso é praticamente indecidível, porque:

- dois ponteiros podem apontar para o mesmo lugar (aliasing);
- uma função chamada no corpo pode escrever em qualquer coisa;
- qualquer ponteiro pode ter sido guardado em outro lugar.

No Loi, as regras da linguagem já eliminam essas dúvidas:

| Garantia da linguagem | Efeito para o paralelismo |
|----------------------|---------------------------|
| Um dono e um nome vivo por valor | Não há dois caminhos para o mesmo dado |
| Sem `&`, sem `*T` (spec 5.2) | Ninguém guarda o endereço de `p` para usar depois |
| `<->` é exclusivo na chamada | O checker já sabe que `w` e `p` não se sobrepõem |
| `<-` só lê, não guarda nem devolve | Uma função com parâmetros `<-` não deixa vazar acesso |
| Direções obrigatórias na assinatura | Dá para saber o que cada função escreve sem olhar o corpo |

O último ponto é o mais valioso: como cada `flow` declara `<-`, `->` e `<->`, a análise interprocedural fica simples. Uma função só escreve nos parâmetros marcados `->` ou `<->`.

## 4. O que foi medido e o que não foi

**Medido** (VM com 1 vCPU, GCC 13.3, melhor de 9 rodadas, 200 milhões de atualizações):

| Versão | 1M partículas × 200 | 10 mil × 20.000 |
|--------|---------------------|-----------------|
| Loi → C `-O2` | 0,450 s | 0,364 s |
| C à mão, AoS `-O2` | 0,461 s | 0,360 s |
| C à mão, SoA `-O2` | 0,385 s | 0,335 s |
| C à mão, SoA com SIMD | 0,337 s | 0,280 s |

- O C gerado pelo Loi empata com o C à mão: não há custo escondido no backend.
- `restrict` adicionado ao C gerado não mudou nada neste laço.
- SIMD e layout somados deram no máximo 1,3×.

**Não medido:**

- Qualquer ganho por usar várias threads. Com 1 vCPU o teste não é possível.
- O custo de acordar threads a cada chamada de `step` (200 chamadas no benchmark).
- Se o laço é limitado por largura de banda de memória. Com 48 MB de dados e 8 núcleos competindo pelo mesmo barramento, o ganho real pode ficar bem abaixo do número de núcleos. **Essa é a pergunta que o primeiro experimento precisa responder.**

## 5. Quando um laço é seguro de paralelizar

O corpo do laço precisa passar em todas estas regras. Qualquer violação mantém o laço sequencial, sem erro para o programador (a otimização é silenciosa e opcional).

| No corpo do laço | Pode? | Motivo |
|------------------|-------|--------|
| Escrever em `p` e nos campos de `p` | Sim | Cada iteração tem o seu |
| Ler outros campos de `w` (`w.gravity`, `w.bounds`) | Sim | Só leitura, desde que ninguém escreva neles |
| Escrever em outro campo de `w` (`w.tick += 1`) | **Não** | Corrida de dados entre iterações |
| Ler o elemento vizinho (`xs[i + 1]`) | **Não** | Outra iteração pode estar escrevendo nele |
| Escrever em variável declarada fora do laço | **Não** | Exceto como redução (seção 6) |
| `alloc` | **Não** | A arena global não é thread-safe |
| `protect` dentro do corpo | **Não** | Usa a mesma arena global |
| `break` ou `=>` (retorno) no corpo | **Não** | OpenMP não permite sair de um laço paralelo. `continue` é permitido |
| `panic` ou erro de índice em runtime | Cuidado | Aborta de dentro de uma thread; a mensagem pode variar entre execuções |
| Chamar um `flow` | Depende | Só se o `flow` também passar nestas regras, usando as direções da assinatura |

### Laços por índice

`for i in 0..n { ps[i] = ... }` (como o `world_new`) é um caso mais difícil: a análise precisa provar que cada iteração só escreve em `ps[i]` e em nenhum outro índice. Isso dá para uma segunda fase. A primeira fase deve cobrir só os laços `for <- p in xs` e `for <-> p in xs`, que já entregam a exclusividade por construção.

### Laços aninhados

Paralelizar só o laço mais externo que passar nas regras. Paralelizar os dois níveis cria threads demais e piora o desempenho.

## 6. Reduções e o problema do determinismo

Um laço como este soma em uma variável de fora:

```loi
mut m: f64 <- 0
for <- p in w.particles { m += p.mass }
```

Cada thread somaria o seu pedaço, e no fim os parciais se juntam. O problema: **soma de ponto flutuante não é associativa**. Somar em ordem diferente pode dar um resultado diferente nos últimos bits. Hoje o Loi dá o mesmo resultado em toda execução; com redução paralela isso deixa de valer.

| Tipo | Redução paralela é segura? |
|------|---------------------------|
| Inteiros com `+`, `*`, `and`, `or`, mínimo, máximo | Sim, o resultado é idêntico |
| `f64` com `+` ou `*` | Muda os últimos bits, e pode mudar entre execuções com número de threads diferente |
| `f64` com mínimo ou máximo | Sim (sem NaN) |

Decisão de design a registrar na spec: o que a linguagem promete?

1. **Determinismo sempre:** reduções de `f64` nunca são paralelizadas automaticamente. Seguro e simples, mas perde um caso comum.
2. **Determinismo por padrão, velocidade opcional:** uma anotação explícita (por exemplo, um atributo de bloco) libera a redução de `f64` fora de ordem.
3. **Sem promessa de determinismo em `f64`:** o mais rápido, mas complica testes e depuração.

Minha recomendação para a primeira versão é a opção 1. Laços **sem** redução, como o `step`, já são determinísticos por elemento: o resultado é idêntico bit a bit ao sequencial, porque cada elemento é calculado com as mesmas operações na mesma ordem.

## 7. Como o compilador geraria o código

Para o primeiro experimento não precisa de runtime próprio. O C gerado ganha uma linha antes do laço que passou na análise, e o `build` passa a compilar com suporte a OpenMP:

- A linha é um `#pragma omp parallel for` antes do `for` que já existe no C gerado.
- Uma cláusula `if (len > LIMITE)` impede paralelizar laços pequenos, onde criar a divisão de trabalho custa mais que o trabalho.
- O `loi build` precisa passar `-fopenmp` ao compilador C (já existe `--cc-flag` para isso, e o compilador pode passar sozinho quando gerar o pragma).
- Compiladores sem OpenMP ignoram o pragma, então o programa continua correto, só sequencial.

**Escalonamento:** usar divisão estática em blocos grandes. Com elementos de 48 bytes, duas threads trabalhando em elementos vizinhos nas fronteiras dos blocos dividem uma linha de cache (*false sharing*), e blocos grandes limitam isso às fronteiras.

**Chamada repetida:** o `step` roda 200 vezes dentro de um laço sequencial. O OpenMP reaproveita as threads entre chamadas, então o custo por chamada tende a ser pequeno (da ordem de microsegundos, estimativa a medir). Com 1 milhão de partículas isso é desprezível; com 10 mil pode não ser, e por isso o limite de tamanho.

## 8. Plano em fases

Cada fase termina com algo que dá para testar. A fase 2 só começa se a fase 1 mostrar que vale.

**Fase 1 — só análise, sem mexer no backend**
- Adicionar um modo de diagnóstico (por exemplo `loi check --parallel`) que lista, para cada laço `for <- p in xs` e `for <-> p in xs`, se é seguro e por que não é quando rejeita.
- Critério de pronto: o `step` do `particles.loi` aparece como seguro; um laço com `w.tick += 1` aparece como rejeitado, com o motivo.
- Como não gera código, não há risco de quebrar nada.

**Fase 2 — experimento de desempenho (decide se continua)**
- Gerar o `#pragma omp parallel for` nos laços aprovados.
- Rodar o `benchmark/bench.loi` numa máquina com vários núcleos e comparar com a versão sequencial.
- Checar que o checksum é idêntico.
- **Critério de parada:** se com 8 núcleos o ganho for menor que cerca de 3×, o gargalo é a memória e o resto do plano não compensa.

**Fase 3 — reduções inteiras e laços por índice**
- Reduções de inteiros e de mínimo/máximo.
- Laços `for i in a..b` onde cada iteração só escreve em `xs[i]`.

**Fase 4 — decisão sobre `f64` e controle do usuário**
- Decidir a promessa de determinismo (seção 6).
- Uma forma de desligar a otimização: uma opção de linha de comando e/ou um atributo em um laço específico.

## 9. Riscos

| Risco | Efeito | Mitigação |
|-------|--------|-----------|
| Furo no checker de aliasing | Corrida de dados: resultados errados e **não repetíveis**, o pior tipo de bug | Os furos do relatório de análise precisam estar fechados antes. Em especial o literal de slice que escapa do bloco |
| Arena global sem proteção entre threads | Corrupção de memória se um `alloc` entrar num laço paralelo | A regra de rejeitar `alloc` e `protect` no corpo (seção 5) |
| Paralelizar laço pequeno | Programa fica mais lento | Cláusula de tamanho mínimo e medição |
| Gargalo de memória | Ganho bem menor que o número de núcleos | Fase 2 mede antes de investir mais |
| Depuração de programas paralelos | Erros que dependem de tempo | Flag para desligar tudo e comparar com a versão sequencial |
| Erros de runtime (índice fora do limite) dentro do paralelo | Mensagem de posição pode variar de uma execução para outra | Aceitar (continua abortando) ou rodar sequencial em modo `--debug` |

O primeiro risco é o mais sério. Essa otimização **aumenta o preço de qualquer furo no checker**: hoje um furo causa comportamento indefinido; com paralelismo, um furo vira corrida de dados intermitente. Por isso a ordem recomendada é fechar os furos primeiro.

## 10. Perguntas de design em aberto

1. **Modo debug e modo release:** em `--debug` os laços ficam sempre sequenciais, para facilitar a depuração?
2. **Controle do usuário:** como o programador desliga ou força o paralelismo em um laço? Opção global, atributo no laço, ou ambos?
3. **Reduções de `f64`:** qual das três opções da seção 6?
4. **Chamadas de `flow` dentro do corpo:** a análise segue a chamada até o corpo da função, ou confia apenas nas direções da assinatura? Seguir é mais preciso e mais trabalhoso.
5. **Arena por thread:** no futuro, permitir `alloc` dentro de laço paralelo dando uma arena para cada thread? Isso resolve a regra mais restritiva, mas muda o runtime.
6. **Limite de tamanho:** valor fixo, ou calculado pelo custo estimado do corpo do laço?

## 11. Como testar

- **Correção:** o checksum do `benchmark/bench.loi` deve ser idêntico (código de saída 124 no tamanho grande) com e sem paralelismo. Como laços sem redução são determinísticos por elemento, a igualdade deve ser exata, não aproximada.
- **Casos que devem ser rejeitados:** um teste para cada linha "Não" da tabela da seção 5 (`w.tick += 1`, `xs[i + 1]`, `alloc` no corpo, `break`). A análise deve recusar todos.
- **Casos que devem ser aceitos:** o `step` do `particles.loi` e do benchmark.
- **Robustez:** rodar o programa paralelo muitas vezes e comparar o resultado. Uma corrida de dados costuma aparecer como resultado diferente de vez em quando.
- **Desempenho:** medir com 1, 2, 4 e 8 núcleos para ver a curva de escala, e com tamanhos pequenos e grandes para calibrar o limite de tamanho. Se possível, rodar também um verificador de corridas (ThreadSanitizer do GCC/Clang) no C gerado.

## 12. Conclusão

Paralelismo automático é a otimização com o maior teto e a que mais se apoia no que o Loi já tem de diferente: exclusividade provada pelo compilador. O risco é concentrado em um ponto (a confiabilidade do checker), e o plano é construído para falhar rápido: a fase 1 não toca no backend, e a fase 2 tem um critério de parada claro. Antes de qualquer coisa, **meça em uma máquina com vários núcleos**: se o laço for limitado pela memória, a conclusão muda.

