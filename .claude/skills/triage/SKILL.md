---
name: triage
description: Use quando pedir para organizar o backlog — "organiza as issues", "o que atacar primeiro?", "tem issue duplicada?", "monta a próxima sprint". Entrega issues categorizadas por prioridade, esforço e dependência, sem alterar nada no repositório.
---

# Triage — triagem de issues

Read-only. Esta skill **não fecha, não edita, não rotula** issue — só reporta. Alteração é decisão humana.

## Coletar

```bash
gh issue list --state open --json number,title,body,labels,assignees,createdAt,updatedAt --limit 100
```

## Analisar cada issue

**Ler o corpo, não só o título.** Triagem por título é chute com formatação bonita — o título mente sobre o escopo na maioria das issues.

- **Área** — que parte do sistema é afetada
- **Prioridade** — crítica (produção quebrada), alta (funcionalidade comprometida), média (melhoria relevante), baixa
- **Esforço** — P (horas), M (1-2 dias), G (3+ dias)
- **Dependência** — precisa de outra antes? bloqueia outra?

Prioridade se mede por **impacto no usuário final**, não por dificuldade técnica. Issue difícil e irrelevante continua sendo baixa.

## Detectar sobreposição

- Mesmo problema descrito com palavras diferentes
- Issues que tocam os mesmos arquivos e caberiam numa entrega só
- Issues que se contradizem — as duas não podem ser implementadas como escritas

## Saída

```markdown
# Triagem — <data>

## Crítica
- #N <título> — [<área>] [<P/M/G>] <depende de #M>

## Alta / Média / Baixa
- ...

## Sobreposição
- #N e #M: <por que parecem a mesma coisa>
- #N e #O: tocam <arquivos> — considerar entrega única
- #N e #P: **conflitam** — <em quê>

## Sugestão de recorte
- Agora: #N, #M (esforço somado <X>)
- Depois: #P, #Q
- Backlog: #R

## Sem informação suficiente
- #N — falta: <reprodução | contexto | critério de aceite>
```

## Regras

- Issue sem informação para triagem vai para "sem informação suficiente". **Não adivinhar** prioridade nem esforço.
- Estimativa é ordem de grandeza, não promessa. Marcar como estimativa.
- Não criar, fechar, rotular ou comentar issue. Só reportar.
- Backlog que não anda equivale a não ter registrado — se a lista está crescendo há meses, dizer isso em vez de reorganizar em silêncio.
