# AGENTS.md — Vesper

Instruções para agentes de código neste repositório. Lido por Codex, Cursor, Copilot e Gemini CLI diretamente; o Claude Code chega aqui pelo `@AGENTS.md` no topo do `CLAUDE.md`.

## Projeto

- **O que é**: aplicativo desktop de notas de reunião com IA, privacy-first — grava, transcreve e resume reunião local, sem conta e sem telemetria.
- **Stack**: Rust + Tauri 2 no backend; React 19 + TypeScript 5.8 + Tailwind 4 + Vite 7 no front. SQLite via `rusqlite` (bundled). STT local com `whisper-rs` (whisper.cpp), LLM local com `llama-cpp-2` (llama.cpp). OpenRouter é opcional, nunca obrigatório.
- **Gerenciador de pacotes**: `npm` (o repo versiona `package-lock.json`) — usar **só** esse; misturar gera lockfile conflitante. Do lado Rust, `cargo` com `src-tauri/Cargo.lock`.
- **Estrutura**:
  - `src/` — UI React, tema escuro "Grok Night"
  - `src-tauri/src/domain/` — lógica pura; **é aqui que moram os testes**
  - `src-tauri/src/{audio,stt,llm,db}/` — ports e implementações
  - `src-tauri/src/commands.rs` — superfície IPC exposta ao front
  - `docs/` — notas de produto versionadas

## Comandos

```bash
npm install                    # instalar dependências do front
npm run tauri dev              # subir a app em desenvolvimento
npm run build                  # tsc --noEmit && vite build
npm run tauri build            # bundle desktop de produção
npm run typecheck              # tsc --noEmit
npm run lint                   # alias de typecheck — não há ESLint configurado
cd src-tauri && cargo test     # testes Rust
```

`lint` e `typecheck` executam o mesmo `tsc --noEmit`: o projeto ainda não tem linter de verdade do lado do front. Não tratar "lint verde" como cobertura de estilo. Do lado Rust, o CI roda `cargo fmt --check` e `cargo clippy -- -D warnings -A dead_code` em Linux e Windows.

**Pré-requisito de build no Windows**: o build script do `llama_cpp_sys` procura as binutils da LLVM e aborta com `No suitable tool equivalent to "nm"/"objcopy" has been found`. A LLVM traz as duas, mas não entra no PATH:

```bash
export PATH="/c/Program Files/LLVM/bin:$PATH"   # antes de cargo build/test
```

Pôr o diretório inteiro no PATH, e não `NM_PATH`/`OBJCOPY_PATH` uma a uma: ele pede a próxima ferramenta só depois de achar a anterior, então resolver individualmente vira uma falha por rodada de build.

Só morde em checkout novo — worktree recém-cortada tem `target/` vazio e roda o build script do zero.

**Build com GPU** (`--features gpu-vulkan`, `gpu-cuda` ou `gpu-all`): desligado por padrão, e cada um exige SDK **de build**, nunca de execução — o binário entregue não pede nada instalado.

- `gpu-vulkan` precisa do Vulkan SDK; `gpu-cuda` precisa do CUDA Toolkit (`nvcc`).
- **Caminho longo trava o build do Vulkan no Windows.** O `vulkan-shaders-gen` é um ExternalProject aninhado — do `whisper-rs-sys` *e* do `llama-cpp-sys-2` — e o `tracker.exe` do MSBuild ignora `LongPathsEnabled`: acima de ~260 caracteres ele aborta com `FileTracker : error FTK1011: could not create the new file tracking log file`. Dentro de `.claude/worktrees/<nome>/` o caminho já passa disso, e no runner do GitHub (`D:\Vesper\Vesper\...`) também. Solução em ambos: `CARGO_TARGET_DIR=/c/vt`. O CI faz isso num passo próprio.

  Build de Vulkan no Windows, então, é: `CMAKE_GENERATOR=Ninja CARGO_TARGET_DIR=/c/vt cargo build --features gpu-vulkan`.
- **Vulkan no Windows precisa do gerador Ninja.** O `vulkan-shaders-gen` é um ExternalProject aninhado e o MSBuild espalha os passos dele por nós paralelos: o install começa antes do configure terminar e sai `CMake error : Not a file: .../cmake_install.cmake`, que o `cmake-0.1.58` reporta só como pânico sem mensagem útil. Com `CMAKE_GENERATOR=Ninja` a ordem fica certa (e compila mais rápido). Repetir o comando às vezes passa, porque a segunda tentativa acha os artefatos já prontos — é sintoma, não solução.
- **O `dynamic-link` não é opcional em plataforma nenhuma.** `whisper-rs-sys` e `llama-cpp-sys-2` trazem cada um a sua cópia do ggml; duas cópias estáticas dão `LNK2005` no MSVC e `rust-lld: error: duplicate symbol` no Linux. Não tentar linkar estático de novo "porque no Linux o linker aceita" — ele não aceita.
- O `build.rs` copia os backends do ggml para `src-tauri/gpu-backends/` (gitignored). O `llama-cpp-sys-2` está declarado como dependência direta **só** para o cargo entregar `DEP_LLAMA_ROOT` e `DEP_LLAMA_BACKENDS_DIR` ao build script: metadado de `links` só chega em dependente direto, e são os únicos ponteiros que nomeiam a unidade que o cargo está construindo agora. O `tauri.conf.json` declara a pasta como recurso; sem isso o app instalado não acha backend nenhum, nem o de CPU, e nenhum modelo carrega. No Linux o `build.rs` ainda acrescenta `-rpath $ORIGIN/../lib/Vesper`, que é onde o Tauri põe recurso enquanto o binário fica em `/usr/bin`.

Falhou qualquer um deles: **parar e reportar a saída**. Não contornar, não desabilitar regra, não marcar teste como skip para seguir.

## Git e Pull Request

- **Branch de integração**: `dev` — toda branch de trabalho sai daí, atualizada (`git fetch origin` antes, nunca de branch local stale).
- **Branch de produção**: `main` — recebe tag de versão e artefato de updater.
- **Nome de branch**: `tipo/descricao-curta`, com `tipo` em `feat`, `fix`, `chore`, `docs`, `ci` ou `refactor`
- **Commit**: conventional commits, em **inglês**.
- **Título e corpo de PR**: **inglês**. Corpo com contexto, o que foi feito e como verificar.
- **Nunca commitar direto** em `dev` ou `main`.
- **Nunca `git add -A`** — adicionar arquivo por arquivo, conferindo o que entra. Segredo e arquivo pessoal vazam exatamente assim.
- **Merge para `dev`**: permitido sem perguntar, com gate de **CI verde E review aprovado** sobre a versão já corrigida. Gate é qualidade, não permissão.
- **Merge para `main` é promoção para produção**: exige confirmação humana explícita, sempre.

## PRs em pilha

Fase dependente vira pilha, não PR sequencial. O fluxo, as flags que travam e a ordem que mantém uma PR por vez no CI estão na skill `stacked-prs`.

## Antes de abrir PR

Rodar, nesta ordem, e **colar a saída** no relato — afirmação sem evidência não conta:

1. `npm run typecheck`
2. `npm run build`
3. `cd src-tauri && cargo test`

Mudou hook ou skill: `sh .claude/hooks/test-hooks.sh` também.

Mexeu em `.github/workflows/`: `npm run preflight` também. Ele checa as quatro coisas que já custaram um build de matriz inteiro cada uma — versão que o MSI recusa, codename de distro escrito à mão em vez de derivado do runner, empacotamento de AppImage sem `LD_LIBRARY_PATH` para as nossas `.so`, e chave YAML duplicada. `python scripts/check-workflows.py --self-test` verifica que as regras ainda disparam; duas delas já nasceram quebradas passando limpo.

Vermelho em qualquer etapa = PR não sai.

## Como escrever a mudança

- **Mínimo que resolve.** Nada especulativo: sem feature além da pedida, sem abstração para uso único, sem "flexibilidade" que ninguém pediu, sem tratar erro impossível.
- **Cirúrgico.** Não "melhorar" código vizinho, não refatorar o que não está quebrado, **imitar o estilo existente mesmo discordando dele**. Órfão que a própria mudança criou (import, variável, função) sai junto; código morto pré-existente se **aponta, não se apaga**.
- **Toda linha alterada rastreia direto para o pedido.** Não rastreia, sai do diff.
- **Critério de sucesso definido antes de começar**: "adiciona validação" vira "teste de input inválido passando".
- Ambíguo, ou com mais de uma leitura possível: **perguntar**. Não escolher em silêncio.

## Segurança — não é opcional

O produto promete privacidade. Regressão aqui é quebra de promessa, não bug de conforto.

- **Todo `#[tauri::command]` é superfície de ataque.** O WebView é a fronteira de confiança: validar parâmetro que vira caminho de arquivo, URL de download ou query. Não confiar em valor "que vem do próprio front".
- **Nada de URL arbitrária vinda do front** para download de modelo — origem sai de catálogo interno, e artefato baixado é verificado por checksum antes de ser carregado.
- Query parametrizada sempre. Concatenar string em SQL é bug, não estilo.
- **Segredo nunca** em código, commit, log, mensagem de erro, corpo de PR ou output do agente. Chave de API do usuário não é exceção: não vai para o banco em texto puro, não aparece em log, não é ecoada em erro.
- **Áudio e transcrição não saem da máquina** a não ser que o usuário tenha escolhido explicitamente um provedor de nuvem.
- Dependência nova: conferir se já existe algo equivalente no projeto antes de adicionar. Toda dependência nova entra no bundle desktop do usuário.
- `@animateicons/react` custa **+527 kB fixos** no bundle e não faz tree-shaking: os ícones são `forwardRef(...)` no topo do módulo sem anotação `/*#__PURE__*/`, então o Rollup não descarta nenhum, mesmo com `sideEffects: false` declarado. Importar um ícone traz os 293. Custo aceito por ser app desktop — o preço é parse no startup, não download por visita. Não usar isso como precedente para lib de web.
- **Ícone animado não vai dentro de `<button>`**: ele renderiza `<div><svg/></div>`, e `button` só aceita *phrasing content* — `div` é flow content, o que torna o HTML inválido. Dentro de botão, usar `lucide-react`, que renderiza `<svg>` direto. O animado fica em contêiner de bloco: empty state, cabeçalho de card, estado vazio de lista.

## Dados e migrations

- Não há ferramenta de migration. O schema vive em `src-tauri/src/db/mod.rs`, no método `migrate()`, com `CREATE TABLE IF NOT EXISTS`, executado na abertura do banco.
- O banco é o **arquivo local do usuário** (`%APPDATA%/Vesper/vesper.db` no Windows, equivalente em `dirs::data_dir()` nos outros SOs). Não existe ambiente onde "é só recriar": mudança destrutiva de schema apaga reunião de gente real.
- Alteração destrutiva (drop, rename, mudança de tipo com perda) exige confirmação humana explícita **e** caminho de migração dos dados existentes.

## Ambiente de QA/staging

**Não existe** — é aplicativo desktop, não serviço deployado. Validação visual exige rodar a app localmente, o que é exceção sob pedido explícito. Sem isso, verificar por teste e build, e **dizer no relato** que o visual não foi conferido.

## O que o agente não faz sem pedir

- Subir a app, servidor de desenvolvimento ou processo de watch por iniciativa própria
- Deletar arquivo, branch, tabela ou dado
- Mexer em configuração de CI/CD
- Mergear para `main`
- Instalar dependência nova quando já existe equivalente no projeto
- Reescrever histórico (`rebase`, `push --force`, `reset --hard`) em branch compartilhada
- Gerar, commitar ou imprimir chave privada de assinatura

## Skills

Cada skill em `.claude/skills/NOME/SKILL.md`, com frontmatter `name` + `description`. A `description` é o único texto sempre em contexto e é ela que decide a invocação — se estiver boa, este arquivo **não** precisa de uma tabela "pediu X → use skill Y".
