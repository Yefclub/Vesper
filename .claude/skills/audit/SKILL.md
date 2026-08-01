---
name: audit
description: Use quando pedir análise profunda de uma área do código — "analisa o módulo X", "audita a segurança de Y", "como está a performance de Z", "esse código tem problema?". Entrega achados com arquivo e linha, separados por severidade, sem alterar nada.
---

# Audit — auditoria focada

Análise profunda de um recorte do código. **Ler o código real antes de qualquer afirmação** — não responder de memória, não inferir comportamento por nome de arquivo.

Read-only: esta skill não edita, não cria issue, não abre PR.

## Entradas

- **Alvo**: módulo, camada ou concern (`segurança`, `performance`, `tipos`, `testes`)
- **Profundidade**: `rápida` (mapa + pontos quentes) ou `completa` (arquivo por arquivo)

## Fluxo

1. **Mapear o escopo** — listar os arquivos que pertencem ao alvo. Se for grande demais para a profundidade pedida, dizer isso **antes** de começar e propor o recorte.
2. **Ler.** Cada arquivo do escopo, inteiro. Este é o passo que a skill existe para forçar.
3. **Analisar pelo concern.**
4. **Reportar** com arquivo e linha.

## O que procurar

**Segurança**
- Rota protegida sem guarda de **autenticação** e sem guarda de **autorização** (autenticado ≠ autorizado — é o furo mais comum)
- Input sem validação na borda, inclusive o que "vem do próprio front"
- Concatenação de string em query; injeção em qualquer interpolador
- Saída não escapada renderizada como HTML
- Segredo em código, log, mensagem de erro ou resposta de API
- Endpoint caro ou abusável sem rate limit
- Dado de um usuário acessível trocando um id na URL

**Performance**
- Consulta em laço (N+1)
- Coluna filtrada ou ordenada com frequência sem índice
- Payload sem paginação crescendo com a base
- Re-render por assinatura ampla demais de estado
- Trabalho repetido que caberia em cache, e invalidação que não existe

**Tipos**
- Escape do sistema de tipos (`any`, cast forçado, `!`) sem justificativa
- Schema de validação divergente do tipo estático — os dois precisam ser a mesma verdade
- Fronteira (API, fila, storage) tipada por otimismo, sem validação em runtime

**Testes**
- Lógica de negócio sem teste
- Teste que verifica implementação em vez de comportamento — quebra em refatoração e passa com bug
- Mock que esconde o contrato real do que foi mockado

## Saída

```markdown
# Auditoria — <alvo> (<concern>)

## Escopo
<N arquivos lidos em <diretórios>>. Ficou de fora: <o que e por quê>

## Críticos — corrigir antes de mergear
- [<arquivo>:<linha>] <problema> → <como falha na prática> → <correção>

## Importantes
- [<arquivo>:<linha>] ...

## Menores
- [<arquivo>:<linha>] ...

## O que está bem feito
<citar de verdade; auditoria que só acha defeito perde credibilidade e vira ruído>
```

## Regras

- **Nunca afirmar que algo é seguro ou performático sem ter lido.** "Parece ok" não é achado.
- Todo achado cita arquivo, linha e o **modo de falha concreto**. Sem modo de falha, é opinião de estilo — corta.
- Separar defeito real de preferência estilística. Misturar os dois faz o leitor descartar os dois.
- Escopo maior que a profundidade pedida: dizer o que **não** foi coberto. Silêncio sobre cobertura lê-se como "auditei tudo".
- Não criar issue. O achado vira issue quando o Yef decidir.
