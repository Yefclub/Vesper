#!/bin/sh
# PreToolUse / Bash — barra o que as regras deste repositorio proibem.
#
# POSIX puro e sem jq: jq nao vem instalado por padrao em lugar nenhum, e um hook
# que depende dele falha em silencio na maquina errada. Roda em Linux (dash) e
# no bash do MSYS2 (Git Bash no Windows).
#
# ESCOPO: pega o erro comum e o descuido. NAO e fronteira de seguranca — quem
# quiser burlar, burla (`eval`, variavel, script intermediario, base64). A regra
# de verdade mora no AGENTS.md; isto e a rede embaixo dela.
#
# Casa por TOKEN, nunca por substring: glob de substring erra dos dois lados —
# `git -C /tmp push --force` escapava do padrao e `git commit -m invite` casava
# com *vite*. Os dois ja aconteceram aqui.
set -u

input=$(cat)

# Extrai .tool_input.command sem jq. Comando com aspas escapadas quebra este parse;
# nesse caso o guard DEIXA PASSAR. Falso negativo e melhor que travar a sessao por
# um comando legitimo mal parseado.
cmd=$(printf '%s' "$input" | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
[ -z "$cmd" ] && exit 0

decide() {
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"%s","permissionDecisionReason":"%s"}}\n' "$1" "$2"
  exit 0
}
deny() { decide deny "$1"; }
ask()  { decide ask  "$1"; }

# set -f antes do split: sem isso um `*` no comando vira expansao de arquivo.
set -f
# shellcheck disable=SC2086
set -- $cmd
set +f
[ $# -eq 0 ] && exit 0

# Pedir ajuda nunca executa nada. Curto-circuito aqui mata a classe inteira de
# falso positivo do tipo `docker run --help` e `deno task dev --help`.
for _a in "$@"; do
  case $_a in --help|-h|"'--help'"|'"--help"') exit 0 ;; esac
done

# Tira aspas em volta do token. `git push '--force'` chega com as aspas coladas
# no argumento, porque o split aqui e por espaco, nao pelo parser do shell.
unq() { _v=${1#[\"\']}; printf '%s' "${_v%[\"\']}"; }

has() { # token exato, com ou sem aspas
  _n=$1; shift
  for _a in "$@"; do [ "$(unq "$_a")" = "$_n" ] && return 0; done
  return 1
}
hasp() { # algum token comeca com o prefixo dado
  _p=$1; shift
  for _a in "$@"; do case $(unq "$_a") in "$_p"*) return 0 ;; esac; done
  return 1
}

# Desembrulha prefixo que executa outro comando: `command git push --force` roda
# git, mas o executavel lido seria `command`.
while [ $# -gt 0 ]; do
  case $(unq "$1") in
    command|builtin|exec|env|sudo|nohup|time|nice|stdbuf) shift ;;
    *=*) shift ;;   # VAR=valor cmd
    *) break ;;
  esac
done
[ $# -eq 0 ] && exit 0

exe=$(unq "$1")
exe=${exe#\\}      # \git escapa alias, continua sendo git
exe=${exe##*/}     # /usr/bin/git -> git
shift

# ---------------------------------------------------------------- git
if [ "$exe" = "git" ]; then
  # Opcoes globais vem ANTES do subcomando: git -C <path> push, git -c k=v ...
  while [ $# -gt 0 ]; do
    case $(unq "$1") in
      -C|-c|--exec-path|--git-dir|--work-tree|--namespace) shift 2 || exit 0 ;;
      -*=*|--exec-path|--git-dir|--work-tree) shift ;;
      -*) shift ;;
      *) break ;;
    esac
  done
  [ $# -eq 0 ] && exit 0
  sub=$(unq "$1"); shift

  case $sub in
    push)
      # --force, -f, --force-with-lease e --force-with-lease=<ref>. Refspec com
      # `+` na frente forca sem nenhuma flag: git push origin +HEAD:refs/heads/x
      # --force exato: --force-if-includes NAO forca nada sozinho e nao pode cair aqui.
      # --force-with-lease aceita =<ref>, entao esse sim e por prefixo.
      if has --force "$@" || has -f "$@" || hasp --force-with-lease "$@" || hasp + "$@"; then
        deny "push forcado reescreve historico publicado. Se for mesmo necessario, rode voce mesmo e diga o porque."
      fi ;;
    reset)
      has --hard "$@" && deny "reset --hard descarta trabalho sem caminho de volta. Prefira git stash ou um commit temporario." ;;
    clean)
      # Simplificacao deliberada: SO passa se for simulacao explicita. Enumerar as
      # formas destrutivas e jogo perdido (-fd, --force, -i, clean.requireForce=false).
      # `git clean` sem nada falha sozinho, entao negar tambem nao custa nada.
      _dry=1
      has -n "$@" || has --dry-run "$@" || _dry=0
      if [ $_dry -eq 0 ]; then
        for a in "$@"; do
          case $(unq "$a") in --*) ;; -*n*) _dry=1 ;; esac   # cluster tipo -fdn
        done
      fi
      [ $_dry -eq 0 ] && deny "clean apaga arquivo nao rastreado em definitivo, inclusive .env local. Use -n para simular, ou confirme antes." ;;
    checkout|restore)
      has . "$@" && deny "Descarta toda a mudanca nao commitada do diretorio. Se e isso mesmo, rode voce mesmo." ;;
    branch)
      has -D "$@" && deny "Delecao forcada de branch: -D ignora se a branch foi mergeada. Confirme antes." ;;
    add)
      # :/ e :(top) sao pathspec de raiz — stageiam tudo, igual ao ponto.
      # :/ e :(top) SOZINHOS stageiam tudo. Com arquivo colado (':(top)README.md')
      # e o oposto: seleciona um arquivo so, que e exatamente o que a regra pede.
      if has -A "$@" || has --all "$@" || has . "$@" || has :/ "$@" || has ':(top)' "$@"; then
        deny "git add cego e como segredo e arquivo pessoal vazam para o commit. Adicione arquivo por arquivo."
      fi ;;
  esac
  exit 0
fi

# ---------------------------------------------------------------- container
CTR="Subir container e decisao do dono da maquina — o ambiente local pode estar rodando em paralelo."
case $exe in
  docker|podman)
    case $(unq "${1:-}") in
      run) ask "$CTR" ;;
      compose) has up "$@" && ask "$CTR" ;;
    esac ;;
  docker-compose|podman-compose)
    has up "$@" && ask "$CTR" ;;
esac

# ---------------------------------------------------------------- servidor de dev
DEV="Servidor de desenvolvimento por iniciativa propria nao. Validacao visual vai contra o ambiente de QA."
case $exe in
  vite) ask "$DEV" ;;
  npm|pnpm|yarn|bun|npx|deno)
    has dev "$@" && ask "$DEV"
    has vite "$@" && ask "$DEV"
    ;;
esac

exit 0
