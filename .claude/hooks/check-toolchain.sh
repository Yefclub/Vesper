#!/bin/sh
# SessionStart — confere se a toolchain esperada esta declarada neste projeto.
#
# Verifica DECLARACAO, nao conectividade: se um servidor MCP esta declarado mas
# nao conecta, o proprio Claude Code reporta na inicializacao. O que ele nao
# reporta e a ausencia da declaracao — e e esse o buraco que este hook fecha.
#
# Nunca falha a sessao: no maximo injeta um aviso.
set -u

miss=""
note() { miss="${miss}$1
"; }

# --- MCP: precisam estar em .mcp.json (versionado, vale para o time inteiro) ---
if [ -f .mcp.json ]; then
  for s in playwright context7; do
    grep -q "\"$s\"" .mcp.json || note "  MCP '$s' nao esta em .mcp.json"
  done
else
  note "  .mcp.json ausente — playwright e context7 nao chegam ao time"
fi

# --- Plugins: declarados no settings.json versionado do projeto ---
if [ -f .claude/settings.json ]; then
  grep -q 'superpowers' .claude/settings.json || note "  plugin 'superpowers' nao habilitado em .claude/settings.json"
fi

# --- Saida do playwright nao pode cair na raiz nem entrar no git ---
if [ -f .claude/playwright-mcp.config.json ]; then
  grep -q '"outputDir"' .claude/playwright-mcp.config.json \
    || note "  playwright sem outputDir: screenshot vai cair na raiz do projeto"
fi
if [ -f .gitignore ] && [ -f .claude/playwright-mcp.config.json ]; then
  grep -q '^\.playwright' .gitignore || note "  .playwright/ fora do .gitignore — screenshot vai para o commit"
fi

# --- AGENTS.md com marcador nao preenchido ---
# O proprio yef-agents e o template, entao ali os marcadores sao esperados: o
# sentinela abaixo existe so nele e desliga esta checagem na origem.
if [ -f AGENTS.md ] && [ ! -f .claude/.is-template ]; then
  n=$(grep -c '<[a-zà-ú][^>]*>' AGENTS.md 2>/dev/null || echo 0)
  [ "${n:-0}" -gt 0 ] && note "  AGENTS.md tem ${n} marcador(es) <...> sem preencher — o agente le marcador como fato"
fi

[ -z "$miss" ] && exit 0

esc=$(printf '%s' "$miss" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | awk 'BEGIN{ORS=""} {print $0 "\\n"}')
printf '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Toolchain incompleta neste projeto:\\n%s"}}\n' "$esc"
