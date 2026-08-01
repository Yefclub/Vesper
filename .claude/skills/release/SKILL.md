---
name: release
description: Use quando for promover a branch de integração para produção — "prepara a release", "sobe pra main", "monta a PR de release", "o que vai nessa versão?". Levanta o escopo, cura as notas para usuário final e abre a PR de promoção sem mergear.
---

# Release — promoção para produção

> **Este arquivo é a forma, não o procedimento final.** Pipeline de release é a parte mais específica de qualquer projeto: numeração, changelog, tag e artefato mudam por repositório. Preencher os `<...>` com o que este projeto faz de fato, verificado no workflow — nunca por analogia com outro projeto.

Promoção para produção é **sempre** decisão humana explícita, mesmo onde o merge para a branch de integração já esteja autorizado.

## 1. Escopo, com refs remotas frescas

```bash
git fetch origin <base> <produção> --tags
git log origin/<produção>..origin/<base> --oneline
git diff origin/<produção>...origin/<base> --shortstat
```

O ref local costuma estar atrás e induz a escopo e versão errados. Buscar antes de olhar, sempre.

**Levantar também**: migração de banco nova no intervalo — é o item que mais causa incidente em promoção e precisa de destaque no corpo da PR.

## 2. Versão

Como este projeto calcula a próxima versão: `<descrever o pipeline real>`.

Duas armadilhas que valem para quase todo pipeline automatizado e por isso ficam registradas aqui:

- **Republicar a mesma versão costuma ser impossível.** Corrigir uma release publicada custa uma versão nova.
- **Push na branch de produção pode re-disparar o workflow** e bumpar de novo. Conferir antes de empurrar qualquer correção direto lá.

## 3. Verificar prontidão

```bash
gh pr list --state open --base <base> --json number,title
```

Há PR aberta que deveria entrar nesta release? Listar e **perguntar** se espera. Cortar a release por conta própria com entrega relevante em voo é retrabalho garantido.

## 4. Branch de release a partir de `origin/<base>`

```bash
git worktree add <path> -b release/<versão> origin/<base>
```

Nada é commitado direto na branch de integração. O que mergear nela depois do corte fica para a próxima.

## 5. Notas de release — curar, não despejar

O texto é lido por **usuário final**. Lista crua de PR não serve.

- **Incluir** o que muda a experiência de quem usa o produto
- **Deixar de fora** ruído interno: infraestrutura de bastidor, refatoração, ajuste de pipeline, correção de correção
- **Agrupar por tema ou área**, não por PR
- Conferir **completude** contra o log do intervalo: área grande e visível não pode sumir só porque o título da PR era técnico

## 6. PR de promoção

```bash
gh pr create --base <produção> --head release/<versão> --title "Release <versão> — <tema>" --body "<corpo>"
```

Corpo com: destaques por área · números do período · **migração de banco (listar ou dizer "nenhuma")** · nota adicional.

## 7. Pós-merge — não encerrar sem verificar

Merge dispara o workflow de release. **Acompanhar o run até o fim e conferir o artefato**, um por um: versão publicada, notas consumidas, tag criada, versão sincronizada de volta.

Este passo existe porque o modo de falha real é o workflow falhar num passo intermediário e a release ser dada como entregue sem que ninguém tenha olhado.

## Regras

- **Nunca mergear a PR de release.** Criar, verificar e reportar o link.
- Migração nova no escopo: destacar como alerta, não como item de lista.
- Diff muito grande (>50 arquivos): sugerir revisão por área antes de promover.
- Ao final, remover a worktree e conferir `git worktree list`.
