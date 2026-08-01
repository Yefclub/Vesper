---
name: review-local
description: Use quando precisar revisar um commit local antes do push — "revisa esse commit", "review antes de abrir PR", ou como fallback quando o revisor externo de outra família de modelo estiver indisponível. Entrega veredicto APROVADO ou MUDANÇAS NECESSÁRIAS com escopo estreito.
---

# Review local — antes do push, nunca depois do CI

Revisão do **commit local**, na worktree, sem push e sem PR. É o passo 4 do ciclo. Existe para que cada achado não custe um ciclo de CI inteiro.

Fallback: só rodar esta skill quando o revisor externo de **outra família de modelo** estiver ausente, não autenticado ou tiver falhado. Ele é preferido — quem escreveu o código não enxerga a classe de bug que cometeu.

## Entradas

- Alvo: sha do commit, ou `origin/<base>...HEAD` para série
- Issue que a mudança resolve: número e título (**obrigatório** — define o que é "lacuna")

## Regra de escopo — o que faz ou quebra esta skill

**Reportar apenas:**
- defeito introduzido, ou não corrigido, **por este diff**
- caso em que a mudança quebra ou degrada comportamento existente
- lacuna na resolução da issue — algo que ela pede e o diff não entrega

**Não reportar:**
- problema pré-existente que o diff não introduziu nem piorou
- oportunidade de melhoria em código vizinho que o diff apenas encostou
- sugestão de refatoração, padronização ou cobertura fora do escopo da issue
- observação de qualidade que não afeta corretude do que está sendo entregue

Pré-existente **agravado** pelo diff: reportar, dizendo que é pré-existente e como o diff piora. Se apenas convive com a mudança, ignorar.

Sem esse recorte o review traz achado legítimo porém periférico, cada um vira issue, e o sinal — o que **esta** entrega precisa antes de mergear — some no volume.

## Fluxo

1. Ler o diff do alvo. Ler também os arquivos que ele toca, inteiros — diff sem o entorno esconde quebra de contrato.
2. Ler a issue. Sem ela não há como julgar lacuna.
3. Analisar, nesta ordem de peso:
   - **Corretude** — faz o que a issue pede? Edge case, off-by-one, condição invertida, estado inconsistente?
   - **Segurança** — guarda de auth e de autorização em rota nova; input validado e sanitizado na borda; query parametrizada; segredo fora de código, log e mensagem de erro; rate limit no que é caro
   - **Contrato quebrado** — assinatura, tipo de retorno, formato de payload, migration sem caminho de volta
   - **Teste** — mudança de lógica sem teste correspondente é lacuna, não sugestão
   - **Raio** — arquivo tocado que não rastreia para a issue
4. Emitir o veredicto.

## Saída

```
## Review — <alvo> (issue #N: <título>)

Arquivos: X alterados (+Y/-Z)

### Achados
1. [<arquivo>:<linha>] <o que está errado> → <por que quebra> → <correção sugerida>
   (ou "Nenhum achado dentro do escopo")

### Veredicto: APROVADO | MUDANÇAS NECESSÁRIAS
```

Sem seção de sugestão, nice-to-have ou não-bloqueante. Se um achado não merece entrar no diff agora, ele não merece ser escrito — rótulo de severidade é a porta pela qual dívida entra.

## Regras

- **Read-only.** Esta skill não edita, não commita, não cria issue. Quem implementa corrige.
- Todo achado cita **arquivo e linha**, e descreve o modo de falha concreto — não "poderia ser melhor".
- Nunca afirmar que algo é seguro ou correto sem ter lido o código. Não responder de memória.
- Correção aplicada → **re-submeter ao review**. Não existe auto-aprovação de quem corrigiu.
- **Terceira rodada trazendo casos cada vez mais periféricos = problema de escopo, não de qualidade.** Fechar com o que está correto e dizer isso.
