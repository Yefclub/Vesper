#!/bin/sh
# PreToolUse / Write|Edit — impede escrita em arquivo de credencial.
#
# A regra "segredo nunca em codigo, commit ou log" so vale se algo a aplicar.
# Prosa em CLAUDE.md pede; hook impede.
set -u

input=$(cat)

path=$(printf '%s' "$input" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
[ -z "$path" ] && exit 0

# Normaliza separador do Windows para casar com os padroes abaixo.
norm=$(printf '%s' "$path" | tr '\\' '/')
base=${norm##*/}

deny() {
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"%s"}}\n' "$1"
  exit 0
}

# .env.example e irmaos sao template SEM segredo, versionados de proposito —
# bloquear eles trava trabalho legitimo. Sai antes da checagem de credencial.
case "$base" in
  *example*|*sample*|*template*|*.md) ;;
  .env|.env.*|*.pem|*.key|*.p12|*.pfx|id_rsa|id_ed25519|.credentials.json|.npmrc|.pypirc)
    deny "Arquivo de credencial. Segredo se edita a mao, fora do agente — e nunca entra em commit, log ou output." ;;
esac

case "$norm" in
  */.ssh/*|*/.aws/credentials|*/.docker/config.json|*/.kube/config)
    deny "Configuracao de credencial de infraestrutura. Alterar isso a partir do agente nao esta no escopo de nenhuma tarefa." ;;
esac

exit 0
