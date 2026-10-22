# Decisões de Implementação - Diagnósticos Estilo Rust (Fase A)

## Decisão 1: Formato de Código de Erro
**Escolha**: `E` + 4 dígitos (E0001-E9999)
- E0001-E0999: Parser
- E1000-E1999: Typecheck
- E2000-E2999: Codegen
- E3000-E3999: Preprocessor
- E4000-E4999: Runtime/Linker
- E5000-E5999: CLI/Flags

**Justificativa**: Formato Rust-like, expansível, categorizado por módulo.

---

## Decisão 2: Estrutura de Span
**Escolha**: `struct Span { const char* file; int line_start, col_start, line_end, col_end; }`
- Linha/coluna 1-based (como Rust)
- `line_end`/`col_end` opcionais (0 = não disponível)
- Para tokens: início = posição do token, fim = fim do token
- Para expressões: início do primeiro token, fim do último token

---

## Decisão 3: Estrutura de Diagnóstico
```c
typedef enum { DIAG_ERROR, DIAG_WARNING, DIAG_NOTE, DIAG_HELP } DiagSeverity;

typedef struct {
    const char* code;        // "E0001"
    DiagSeverity severity;
    const char* message;     // Mensagem principal
    Span span;               // Localização primária
    Span* notes;             // Array de spans relacionados (notas)
    size_t note_count;
    const char* suggestion;  // "Você quis dizer `foo`?"
    const char* help;        // "help: converta com `as i32`"
} Diagnostic;
```

---

## Decisão 4: Códigos de Erro por Módulo
| Range | Módulo | Exemplos |
|-------|--------|----------|
| E0001-E0999 | Parser | E0001: expected ';', E0002: expected '}', E0003: expected expression |
| E1000-E1999 | Typecheck | E1001: type mismatch, E1002: unknown type, E1003: missing return |
| E2000-E2999 | Codegen | E2001: undefined symbol, E2002: stack overflow |
| E3000-E3999 | Preprocessor | E3001: include not found, E3002: macro limit |
| E4000-E4999 | Runtime/Link | E4001: link failed, E4002: undefined symbol |
| E5000-E5999 | CLI | E5001: unknown flag, E5002: missing argument |

---

## Decisão 4: Recuperação de Erro no Parser
**Estratégia**: Panic mode → sincronização em tokens de sincronização
- Tokens de sincronização: `;`, `}`, `)`, `else`, `elif`, `endif`, `endwhile`, `endfor`, `endfor`, `else`, `elif`
- Ao encontrar erro: consumir tokens até encontrar sync token ou EOF
- Continuar parsing para reportar mais erros
- Limite: máximo 50 erros por arquivo

---

## Decisão 5: "Você quis dizer?" (Did you mean?)
**Algoritmo**: Levenshtein distance ≤ 2 (ou ≤ 30% do comprimento)
**Escopo**: Identificadores no mesmo scope, tipos conhecidos, campos de struct, funções std.*
**Limite**: Máximo 3 sugestões
**Exemplo**: `std.pring_str` → "Você quis dizer `print_str`?"

---

## Decisão 5: Help/Correção Provável
**Padrões**:
- Tipo mismatch → `help: converta com \`as i32\``
- Missing return → `help: adicione \`return expr\` no final da função`
- Missing capability → `help: passe \`@cap(borrow Gfx)\` ou adicione \`!Gfx\` ao efeito`
- Unknown identifier → `help: você quis dizer \`print_str\`?`
- Missing semicolon → `help: adicione \`;\` no final da declaração`

---

## Decisão 6: Múltiplos Erros por Execução
- **Parser**: Coleta até 50 erros, para em EOF
- **Typecheck**: Continua após erros não-fatais, para em erros fatais (ex: tipo desconhecido em assinatura)
- **Codegen**: Para no primeiro erro (fatal)
- **Total**: Relatório final com contagem "X errors, Y warnings"

---

## Decisão 7: Formato de Saída (Estilo Rust)
```
error[E0001]: expected ';', found '}'
  --> tests/test.loi:10:25
   |
10 |     let x = 1 }
   |                   ^ expected ';', found '}'
   |
   help: adicione `;` antes de `}`
```

- Cores: vermelho (error), amarelo (warning), azul (note), verde (help)
- Apenas se stderr for TTY e não NO_COLOR
- Testes comparam saída sem cor (strip ANSI)

---

## Decisão 8: `loi --explain E0001`
```
loi --explain E0001

E0001: expected ';', found '}'
Esta erro ocorre quando o parser esperava um ';' mas encontrou outro token.
Em loi, declarações e expressões de statement devem terminar com ';'.

Exemplo errado:
    let x = 1 }
    
Exemplo correto:
    let x = 1;
```

---

## Decisão 9: Migração de Mensagens Existentes
| Atual | Novo Código | Novo Formato |
|-------|-------------|--------------|
| "Parse error at %d:%d: expected ';'" | E0001 | error[E0001]: expected ';' |
| "Type error at %d:%d: type mismatch" | E1001 | error[E1001]: type mismatch |
| "error: line %zu: cannot find included file" | E3001 | error[E3001]: cannot find included file |

Todas as mensagens existentes devem ser mapeadas para códigos E0001+.

---

## Decisão 10: Ordem de Implementação (Commits)
1. `diagnostic.h/c` - Estrutura base + emissor + cores
2. Parser: migrar `parser_error` + recuperação + span
3. Typecheck: `add_error` → novo formato + múltiplos erros
4. Preprocessor/Main: migrar mensagens
5. `--explain` + cores + testes
5. Documentação no `site/`