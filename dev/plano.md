# Plano de Implementação - Diagnósticos Estilo Rust (Fase A)

## 1. Mapa da Organização Atual

### Diagnósticos Atuais
- **Parser** (`src/parser.c`): `parser_error()` - imprime "Parse error at %d:%d: %s (got %s)"
- **Typechecker** (`src/typecheck.c`): `add_error()` - usa `fprintf(stderr, "Type error at %d:%d: ...")`
- **Preprocessor** (`src/preprocessor.c`): `fprintf(stderr, "error: line %zu: ...")`
- **Main** (`src/main.c`): vários `fprintf(stderr, "error: ...")` para erros de CLI, linkagem, etc.

### Inconsistências Identificadas
1. Formato inconsistente entre componentes
2. Sem códigos de erro (E0001, etc.)
3. Sem span (apenas linha:coluna inicial)
4. Sem trecho do código-fonte com `^^^`
5. Sem sugestões ("Você quis dizer...?")
6. Sem `help:` com correção provável
7. Para no primeiro erro (parser) ou poucos erros (typecheck)
7. Sem `loi --explain E0001`
8. Cores sempre ativas (não respeita NO_COLOR/TTY)

## 2. Riscos Identificados
- Mudança no formato de saída pode quebrar testes existentes
- Parser e typecheck têm lógica de recuperação de erro diferente
- Typechecker para em alguns erros fatais
- Parser não tem recuperação robusta (para no primeiro erro na maioria dos casos)
- Preprocessor para no primeiro erro

## 3. Ordem Proposta das Fases (Diagnósticos)

### Fase A1: Estrutura de Dados Unificada de Diagnóstico
- Criar `src/diagnostic.h/c` com:
  - `Diagnostic` struct: código (E0001+), severidade, mensagem, span (arquivo, linha_inicio, col_inicio, linha_fim, col_fim), notas, sugestões, help
  - `DiagnosticEmitter`: buffer de diagnósticos, impressão com cores/NO_COLOR
  - Códigos de erro categorizados (E0001-E0999 parser, E1000-E1999 typecheck, etc.)

### Fase A2: Parser - Recuperação e Formato
- Modificar `parser_error` para usar novo sistema
- Implementar recuperação de erro (panic mode → sync tokens: `;`, `}`, `)`, `else`, `}`)
- Adicionar span completo (início/fim do token)
- Adicionar "Você quis dizer?" para identificadores/tipos
- Adicionar `help:` com sugestão

### Fase A3: Typechecker - Múltiplos Erros e Formato
- Modificar `add_error` para usar novo sistema
- Permitir múltiplos erros (não parar no primeiro fatal)
- Adicionar span completo
- Adicionar notas e sugestões de correção

### Fase A4: Preprocessor e Main
- Migrar mensagens para novo formato
- Adicionar códigos de erro apropriados

### Fase A5: `--explain` e Cores
- Implementar `loi --explain E0001`
- Cores apenas em TTY (respeitar NO_COLOR)
- Testes comparam saída sem cor

## 3. Plano de Testes
- Cada novo código de erro: teste positivo + negativo com mensagem esperada
- Testes de recuperação de erro (múltiplos erros reportados)
- Testes de sugestão ("Você quis dizer...")
- Teste `--explain`
- Teste cores/NO_COLOR

---

## Ordem de Execução Proposta
1. **A1**: Criar `src/diagnostic.h/c` com estrutura base
2. **A2**: Migrar parser para novo sistema + recuperação
3. **A3**: Migrar typechecker para novo sistema + múltiplos erros
3. **A4**: Migrar preprocessor/main
4. **A5**: `--explain`, cores, testes