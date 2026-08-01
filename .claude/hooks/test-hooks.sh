#!/bin/sh
# Suite dos hooks. Roda em Linux (dash) e no bash do MSYS2.
#   sh .claude/hooks/test-hooks.sh
# Sai 1 na primeira divergencia acumulada. Sem dependencia externa.
set -u
cd "$(dirname "$0")/../.." || exit 1

fail=0
total=0

# chk <comando> <esperado: deny|ask|PASSA>
chk() {
  total=$((total + 1))
  _r=$(printf '{"tool_input":{"command":"%s"}}' "$1" \
        | sh .claude/hooks/guard-bash.sh \
        | sed -n 's/.*"permissionDecision":"\([a-z]*\)".*/\1/p')
  [ -z "$_r" ] && _r=PASSA
  if [ "$_r" = "$2" ]; then
    printf '  ok    %-44s %s\n' "$1" "$_r"
  else
    printf '  FALHA %-44s esperado=%s obtido=%s\n' "$1" "$2" "$_r"
    fail=$((fail + 1))
  fi
}

# chkf <caminho> <esperado>
chkf() {
  total=$((total + 1))
  _r=$(printf '{"tool_input":{"file_path":"%s"}}' "$1" \
        | sh .claude/hooks/guard-secrets.sh \
        | sed -n 's/.*"permissionDecision":"\([a-z]*\)".*/\1/p')
  [ -z "$_r" ] && _r=PASSA
  if [ "$_r" = "$2" ]; then
    printf '  ok    %-44s %s\n' "$1" "$_r"
  else
    printf '  FALHA %-44s esperado=%s obtido=%s\n' "$1" "$2" "$_r"
    fail=$((fail + 1))
  fi
}

echo "git destrutivo — deve BLOQUEAR"
for c in "git push --force" "git push -f" "git push '--force'" \
         "git push --force-with-lease" "git push --force-with-lease=refs/x" \
         "git push origin +HEAD:refs/heads/main" "command git push --force" \
         "git -C /tmp/r push --force" "git reset --hard HEAD~1" \
         "git clean -fd" "git clean --interactive" "git -c clean.requireForce=false clean" \
         "git add -A" "git add ." "git add :/" "git add ':(top)'" \
         "git branch -D x" "git checkout ." "git restore ."; do chk "$c" deny; done

echo "stack local — deve PERGUNTAR"
for c in "docker compose up -d" "docker-compose up" "docker run -it img" \
         "podman run img" "pnpm dev" "npm run dev" "yarn dev" "npx vite" "vite"; do chk "$c" ask; done

echo "uso normal — deve PASSAR"
for c in "git push origin main" "git push -u origin feat/x" "git push" \
         "git push --force-if-includes" "git status --short" "git add src/index.ts" \
         "git add .github/workflows/ci.yml" "git add ':(top)README.md'" \
         "git checkout .gitignore" "git checkout -b feat/x" \
         "git clean -n -e foo" "git clean -fdn" "git commit -m invite" \
         "git log --oneline" "git reset HEAD~1" "git branch -d x" \
         "npm test" "pnpm build" "docker ps" "docker compose logs" \
         "podman run --help" "deno task dev --help" "docker run --help"; do chk "$c" PASSA; done

echo "credenciais — deve BLOQUEAR"
for f in "/proj/.env" "/proj/.env.production" "C:\\\\proj\\\\certs\\\\api.pem" \
         "/proj/server.key" "/home/y/.ssh/config" "/home/y/.aws/credentials"; do chkf "$f" deny; done

echo "arquivo comum — deve PASSAR"
for f in "/proj/src/index.ts" "/proj/README.md" "/proj/.env.example" "/proj/.env.sample" "/proj/.env.example.md" "/proj/docs/keys.md"; do chkf "$f" PASSA; done

echo "session-start — JSON valido ou silencio"
total=$((total + 1))
out=$(echo '{}' | sh .claude/hooks/session-start.sh)
if [ -z "$out" ]; then
  printf '  ok    %-44s %s\n' "session-start" "silencio (fora de repo git)"
elif printf '%s' "$out" | grep -q '^{"hookSpecificOutput":{.*}}$'; then
  printf '  ok    %-44s %s\n' "session-start" "JSON valido"
else
  printf '  FALHA %-44s saida malformada\n' "session-start"; fail=$((fail + 1))
fi

echo ""
if [ $fail -eq 0 ]; then
  echo "$total casos, todos verdes."
else
  echo "$total casos, $fail FALHA(S)."
  exit 1
fi
