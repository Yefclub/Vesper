---
name: ship
description: Use quando pedir para entregar a mudança — "faz a PR", "shipa isso", "manda pra dev", "abre PR e merge se ok". Conduz worktree, commit local, review antes do push, PR e CI, na ordem que não desperdiça ciclo de runner.
---

# Ship — worktree, review, PR, CI

A ordem aqui não é estilo, é economia: **o review acontece antes do push**. Com review depois, cada achado custa um ciclo de CI inteiro, e mais da metade do runner é gasta em commit intermediário que ninguém ia mergear.

## 1. Worktree, nunca o checkout principal

```bash
git fetch origin
git worktree add <path> -b <tipo>/<descrição-curta> origin/<base>
```

Cortar sempre de `origin/<base>` recém-buscada — branch local desatualizada produz validação fora de fase com o estado real. **Nunca** `checkout`, `pull` ou `stash` no checkout principal: ele é de quem está trabalhando nele em paralelo.

Todo edit, teste, build e commit acontece dentro da worktree.

## 2. Checklist antes do commit

Rodar e **ler a saída**: lint, typecheck, teste, build. Vermelho em qualquer um: corrigir antes de seguir. Colar a evidência no relato — afirmação sem saída de comando não conta.

## 3. Commit local, sem push

```bash
git add <arquivo> <arquivo>     # nunca -A: é assim que segredo vaza
git commit -m "<tipo>(<escopo>): <descrição>"
```

## 4. Review do commit local — pré-requisito, não etapa opcional

Preferir revisor de **outra família de modelo**: quem escreveu não enxerga a classe de bug que cometeu. Indisponível ou falhou → skill `review-local` (subagente em contexto limpo, modelo mais capaz por alias). **Nunca pular.**

Escopo estreito, sempre: só defeito introduzido pelo diff, quebra de comportamento existente, e lacuna do que a issue pede.

## 5. Corrigir e repetir

Achado **entra no diff desta entrega**, antes do merge — independente do rótulo ("não-bloqueante", "sugestão", "follow-up"). Não abrir issue de follow-up, não deixar TODO, não mergear com ressalva.

Corrigir na worktree → re-rodar **só o que a correção tocou** → re-submeter ao review. Repetir até voltar limpo. Quem corrigiu não se auto-aprova.

Terceira rodada trazendo casos cada vez mais periféricos: o problema é escopo, não qualidade. Fechar com o que está correto.

## 6. Push único e PR

```bash
git push -u origin <branch>
gh pr create --base <base> --head <branch> --title "<título>" --body "<corpo>"
```

Corpo com contexto, o que foi feito e como testar. Sem emoji, sem rodapé de ferramenta.

## 7. CI

```bash
gh pr checks <N> --watch --fail-fast
```

Acompanhar em background para não travar a conversa. **Uma PR por vez no CI** — várias worktrees podem implementar e revisar em paralelo, mas o runner é gargalo único.

Vermelho: reportar o erro real, corrigir, empurrar de novo. Este é o segundo push legítimo — o objetivo era eliminar o ciclo evitável, não proibir correção de flake ou divergência de ambiente.

## 8. Merge — só com autorização

Merge é decisão humana. Sem autorização explícita: reportar "PR #N com CI verde e review aprovado, pronta para merge" e parar.

Com autorização: `gh pr merge <N> --squash --delete-branch`. Gate obrigatório: **CI verde E review aprovado sobre a versão já corrigida** — verde sozinho não basta.

Promoção para produção **sempre** reconfirma, mesmo com autorização em vigor para a branch de integração.

## 9. Limpar

```bash
git worktree remove <path>
git worktree list        # conferir que não sobrou órfã
git fetch origin         # próxima fase corta worktree nova
```

## Saída

```
PR #N — <título>
CI: <verde | vermelho: check X> · Review: <aprovado | N achados corrigidos>
Status: <mergeada | aguardando autorização>
<link>
```
