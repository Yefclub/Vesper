---
name: design
description: Use quando for construir ou ajustar UI — layout, spacing, motion, animação, tipografia, cor, sombra, profundidade, estado de foco, feedback de interação, empty state. Entrega as regras de craft que separam "funciona" de "parece feito por quem liga pra detalhe".
---

# Design — padrão de exigência para UI

UI que compete com produto de mercado: "funciona" não basta. Regras destiladas do acervo pessoal do Yef; os ids entre parênteses são do acervo — buscar lá para contexto completo antes de aplicar algo que não entendeu.

Fontes-âncora: Emil Kowalski `c81b802df` · Web Interface Guidelines `c81b86af9` · Safe Rules (Hobday) `c81b906cd` · Shadows (Comeau) `c81b8d34f` · Invisible Details (Rauno) `c81b857604`.

## Motion

- Interação responde com `ease-out` e **< 300ms** (≤ 200ms pra sentir imediato). Animação proporcional ao trigger: dialog scale ~0.8→1 (nunca 0→1), press de botão ~0.96 (nunca 0.8) (`c81b802df`, `c81b86af9`)
- **Nunca animar ação frequente ou iniciada por teclado** — abrir menu de contexto, add/delete em lista, hover trivial. Repetida 100×/dia, animação vira atrito. Raycast não anima e parece certo (`c81b802df`)
- Performance: animar só `transform` e `opacity` (composite-only). Main thread ocupada → CSS/WAAPI, não rAF/JS. Loop pausa fora da viewport (`c81b802df`, `c81b86af9`)
- Interruptível sempre: CSS *transition* > *animation*. Gesto aplica delta imediatamente; animação só depois do threshold (`c81b802df`, `c81b857604`)
- `prefers-reduced-motion`: fallback animando só opacity. Troca de tema **não** dispara transition (`c81b86af9`)
- Compostas: blur + scale + opacity juntos (`c44c2e01…`). `height:auto` → grid `0fr→1fr` + overflow hidden, ou `interpolate-size` (`c1973f6f…`). Troca de view → View Transitions API (`cd5f64f2…`)
- Consistência espacial: elemento entra **de onde pertence** e sai **para onde vive** (drawer volta pro trigger). Easing e duração coerentes com a personalidade do produto inteiro (`c81b857604`, `c81b802df`)

## Tipografia

- Headline: letter-spacing negativo −1% a −3%. Espaçamento de letra e linha inversamente proporcional ao tamanho da fonte (`c6dac441…`, `c81b906cd`)
- Body ≥ 16px (também evita zoom do iOS em input). Linha 60–80 caracteres. Weight ≥ 400; heading médio 500–600. **Sem mudança de weight no hover** (layout shift). `tabular-nums` em tabela e timer. Máximo 2 typefaces (`c81b906cd`, `c81b86af9`)

## Cor, profundidade e sombra

- Nunca preto ou branco puro — near-black/near-white. Saturar neutro com a cor da marca (< 5% sat em HSB). Não misturar neutro quente com frio (`c81b906cd`)
- Contraste alto **só** no que precisa de atenção; estrutura e decoração com o mínimo. Ícone ao lado de texto → reduzir contraste do ícone, ele pesa mais (`c81b906cd`)
- Sombra: luz única global (mesma razão offset-X/Y na página inteira). Elevação = offset↑ + blur↑ + opacidade↓, blur ≈ 2× offset. **Layering** de 3-5 sombras empilhadas pra realismo. Cor da sombra = hue do fundo com sat/lightness ajustados — **nunca preto transparente**, lava a cor. Token de elevação com `--shadow-color` por contexto (`c81b8d34f`, `c6dac441…`)
- Dark UI: sombra funciona mal. Escolher **uma** técnica de profundidade e manter no app inteiro. "Mais perto do usuário = mais claro" vale em dark e light. Container vs fundo dentro de ~12% de brilho (dark) / ~7% (light) (`c81b906cd`)
- Borda 1px: mais clara que **ambos** os fundos (dark) ou mais escura que ambos (light). Intermediária borra a aresta (`c81b906cd`)

## Layout e espaçamento

- Escala única de spacing e tamanho (múltiplos de 8). Padding externo do container ≥ padding interno entre itens. Radius aninhado = externo − distância entre as bordas (`c81b906cd`, `c6dac441…`)
- **Tudo deliberado**: se alguém apontar qualquer pixel, existe um porquê. Alinhar pelo olho quando o centro óptico ≠ centro matemático. Nada desalinhado de tudo (`c81b906cd`)
- Hierarquia em triângulo: mais pesado primeiro, na borda externa. Complexo sobre simples (fundo complexo → conteúdo simples, e vice-versa). Nunca 2 hard divides adjacentes (`c81b906cd`)

## Interação e feedback

- Input: label clicável foca o input. `<form>` + Enter submete. `type` correto. Decoração prefix/suffix posicionada absoluta **e** focando o input. Validação HTML nativa quando couber (`c81b86af9`)
- Botão desabilita após submit (dupla requisição). Disabled **não** ganha tooltip (inacessível). Toggle aplica na hora, sem confirmação. Dropdown abre no `mousedown` (`c81b86af9`)
- Feedback perto do trigger: checkmark inline no copy, não toast global. Erro de form destaca os inputs errados. Optimistic update com rollback + aviso em erro (`c81b86af9`)
- Área de ação responde **antes** do gesto completar (drop zone: borda + glow + copy no dragover). Progresso honesto (% + estimativa, não spinner). Retry inline sem perder estado. N itens = progresso e retry individuais (`c3e856eb…`)
- Empty state sempre com CTA de criação. Focus ring com `box-shadow` (respeita radius; `outline` não). Lista focável navega com ↑↓. Hover state só em `@media (hover: hover)`. Icon-only com `aria-label` (`c81b86af9`)

## Processo

- **Referência real antes de construir**: buscar no acervo + abrir tela semelhante no ambiente de QA **antes** de desenhar UI nova. Construir com referência e intenção, não de memória (`c59a11a5…`)
- Ícone: não parar no default do projeto — Phosphor/Iconoir/Tabler/Solar via `unplugin-icons` com tree-shaking quando fizer sentido (`c72a3397…`). Micro-som de UI é opção barata (`c52d490e…`)
- Revisar UI no dia seguinte com olhos frescos quando der. Animação é tentativa-e-erro paciente (`c81b802df`)
