---
name: changelog
description: Use quando pedir resumo de entregas para quem não é técnico — "o que entregamos essa semana?", "resumo pra gestão", "changelog do mês", "update pro cliente". Entrega texto em linguagem de negócio, focado em valor, não em implementação.
---

# Changelog — resumo para gestão

Público **não técnico**. O critério de qualidade é: alguém de fora do time entende o que mudou e por que importa, sem perguntar nada.

Para relatório técnico (o que está bloqueado, dívida, branch parada) use `sprint-summary` — é outra audiência e outro documento.

## Entradas

- **Período**: `semana`, `mês`, ou intervalo `AAAA-MM-DD..AAAA-MM-DD`
- **Formato** (opcional): `markdown` (padrão), `bullets` para colar em ferramenta de gestão, `email`

## Coletar

```bash
gh pr list --state merged --search "merged:>=<data>" --json number,title,mergedAt,author,labels,additions,deletions
gh issue list --state closed --search "closed:>=<data>" --json number,title,closedAt,labels
git log --since=<data> --until=<data> --oneline --shortstat
```

## Escrever

Agrupar por **funcionalidade nova · correção · melhoria de uso · infraestrutura · segurança · performance**.

Regra que define a skill: **traduzir implementação em valor.**

| Não escrever | Escrever |
|---|---|
| "implementou CalendarStore com cache" | "agenda agora abre instantânea ao trocar de mês" |
| "corrigiu race condition no upload" | "arquivo grande não falha mais no meio do envio" |
| "migrou para índice composto" | "busca de pedido ficou 4× mais rápida" |

```markdown
# <Projeto> — <período>

## Destaques
<3 a 5 itens de maior impacto, em linguagem de negócio>

## Novidades
- <o que o usuário passa a conseguir fazer> (#N)

## Correções
- <o que parou de dar errado> (#N)

## Melhorias
- <o que ficou mais rápido, claro ou simples> (#N)

## Números
<X> entregas · <Y> correções · <período>
```

## Regras

- **Sem jargão nos destaques.** Nome de biblioteca, padrão de projeto e nome de arquivo não entram.
- Entrega interna sem efeito visível (refatoração, atualização de dependência) vira **uma linha agregada** em infraestrutura — não some, mas não ocupa destaque.
- Número serve de apoio, não de manchete. Linha de código alterada não é medida de valor e pode ser omitida.
- Período sem entrega relevante: **dizer isso**. Inflar changelog queima a credibilidade do próximo.
- Não incluir link de PR nos destaques quando o leitor não tem acesso ao repositório.
