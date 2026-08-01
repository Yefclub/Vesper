---
name: monday
description: Use quando pedir para registrar trabalho na ferramenta de gestão — "atualiza o Monday", "documenta essa decisão no board", "sincroniza as PRs com o Monday", "cria um update lá". Consulta antes de escrever e nunca cria estrutura nova sem confirmar.
---

# Monday — registro na ferramenta de gestão

Ponte entre o trabalho no repositório e o board de gestão. Depende do servidor MCP do Monday estar registrado na sessão — ausente, dizer isso e parar, não tentar contornar por outro caminho.

> Nome de ferramenta MCP envelhece. Descobrir as disponíveis na sessão em vez de assumir a lista; se um nome aqui não existir mais, **corrigir este arquivo na hora**.

## Regra que define a skill: ler antes de escrever

Sempre nesta ordem: **contexto do usuário → buscar → ler o item → só então escrever.** Criar item porque a busca não achou é o erro clássico: gera duplicata que ninguém reconcilia depois.

Não encontrou o board ou o item esperado? **Perguntar.** Não criar.

## Fluxos

**Atualizar status de trabalho**
1. Buscar o board relevante
2. Localizar o item — conferir que é o certo lendo o conteúdo, não só o título
3. Atualizar a coluna de status
4. Adicionar update com o detalhe e o link da entrega

**Documentar decisão técnica**
Estruturar sempre como **contexto → decisão → alternativas consideradas → motivo**. O motivo é a parte que envelhece bem; sem ele, o documento vira registro de que algo foi decidido, não de por quê.

**Sincronizar repositório → board**
Entrega mergeada ou issue fechada: localizar o item correspondente, atualizar status, anexar o link como update. Sem item correspondente → sugerir criação, não criar.

**Relatório de progresso**
Gerar o conteúdo com `changelog` (gestão) ou `sprint-summary` (técnico) e publicar aqui. Não reescrever a lógica de coleta dentro desta skill.

## Regras

- **Não criar board, grupo ou estrutura nova sem confirmação.** Item, update e documento são reversíveis; estrutura não é, e bagunça board de time inteiro.
- Conteúdo em PT-BR.
- Board de gestão recebe linguagem de negócio. Jargão de código ali não é lido por ninguém.
- Usar a formatação nativa da ferramenta, não HTML cru.
- Nunca colar segredo, token ou trecho de log com credencial num update — o board tem público muito maior que o repositório.
- Escrita em massa (atualizar N itens de uma vez): confirmar o alvo antes. Erro em lote é caro de desfazer.
