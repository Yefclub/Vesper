---
name: sprint-summary
description: Use quando pedir a radiografia técnica do projeto — "como está o projeto?", "o que travou?", "resumo técnico da sprint", "onde está a dívida?". Entrega estado real para decisão técnica, com o que está bloqueado e por quê.
---

# Sprint summary — radiografia técnica

Público: quem vai **decidir o que fazer a seguir**. Linguagem técnica e direta.

Para público de gestão use `changelog`. Misturar as duas produz um documento que não serve para nenhum dos dois.

## Coletar

```bash
gh pr list --state open --json number,title,headRefName,createdAt,updatedAt,reviewDecision,statusCheckRollup,mergeable
gh pr list --state merged --search "merged:>=<data>" --json number,title,mergedAt,additions,deletions
gh issue list --state open --json number,title,labels,assignees,createdAt
git branch -r --sort=-committerdate
git worktree list
```

## Analisar

- PR bloqueada, e **por qual motivo concreto**: check vermelho (qual), conflito com a base, review parado há dias
- Issue sem responsável, ou sem atividade desde a abertura
- Branch remota sem commit há mais de 7 dias
- Dívida acumulando: quantas issues de dívida técnica entraram vs saíram no período — a **tendência** importa mais que o total

## Saída

```markdown
# <Projeto> — <período>

## Estado
<X> PRs abertas (<Y> verdes, <Z> bloqueadas) · <W> issues abertas (<N> sem responsável)

## Entregue
- #N <título> (+<a>/-<d>)

## Bloqueado — precisa de decisão
- #N <título> — <check `x` falhou: erro real | conflito com a base | sem review há N dias>

## Prioritário e ainda aberto
- #N <título> [<label>]

## Branch parada (>7 dias)
- <branch> — último commit há <X> dias

## Dívida técnica
Entraram <A>, saíram <B> no período. Tendência: <acumulando | estável | reduzindo>

## Próximo passo sugerido
<derivado dos dados acima, nomeando PR e issue — não genérico>
```

## Regras

- **Motivo concreto do bloqueio.** "CI vermelho" não é motivo; "`test:e2e` falhou em `auth.spec.ts:42` por timeout" é.
- Próximo passo tem que citar número de PR ou issue. Sugestão genérica ("melhorar cobertura") é ruído.
- Branch parada e worktree órfã: listar para limpeza. Elas viram confusão em três semanas.
- Não abrir código para auditar qualidade — isso é `audit`. Aqui é estado, não julgamento.
- Dado ausente é dito como ausente. Não estimar o que não foi coletado.
