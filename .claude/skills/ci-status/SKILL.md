---
name: ci-status
description: Use quando perguntar o estado das entregas em voo — "como estão as PRs?", "o CI passou?", "o que está bloqueado?", "tem PR parada?". Entrega panorama classificado das PRs abertas sem ler código.
---

# CI status — panorama das PRs abertas

Consulta de **estado**, não de código. Precisa ser rápida: nada de abrir arquivo, nada de analisar diff.

## Coletar

```bash
gh pr list --state open --json number,title,headRefName,author,updatedAt,statusCheckRollup,reviewDecision,mergeable
gh run list --limit 10 --json displayTitle,status,conclusion,headBranch,createdAt
git worktree list
```

## Classificar cada PR

| Situação | Critério |
|---|---|
| Pronta | CI verde **e** review aprovado |
| Aguardando review | CI verde, sem aprovação |
| Bloqueada | algum check vermelho |
| Em execução | check pendente |
| Conflito | não mergeável, precisa atualizar contra a base |
| Parada | sem atividade há mais de 3 dias |

CI verde sozinho **não** é "pronta". O gate é verde **e** aprovado.

## Saída

```markdown
# CI status — <data/hora>

## Pronta para merge
- #N <título> — CI verde, review aprovado

## Aguardando review
- #N <título>

## Bloqueada
- #N <título> — check `<nome>` falhou: <resumo de 1 linha do erro real>

## Em execução
- #N <título> — <check> rodando há <tempo>

## Conflito com a base
- #N <título>

## Parada (>3 dias)
- #N <título> — última atividade há <X> dias

## Worktrees
- <path> → <branch> <(mergeada — remover)>

Resumo: <X> abertas | <Y> prontas | <Z> bloqueadas
```

## Regras

- **Factual.** Check rodando é "rodando" — nunca "deve passar". Previsão aqui vira decisão errada lá na frente.
- Check vermelho: **ler o log** e resumir o erro real em 1 linha. "CI falhou" sozinho não ajuda ninguém.
- Worktree cuja branch já foi mergeada: listar para remoção. Worktree órfã acumula e depois confunde.
- Não abrir código, não analisar diff, não sugerir correção. Isso é outra skill.
