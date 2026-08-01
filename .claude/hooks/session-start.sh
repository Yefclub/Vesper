#!/bin/sh
# SessionStart — injeta o estado real do repositorio no contexto.
#
# Existe para que o inicio de sessao seja fato levantado, nao memoria da sessao
# anterior. Nunca falha a sessao: qualquer erro vira silencio.
set -u

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0

ctx=""
add() { ctx="${ctx}$1
"; }

branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')
dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')
add "Branch: ${branch} | arquivos modificados: ${dirty}"

# Worktree so aparece se houver mais de uma — lista de um item e ruido.
# NAO usar `... | while read`: em POSIX sh o corpo do while roda em subshell e as
# atribuicoes a $ctx morrem com ele — o contexto sairia com o cabecalho e zero linhas.
wtlist=$(git worktree list 2>/dev/null)
if [ -n "$wtlist" ] && [ "$(printf '%s\n' "$wtlist" | wc -l | tr -d ' ')" -gt 1 ]; then
  add "Worktrees ativas:"
  add "$(printf '%s\n' "$wtlist" | sed 's/^/  /')"
fi

# gh e opcional. Ausente ou sem auth: seguir sem PRs, nunca travar.
if command -v gh >/dev/null 2>&1; then
  prs=$(gh pr list --state open --limit 20 --json number,title,headRefName \
          --template '{{range .}}  #{{.number}} {{.title}} ({{.headRefName}})
{{end}}' 2>/dev/null)
  if [ -n "$prs" ]; then
    add "PRs abertas:"
    add "$prs"
  fi
fi

[ -z "$ctx" ] && exit 0

# Escapa para JSON: barra invertida, aspas e quebra de linha, nesta ordem.
esc=$(printf '%s' "$ctx" \
  | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' \
  | awk 'BEGIN{ORS=""} {print $0 "\\n"}')

printf '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Estado do repositorio no inicio da sessao:\\n%s"}}\n' "$esc"
