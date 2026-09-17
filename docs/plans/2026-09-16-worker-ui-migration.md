# Migração das UIs de worker para o design system do ADE

Estado em 2026-09-16, branch `feat/hidden-agents-and-browser-improvements`, nada commitado.
Origem: auditoria do design system (Claude Doc "Auditoria do Design System do ADE").

## O que já está feito (fases 0–6)

| Fase | Entrega | Onde verificar |
| --- | --- | --- |
| 0 · Docs | `ade/skills/design-system.md` é o único canônico (370 linhas: princípios, tokens reais, tipografia, tabela de números, componentes compartilhados, padrões de UX, Do/Don't); `ade/web/DESIGN.md` virou ponteiro de 12 linhas; `injectable-ui.md` 782 → 464 (só contrato de entrega, `buildWorkerUi`, lint, subpaths, lucide, `useConfirm`); `design-console-ui.md` 627 → 360 (só responsividade e forms, sem cópias); SOPs e `ade/agents/console-ui.md` atualizados (33 workers, `ade`/`ide`, coluna strict) | `ade/skills/*.md`, `docs/sops/console-ui-conformance.md` |
| 1 · Pacote | `@iii-dev/console-ui` ganhou `./format` (`formatRelative`, `formatDuration`, `formatBytes`, `errorMessage`, `errorCode`, `copyText`), `./hooks` (`useContainerNarrow`, `usePaneState`, `useCopyFlash`, `useWorkerLive`), 20 exports novos (`Eyebrow`, `SearchField`, `Toolbar`/`StatusBar`, `MetaRow`/`ActionLine`, `BottomSheet`, `Kbd`/`KeyCombo`, `LiveRegion`…), `Tooltip label=`, `useConfirm()`; `lucide-react` no import map (externo nos workers); 20 stories novas | `packages/console-ui/README.md`, `index.d.ts`, `hooks.d.mts`, `format.d.mts` |
| 2 · Build | `buildWorkerUi()` (`packages/console-ui/build-worker-ui.mjs`) em 33 workers: externals, `assertScoped`, `checkTokens`; `tsconfig.worker-ui.json`; `catalog:` do pnpm (react, react-dom, @types/*, esbuild, typescript, vitest, zod, lucide-react) | `<worker>/ui/build.mjs` (2–4 linhas) |
| 3 · Correções | 6 tokens inexistentes, 8 seleções com accent, 15 focos com accent cru, 15 `window.confirm`, CSS global do `iii-directory`/`onboarding`, ícones < 16px, paleta dos chips de subagente, teste de seleção | `ade/web/src/lib/*-conformance.test.ts` verdes |
| 4 · Lint | `lint-worker-ui.mjs`: 15 regras (3 erros: `no-window-dialogs`, `icon-size`, `accent-selection`; 12 avisos), roda no build; CLI `--all`; keyframes `iii-ui-spin`/`iii-ui-pulse` | `node packages/console-ui/lint-worker-ui.mjs --all` |
| 5 · Três grandes | `browser` (CSS 3.114 → 1.861), `iii-directory` (3.106 → 1.670, bundle 734 → 418 KB), `ide` (5.120 → 2.351, bundle 2,0 MB → 1,17 MB); os três com `lint: { strict: true }` e 0 avisos | `browser/ui`, `iii-directory/ui`, `ide/ui` |
| 6 · ADE | Layout de pane em container queries (`PageShell` virou container; utilitários de viewport 482 → 433, os 433 restantes são chrome de telefone a 640px, allowlisted com motivo), `lowercase` 92 → 0, `uppercase` fora de `Eyebrow.tsx` 183 → 0 (`Eyebrow size="lg"` + `eyebrowClassName`), texto < 11px 101 → 4 (ticks de diagrama), bordas cromáticas 22 → 5, durações literais 11 → 0; teste-catraca de breakpoints; 2.322 testes passando | `ade/web/src/lib/viewport-breakpoint-conformance.test.ts`, `typography-conformance.test.ts` |

## Receita para migrar um worker (o que a fase 5 fez, em ordem)

1. Baseline: `node packages/console-ui/lint-worker-ui.mjs <worker>/ui` e `wc -l <worker>/ui/styles.css`.
2. Código: trocar cópias locais pelos exports compartilhados. Procurar por: `useContainerNarrow`, `readStored`/`writeStored`, `formatBytes`/`formatMtime`/`formatDuration`/`errorMessage`, `navigator.clipboard.writeText`, `use*Live`, `Chip`/`StatusPill`/`MetaRow`/`ActionLine`, `BackButton`/`LiveDot`/`LivePill`, `FunctionIdLabel`, `icons.tsx`/`<svg`, classes `*-empty`/`*-skeleton`/`*-toolbar`/`*-statusbar`/`*-nav-row`, `role="tablist"` à mão, `<input type="search">`.
   Destinos: `@iii-dev/console-ui/hooks`, `@iii-dev/console-ui/format`, `lucide-react`, `Badge`/`Chip`/`MetaRow`/`ActionLine`, `EmptyState`/`Skeleton`/`StatusPanel`, `SearchField`, `Toolbar`/`StatusBar`, `List`/`ListItem`/`uiClasses.list*`/`tree*`, `Tabs`/`SegmentedControl`, `IconButton`, `StatusDot`, `TerminalCommandLine`/`TerminalStream`/`AnsiText`, `Eyebrow`, `uiClasses.spin`/`pulse`.
3. CSS: apagar seletores dos componentes trocados; raio 6px (9999px só no que é redondo); fontes só `var(--font-sans|mono|code)`; texto ≥ 11px; `text-transform` → eyebrow; durações → `var(--motion-*)`; `@media` de viewport → `@container`; sombras → `var(--shadow-*)`; hex → tokens (identidade → `--color-glyph-*`); cabeçalho ≤ 4 linhas.
4. Checagem de classes mortas nos dois sentidos (classe usada no TSX sem seletor; seletor sem uso). Script de referência: o que os agentes deixaram no scratchpad da sessão, ~40 linhas: extrai tokens de `className` e seletores `.<prefixo>-*` do CSS e imprime a diferença.
5. `lint: { strict: true }` no `build.mjs`; cada aviso restante vira correção ou `/* lint-allow <regra> */` com motivo na linha.
6. Verificar: `npx tsc --noEmit` e `npx vitest run` no `ui/`, `pnpm --dir <worker>/ui build` (roda `assertScoped` + `checkTokens` + lint), `pnpm -C ade/web exec vitest run src/lib/selection-conformance.test.ts src/lib/icon-size-conformance.test.ts` (varrem os workers).

Um worker por PR. Não mudar comportamento; preservar teclado, ARIA, guards de rascunho sujo e testes.

## Ordem dos 30 workers restantes

Critério: exposição ao usuário × avisos do lint × linhas de CSS × duplicações conhecidas da auditoria. Números de `lint-worker-ui.mjs --all` em 2026-09-16 (após as fases 1–5).

### Onda 1 — os cinco que mais pesam (~2–3 dias cada)

| # | Worker | Avisos | CSS | Duplicações conhecidas a remover |
| --- | --- | ---: | ---: | --- |
| 1 | `ade/ui` (catálogo do próprio console, scope `console`) | 87 (hex 39, font-size 15, radius 14) | 2.319 | É a "referência" citada pelas skills; `LiveDot`, `errorMessage`, 2 clipboards crus, `HttpTester`; fallbacks `var(--x, rgba(...))` copiando valores de token (40) |
| 2 | `database` | 48 (font-size 18, hex 18) | 2.419 | `useContainerNarrow`, `FunctionIdLabel`, `BackButton`, `icons.tsx` (236 linhas), `Row({label,value})` → `SettingsRow`, `formatMs`/`formatBytes`, `useCopyFeedback`, 7 toolbars `db-*-bar` → `Toolbar`, "derived inks" (20 hex por contraste AA: propor token `--color-warn-strong`/`--color-alert-strong` no pacote em vez de literais); manter `result-grid.tsx` |
| 3 | `security-scan` | 68 (font-size 34, radius 19) | 1.800 | `useContainerNarrow`, `classNames` ×2, `formatRelativeTime`/`formatTimestamp`, `icons.tsx` (90), `.is-selected` → `data-selected`, `useSecurityRunsLive` (302 linhas) → `useWorkerLive` |
| 4 | `sandbox-code-runner` | 52 (case-transform 23, font-size 19) | 2.215 | `sandbox-family/ansi.tsx` (214) → `AnsiText`/`TerminalStream`; `shared.tsx` chrome de terminal → `TerminalCommandLine`; `Chip`/`StatusPill`; `clipboard.ts` (já é a implementação boa; trocar pela do pacote); `ErrorView.tsx` (3ª cópia do card de erro); glifos `×`/`→`; `icons.tsx` (117) |
| 5 | `eval` | 46 (font-size 28, case-transform 10) | 1.318 | Tablist à mão em `page/index.tsx:244-273` → `Tabs`; `errorMessage`; clipboard cru; mono no root (`.eval-ui`) → só em valores; glow local → `pulse-dot`; foco accent |

### Onda 2 — médios (~1 dia cada)

| # | Worker | Avisos | CSS | Duplicações conhecidas |
| --- | --- | ---: | ---: | --- |
| 6 | `storage` | 32 | 1.229 | `useContainerNarrow` ×3, `formatBytes`, `errorMessage`, clipboard cru, `storage-ui-row-*` → `ListItem`, 10 `<svg>` |
| 7 | `memory` | 28 | 903 | `useContainerNarrow`, `readStored`, `useMemoryLive` (197) → `useWorkerLive`, `BackButton`, `icons.tsx` (169), `Cmd+S` |
| 8 | `editor` | 27 (font-family 10) | 725 | 10 stacks mono literais → `var(--font-code)`; `build.mjs` próprio (mantém plugins do shiki) |
| 9 | `github` | 24 | 697 | `useContainerNarrow` ×3 (`narrow.ts`), `readStored`, `BackButton`, `formatDuration`, `humanBytes`, `icons.tsx` (111), `gh-ui-toolbar` |
| 10 | `onboarding` | 24 (tailwind-in-worker 13) | 274 | Classes Tailwind em markup injetado (não compilam): trocar por componentes/recipes; 5 sombras e 4 durações literais |
| 11 | `harness` | 22 | 603 | `context-chip/index.tsx` (931 linhas): popover com `createPortal` + `useMediaQuery` próprio → `Dialog`/`Tooltip`; clipboard cru |
| 12 | `state` | 20 | 675 | `useContainerNarrow`, `ColumnBody` skeleton à mão → `Skeleton`, `LiveDot`, `BackButton`, `state-ui-nav-row` → `ListItem`, `Cmd+S` |
| 13 | `computer` | 19 | 634 | `useContainerNarrow`, `useSessionsLive` + `events.ts` (cópia literal do browser) → `useWorkerLive`, `LivePill`, `BackButton`, `FunctionIdLabel`, `formatJson`, `cn.ts`, `useLiveFrames` (parcialmente igual ao do browser) |

### Onda 3 — pequenos (½ dia cada; um PR pode agrupar 2–3)

| # | Worker | Avisos | Notas |
| --- | --- | ---: | --- |
| 14 | `cron` | 17 | 8 `<svg>`, 2 media queries, clipboard cru; já usa `uiClasses` |
| 15 | `worktree` | 16 | `useContainerNarrow`, `useWorktreesLive` (é a melhor das 7 cópias; virou a base do `useWorkerLive`), `Row` → `SettingsRow`, `formatMs`, clipboard, `icons.tsx` (139) |
| 16 | `code-runner` | 15 | `lib/shared.tsx` (541 linhas) → átomos de terminal do pacote; `copyText` |
| 17 | `voice` | 11 | `formatBytes` decimal, `errorMessage`, `icons.tsx` (176), 3 keyframes |
| 18 | `compose-ui` | 10 | `icons.tsx` (140); já usa `uiClasses`/`ListItem` |
| 19 | `canvas` | 9 | `FunctionIdLabel`, `relativeTime`, `errorMessage`, resizer sem teclado (`MermaidPane.tsx`), `icons.tsx` (121); `build.mjs` próprio |
| 20 | `pdf` | 6 | radius |
| 21 | `tailscale` | 6 | `icons.tsx` (173), `#ffffff`, clipboard cru |
| 22 | `provider-openai-codex` | 5 | radius, 1 media query |
| 23 | `llm-router` | 3 | radius, 1 hex |
| 24 | `kanban` | 2 | 1 `<svg>`, 1 media query; `usePaneState`/`useContainerNarrow` locais viraram a base das versões do pacote — trocar pelos imports |
| 25 | `web` | 1 | 1 media query |

### Já limpos — só ligar `strict`

`a2ui`, `claude-code`, `context-manager`, `pi`, `vscode`: 0 avisos. Colocar `lint: { strict: true }` no `build.mjs` de cada um e, no `vscode`, trocar `icons.tsx` (96 linhas) por `lucide-react` na passada.

## Lacunas do pacote apontadas pelos três primeiros (fazer antes ou durante a onda 1)

- `SearchField`: repassar `data-*`/`aria-*`/`autoFocus` para o `<input>` (ide e iii-directory usaram `setAttribute` num callback ref).
- `MetaRow`: `items` sempre antes de `children`; falta tom por item (warn) — browser/iii-directory contornaram com `Chip tone`.
- `StatusPanel`: sem slot de ação nem `role` — botões de retry ficaram em `detail`.
- `EmptyState`: sem variante compacta (dentro de card) e só uma ação.
- `Toolbar`: sem `as` (barra de endereço precisa ser `<form>`) nem orientação vertical (rail do ide continua local).
- `ListItem`: emite `aria-pressed` sempre; linha não pode ser `<button>` quando há ações internas (downloads do browser).
- `Table`: zera padding da primeira/última célula.
- Faltam: `useDebounce`, `Checkbox`, `SplitPane` (três implementações locais: `ide/ui/src/page/use-split-drag.ts`, `iii-directory` browser.tsx, `canvas` MermaidPane), `unwrapEnvelope` (três cópias: `ide/ui/src/lib/envelope.ts` é a mais completa).
- Token de contraste para status em texto pequeno (o motivo dos 20 hex do `database`).

## Commits sugeridos

Nada foi commitado. Sugestão de cortes: (1) pacote + stories + import map, (2) build compartilhado + catalog + 33 `build.mjs`/`tsconfig`/`package.json`, (3) correções imediatas + lint + keyframes, (4) `browser/ui`, (5) `iii-directory/ui`, (6) `ide/ui`, (7) `ade/web` container queries + eyebrows + catraca.

## Estado em 2026-09-17 — tudo migrado

Executado nesta data, na branch `feat/console-ui-consolidation`, ainda sem commit:

- **Lacunas do pacote** (todas): `SearchField` repassa `data-*`/`aria-*`/`autoFocus`; `MetaRow` item com `tone`; `StatusPanel` com `action` e atributos/`role`; `EmptyState` `compact` + `actions[]`; `Toolbar` `as`/`orientation`; `ListItem as="div"`; `Table inset`; `Checkbox` (recipe `iii-ui-checkbox*`, story, manifest, shim regenerado); `useDebounce`, `useSplitDrag` (ide, iii-directory e canvas usam), `unwrapEnvelope` (13 cópias apagadas); tokens `--color-warn-strong`/`--color-alert-strong`; `useWorkerLive` aceita `{ type, config }` (trigger `stream` do security-scan); `EyebrowProps` com `as` completo e `size`; byte NUL literal em `hooks.mjs` virou `'\u0000'`.
- **32 workers em `lint: { strict: true }`, 572 → 0 avisos** (`lint-worker-ui.mjs --all`). `editor` foi removido no commit de consolidação. `styles.css` somados: 24.946 → 20.761 linhas.
- `lint-allow` restantes (com motivo na linha): diagramas em `<svg>` (github DAG, worktree, memory graph, database ERD, compose-ui topology), marcas de terceiros (GitHub, Discord/X/LinkedIn no onboarding), `.wt-ui-edge-label` 10px (rótulo de diagrama), `.kanban` 16px em telefone (`viewport-media`), três animações de chegada mais longas que qualquer token (`ade` 2s, `state` 1.4s, `database` 4s).
- Teste de seleção (`ade/web/src/lib/selection-conformance.test.ts`): linhas do `editor` e dos seletores que viraram `ListItem`/`Tabs` (storage ×2, eval ×2, memory, state) removidas.
- `voice/ui`: 2 testes de `dictation-wake-lock` já falhavam no HEAD (reproduzido em worktree limpa).
- Script de classes mortas agora vive em `packages/console-ui/dead-classes.mjs`.
- Mudanças visíveis aceitas como parte da migração: tempos relativos no vocabulário do `formatRelative` (`5m`, `just now`), bytes binários (`1.0 KiB`), popover do context-chip do harness virou `Dialog`/`BottomSheet`, rótulos perderam `uppercase`/`capitalize` fora de `Eyebrow`.
