@AGENTS.md

# Camada Claude Code

O conteúdo do projeto está no `AGENTS.md` acima — lido também por Codex, Cursor, Copilot e Gemini. Este arquivo só carrega o que é específico do Claude Code. **Não duplicar regra daqui para lá.**

## Skills

Ficam em `.claude/skills/<nome>/SKILL.md`, cada uma com frontmatter `name` + `description`. A `description` é o que decide a invocação automática — se ela estiver boa, não existe tabela "usuário pediu X → use skill Y" neste arquivo. Se você sentir vontade de escrever essa tabela, o problema é a `description`, não a falta da tabela.

## Subagentes

Toda chamada passa o modelo por **alias** (`opus`, `sonnet`), nunca por id versionado — id fixo envelhece e trava o subagente num modelo velho sem ninguém perceber.

Exploração ampla vai para subagente em contexto próprio, para não sujar o contexto principal com o material lido pelo caminho.

## Hooks

`.claude/settings.json` registra guardas em `.claude/hooks/`, escritos em POSIX `sh` — rodam em Linux, macOS e, no Windows, pelo Git Bash. Bloqueiam git destrutivo, `git add` cego e escrita em arquivo de credencial; perguntam antes de subir stack local.

A guarda erra para o lado de **deixar passar**: ela é rede, não muralha. A regra continua sendo a do `AGENTS.md`.

## Preferência pessoal

Fica em `CLAUDE.local.md` na raiz, **gitignored** — é de quem está rodando, não do projeto. Em conflito com este arquivo, **o do projeto ganha**, exceto regras de ambiente: o que pode subir na máquina e onde o checkout de trabalho pode ser tocado são decisões de quem senta na cadeira.
