---
name: qa
description: Use quando precisar ver a Vesper rodando para conferir uma mudança antes da PR — "testa na app", "confere a tela", "abre pra eu ver", "valida rodando", ou quando a mudança só se prova com a app de pé. Sobe o build QA isolado em segundo plano e o dirige pelo DevTools do WebView, sem tocar mouse, teclado, foco ou tela de quem usa a máquina.
---

# QA — a branch rodando, sem encostar no que é do usuário

Isolamento e regras estão no `AGENTS.md` ("Ambiente de QA"). Aqui fica o como.

## Fluxo

```bash
npm run qa -- build     # da worktree da branch; encerra a QA que estiver rodando
npm run qa -- start     # segundo plano; lista as páginas abertas
npm run qa -- cdp ...   # testar
npm run qa -- stop      # teste só do agente termina aqui
```

A primeira build de uma versão de llama/whisper compila o C++ inteiro. Depois, `C:\vt` guarda isso para todas as worktrees e a build recompila só a Vesper e o front.

## Dirigindo

```bash
npm run qa -- cdp shot tela.png                    # screenshot só da página
npm run qa -- cdp click <data-testid>
npm run qa -- cdp key Tab                          # Shift+Tab, Enter, Escape, ArrowDown, Space
npm run qa -- cdp type "texto"                     # no elemento com foco
npm run qa -- cdp eval "document.activeElement?.dataset.testid"
npm run qa -- cdp eval "window.__TAURI_INTERNALS__.invoke('get_settings')"
npm run qa -- cdp shot card.png --overlay          # a janela do card
```

- **Clique só por `data-testid`.** Seletor largo erra alvo mesmo dentro da página: `[aria-label*=Fechar]` já casou com o fechar da janela e derrubou a app. Elemento sem testid ganha um na própria mudança.
- `click` recusa quando outro elemento cobre o alvo e diz qual. `MISS` e `COVERED` são falha do teste, não ruído.
- Screenshot se lê com a ferramenta de imagem e se mede no pixel. Box model passa em review e falha na tela.
- `invoke` direto testa o backend sem diálogo nativo: importar áudio é `import_audio` com `path`.

## Armadilhas

- **Uma QA por vez.** Toda worktree constrói o mesmo `C:\vt\release\vesper.exe`; o que roda é a última build.
- **Perfil QA começa vazio**, então a primeira execução cai no onboarding, e os botões dele não têm `data-testid`. Para passar sem clicar por texto: `invoke('get_settings')`, devolver o objeto em `invoke('complete_onboarding', { settings })` e `location.reload()`. `npm run qa -- reset` volta ao perfil vazio.
- **Atalho global aparece como indisponível**: a QA não registra nenhum, de propósito. Tecla dentro da janela chega por `cdp key`.
- **`eval` espera no máximo 15 s.** Operação longa (importar, transcrever, resumir) se dispara sem `await`, guardando o resultado em `window`, e se consulta depois com outro `eval`.
- **Lançada de dentro do Claude desktop, a QA grava no contêiner do pacote.** O app é MSIX, e o Windows redireciona o que processos filhos escrevem em `%APPDATA%`/`%LOCALAPPDATA%` para `...\Packages\Claude_*\LocalCache\`. De dentro, o caminho continua sendo `%APPDATA%\Vesper QA`; no Explorer ele não existe. Os hard links funcionam do mesmo jeito.
- **Porta 9333 ocupada** faz o `start` recusar. Descobrir quem está nela antes de mexer em qualquer coisa.
- **Ninja e LLVM**: o `build` põe no PATH o LLVM de `C:\Program Files\LLVM\bin` e o Ninja das Build Tools. Faltando um, ele diz qual.

## Gravação de verdade

Só quando a mudança mexe em áudio. Poucos segundos, transcrição local, gravação apagada ao fim. Conferir sinal, duração e estados — **sem ler nem relatar o que foi dito**: o áudio do sistema pega o que estiver tocando na máquina, até uma chamada de quem está usando.

## Terminar

- Teste do agente: `npm run qa -- stop`.
- Para uma pessoa conferir: deixar rodando e dizer no relato o que olhar.
- O relato separa o que foi visto rodando do que não foi.
