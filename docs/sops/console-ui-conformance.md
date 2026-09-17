# Console UI conformance inventory

This inventory keeps native Console and runtime-injected worker UI on one
shared visual and interaction contract. Update it whenever a repeated UI
pattern is promoted to `@iii-dev/console-ui`, a worker keeps a local control,
a worker turns strict lint on, or the selection/motion rules change.

## Shared contract

- Structure: `List`/`ListItem`, `Card`, `Panel`, `Chip`, `Badge`,
  `IconButton`, the semantic `Table` family, and equivalent stable
  `uiClasses` recipes.
- Chrome: `Eyebrow` (the mono caps label; `uiClasses.eyebrow`), `Toolbar` /
  `StatusBar` (raised and quiet strips with an `end` slot), `SearchField`,
  `MetaRow`/`ActionLine` (a card's metadata strip and icon-led action lines),
  `Kbd`/`KeyCombo`, `LiveRegion`, `EmptyState`/`Skeleton`/`StatusPanel`.
  Workers do not keep local versions of these.
- Choice: shared line `Tabs` or `SegmentedControl variant="tabs"` for peer
  content views, `SegmentedControl variant="radio"` for a persisted exclusive
  choice, `Select` for small finite lists, and `Selector` for searchable
  single choice. Line tabs use a neutral underline, 600 weight, natural case,
  and semantic icons by default.
- Overlays: shared `Tooltip` (`label=` is the one-line form), `Dialog`,
  `ConfirmDialog`, `DropdownMenu`, `Select`, `Selector`, and `BottomSheet`
  preserve injected worker scope through their portals. Confirmation is
  `useConfirm()` or `ConfirmDialog`, never `window.confirm` (a lint error);
  the native prompt is only for browser reload or close.
- Hooks and formatters: `@iii-dev/console-ui/hooks` (`useContainerNarrow`,
  `usePaneState`, `useCopyFlash`, `useWorkerLive`) and
  `@iii-dev/console-ui/format` (`formatRelative`, `formatDuration`,
  `formatBytes`, `errorMessage`, `errorCode`, `copyText`) bundle into the
  worker asset; local copies are migration debt, not retained surface.
- Images: a picture the user may want to inspect opens the shared
  `ImageViewer` through an `ImageThumbnailButton`; no worker ships its own
  lightbox, zoom or pan. Captions carry the attachment name or a relative
  path, never a host path.
- Selection: neutral `surface-selected` + `ink`, with an optional neutral
  `edge`. Accent is reserved for primary actions, form focus, live activity,
  and semantic domain data.
- Elevation: `--shadow-raised` (cards), `--shadow-floating` (overlays),
  `--shadow-lift` (crisp-edged instrument surfaces such as the chat composer)
  and `--shadow-keycap` (key caps) are the only shadows; each is a complete
  theme-aware value used alone.
- Motion: public duration/easing tokens and motion recipes
  (`uiClasses.spin`/`uiClasses.pulse` for the shared keyframes); no
  transition on streaming, rapidly updating, dragged, resized, or
  pointer-following values; reduced motion resolves immediately.
- Typography: interface chrome is sans and authored in natural sentence/title
  case. Mono is reserved for machine-readable identifiers, paths, values,
  payloads, code, and tabular data; no panel-wide mono or CSS case transforms.
- Icons: `lucide-react`, an external shared with the console, at the 16 px
  baseline; no inline `<svg>` copies and no `icons.tsx` (lint `no-inline-svg`,
  `icon-size`). Icon-only actions use `IconButton` so the accessible label and
  shared tooltip remain present.
- Tables: `TableViewport`/`TableFrame` owns responsive overflow; semantic table
  parts use natural-case sans headers and horizontal dividers without an outer
  card or border. Comfortable density is the page default, compact density is
  for chat, and mono is applied only to technical cell values.
- Layout responds to the pane: `@container` and `useContainerNarrow`, never a
  viewport `@media` in worker CSS (lint `viewport-media`).

Every worker builds through `buildWorkerUi` (`assertScoped`, `checkTokens`,
then `lintWorkerUi`; see `ade/skills/injectable-ui.md`). Current per-worker
lint numbers: `node packages/console-ui/lint-worker-ui.mjs --all`. The
migration recipe and order: `docs/plans/2026-09-16-worker-ui-migration.md`.

## Injectable worker sweep

All 32 checked-in injectable UI packages (`ls -d */ui/package.json`) are
listed; every one builds with strict lint since 2026-09-17. "Domain adapter" means a local component still adds information
architecture or semantics; it must compose shared controls/tokens and is not
permission to fork base hover, selection, tooltip, or selector behavior.
"Strict" is `lint: { strict: true }` in the worker's `build.mjs`.

| Worker UI | Strict | Shared/conformance result | Retained domain surface |
|---|---|---|---|
| `a2ui` | yes | Shared page chrome, sidebar, lists, overlays, controls, and neutral selection; `unwrapEnvelope` from the format subpath | Validated A2UI component graph rendering and workspace export |
| `ade` (scope `console`) | yes | Migrated: format/hooks subpaths (`errorMessage`, `unwrapEnvelope`, `useCopyFlash`, `formatRelative`), `StatusDot`, `StatusPanel`, `Skeleton`, `EmptyState`, `Badge`/`Chip`/`Eyebrow`, `lucide-react`; token-value fallbacks dropped; sans root, mono on machine values | Key/value catalog chips, trigger metadata, injectable-UI toggle board |
| `browser` | yes | Migrated: shared hooks/format/`lucide-react`, `Toolbar`/`StatusBar`, `Eyebrow`, `MetaRow`/`ActionLine`, `BottomSheet`, `host.overlays` live preview; neutral rail/config selection | Browser feed, device toolbar, annotations, element references |
| `canvas` | yes | Migrated: `formatRelative`/`errorMessage`/`unwrapEnvelope`, `lucide-react`, `useSplitDrag` on the editor/preview separator (keyboard resize), `usePaneState`, `StatusDot`, eyebrows, `uiClasses.pulse`; own `build.mjs` keeps the mermaid/excalidraw vendor bundles | Infinite canvas gestures and graph semantics |
| `claude-code` | yes | Thin wrapper over the shared `@iii-workers/agent-terminal-ui` page (same page as `pi`); no local CSS | Agent terminal bound to a `shell::pty` session |
| `code-runner` | yes | Migrated: `TerminalStream`, `MetaRow`/`Badge`/`Chip`/`Eyebrow`, `StatusPanel`, `Input`/`Checkbox` config form, `unwrapEnvelope`/`useCopyFlash`/`formatDuration` | Execution-result composition |
| `compose-ui` | yes | Shared page chrome with `PageSidebar`, `ListItem`, `IconButton`, `Table` family, `Badge`/`Chip`/`StatusDot`, `EmptyState`/`Skeleton`, `TerminalStream`, `ConfirmDialog`; `lucide-react`, `uiClasses.spin`, `--shadow-floating`; topology edge `<svg>` allowlisted | Topology graph, project health stats, container table with log tails, worker package declaration, daemon projects table |
| `computer` | yes | Migrated: `useContainerNarrow`, `useWorkerLive` for lifecycle events, `StatusDot`, `IconButton`, `StatusPanel`, `EmptyState`, `Skeleton`, `StatusBar`, `Eyebrow`, `lucide-react`; `useLiveFrames` stays local (stream-fed frames) | Remote-session viewport and controls |
| `context-manager` | yes | Minimal shared renderer; no selectable navigation | Context accounting payload |
| `cron` | yes | Shared page chrome with `PageSidebar`, `List`/`ListItem`, `Tabs`, `Table` family, `Select`/`SegmentedControl`, `IconButton`, `Dialog`/`DropdownMenu`, `RawValueInput`; `SearchField`, `lucide-react`, `uiClasses.spin`, `useContainerNarrow`, `copyText`/`errorMessage`, container queries | Schedule composer and run history |
| `database` | yes | Migrated: `useContainerNarrow`, `IconButton`, `lucide-react`, `useCopyFlash`, format subpath, `MetaRow`, `Toolbar`/`StatusBar`, `PageBody`/`PageMain`, `Panel`, `Table` family, `SearchField`, `Select`, `Badge`/`Eyebrow`, `uiClasses.pulse`, `--color-*-strong` status text; `result-grid.tsx` stays; ERD edge `<svg>` allowlisted | Data grid, query plan, ERD, health metrics, multi-filter chips |
| `eval` | yes | Migrated: `Tabs`, `SearchField`, `Checkbox`, `Button`/`IconButton` + `useCopyFlash`, `StatusDot pulse`, `StatusPanel`, `Eyebrow`, field recipes, format subpath, `lucide-react`; mono only on values | Session comparison is intentionally multi-select |
| `github` | yes | Migrated: hooks/format subpaths, `lucide-react`, `Toolbar`, `IconButton`, `EmptyState`, `Skeleton`, `Eyebrow`; lane palette on `--color-glyph-*`; commit DAG `<svg>` and the GitHub mark allowlisted | Commit graph and repository status semantics |
| `harness` | yes | Migrated: context-chip popover → `Dialog` (desktop) / `BottomSheet` (phone), `useCopyFlash`, `MetaRow`/`Badge`/`Table` family in the metrics card, `unwrapEnvelope` | Harness run/approval payloads |
| `ide` (scope `ide`) | yes | Migrated: shared tooltips, `CodeEditor`/`FileDiff`, terminal atoms, `DirectoryPicker`, shared hooks/format/icons; `.xterm` vendor CSS allowlisted via `allowUnscopedSelectors`; neutral editor/terminal tabs | Terminal, filesystem, source control, timeline |
| `iii-directory` | yes | Migrated: `SearchField`, `MetaRow`/`ActionLine`, `Kbd`/`KeyCombo`, `CollapsibleCard`, shared hooks/format/icons; neutral navigation selection | Registry/document editing workflows |
| `kanban` | yes | Shared page chrome, `List`/`ListItem`, `Card`, `Tabs`, `Selector`/`Select`, `ConfirmDialog`, `CodeEditor`, `Markdown`, `SettingsDeck` form; imports `usePaneState`/`useContainerNarrow`/`errorMessage`/`formatRelative`/`unwrapEnvelope` from the package, `lucide-react` | Board columns, ticket detail, function and trigger renderers |
| `llm-router` | yes | Shared finite selects and configuration controls; tokens only | Provider/model configuration semantics |
| `memory` | yes | Migrated: `useWorkerLive`, `usePaneState`, `useContainerNarrow`, `List`/`ListItem`, `IconButton`, `lucide-react`, `Skeleton`, `EmptyState`, `StatusPanel`, `Chip`/`Badge`/`Checkbox`, `Panel`, `CollapsibleCard`, `Card`, eyebrows, format subpath; graph canvas `<svg>` allowlisted | Memory graph and recall rules |
| `onboarding` | yes | Migrated: Tailwind strings → scoped `ob-*` rules + `uiClasses`, `StatusDot`, `Chip`, `Input`, `Button asChild`, `StatusPanel`, `Skeleton`, `useCopyFlash`/`errorMessage`; brand-mark `<svg>` allowlisted | Guided tour spotlight over the console |
| `pdf` | yes | Shared tooltip and motion tokens; `unwrapEnvelope`, eyebrow recipe, 6px radii | Page rendering and document navigation |
| `pi` | yes | Thin wrapper over the shared `@iii-workers/agent-terminal-ui` page (same page as `claude-code`); no local CSS | Agent terminal bound to a `shell::pty` session |
| `provider-openai-codex` | yes | Shared provider-form controls; `copyText`, container query, `--color-rule-focus` | OAuth/device authentication flow |
| `sandbox-code-runner` | yes | Migrated: `TerminalStream ansi`/`AnsiText`, `TerminalCommandLine`, `Chip`/`Badge`, `StatusPanel`, `DropdownMenu`, `Table` family, `Checkbox`, `IconButton`, `useCopyFlash`, format subpath, `lucide-react`; local ANSI parser and clipboard deleted | Sandbox lifecycle, file tree, and execution streams |
| `security-scan` | yes | Migrated: `useContainerNarrow`, `useWorkerLive` (stream trigger via `{ type, config }`), `errorMessage`/`formatRelative`, `lucide-react`, `Skeleton`, `SearchField`, `StatusPanel`, `EmptyState`, `Eyebrow`, `SegmentedControl`, `uiClasses.panel`; state as `data-*` | Scan runs and findings with severity semantics |
| `state` | yes | Migrated: `useContainerNarrow`, `StatusDot`, `IconButton`, `lucide-react`, `Skeleton`, `List`/`ListItem`, `EmptyState` + `KeyCombo`, `StatusPanel`, `StatusBar`, `MetaRow`/`Eyebrow`, `formatBytes`/`errorMessage`/`unwrapEnvelope` | Progressive scope/key/value browser |
| `storage` | yes | Migrated: `useContainerNarrow`, format subpath, `lucide-react`, `PageBody`/`PageMain`, `List`/`ListItem`, `IconButton`, `Eyebrow`, `Toolbar`/`StatusBar`, `Skeleton`, `StatusPanel`, `EmptyState`, `Checkbox`, `Select`, `Chip` | Bucket/object browser |
| `tailscale` | yes | Shared page chrome with `PageSidebar`, `ListItem`, `IconButton`, `Input`/`Select`/`SegmentedControl`, `Table` family, `Badge`/`Chip`/`StatusDot`, `EmptyState`/`Skeleton`, `ConfirmDialog`; `lucide-react`, `useCopyFlash`, `errorMessage`/`formatBytes`, `uiClasses.spin` | Tailnet device table with ping paths, QR link card, netcheck and DNS facts, preference rows |
| `voice` | yes | Shared page chrome, `IconButton`, `Button`, `Badge`, `StatusPanel`, `Input`, `Table` family; chat-slot registrations; `lucide-react`, format subpath (`formatBytes`, `errorMessage`), `useCopyFlash`/`copyText`, `StatusDot pulse`, `uiClasses.spin`/`motionPanel` | Microphone capture, live partial transcript pill, segment timestamps |
| `vscode` | yes | Shared page chrome, `List`/`ListItem`, `IconButton`, `StatusPanel`; `lucide-react` | Embedded VS Code workbench frame |
| `web` | yes | Minimal shared renderer; container query | HTTP response payload |
| `worktree` | yes | Migrated: `useWorkerLive`, `useContainerNarrow`, `lucide-react`, `useCopyFlash`, `IconButton`, `Skeleton`, `uiClasses.spin`, `Eyebrow`, `--color-*-strong`; graph `<svg>` and 10px edge labels allowlisted | Worktree ownership and graph semantics |

## Native Console sweep

| Surface | Result |
|---|---|
| Workspace tabs and mobile menu | Neutral selected fill/ink; shared control motion; roving tab focus, overflow fades + shared `DropdownMenu`, shared `ConfirmDialog` for unsaved work |
| Trigger/function catalogs | Public list/chip recipes, shared icon tabs, and neutral selected row/title/edge |
| Schema and chat tables | Shared responsive table parts; natural-case sans headers, horizontal row dividers, selective technical mono |
| Traces, waterfall, and group-by | Neutral selected trace treatment; group-by uses shared `Selector` |
| Chat system-prompt and sheet navigation | Neutral selection and shared overlay behavior |
| Working-directory picker | Neutral current-directory row; hierarchical navigation retained locally |
| Empty-pane page launcher | Public list/panel recipes; persistent always-open command palette retained locally |
| Dialogs, menus, selects, selectors, tooltips, sheets | Shared portal scope and motion vocabulary |
| Streaming context usage | High-frequency width updates are immediate |
| Pane layout | `PageShell` is a container; panel layouts use `@container`, and the remaining viewport utilities are the 640 px phone chrome, allowlisted per file in `ade/web/src/lib/viewport-breakpoint-conformance.test.ts` |
| Case transforms | `Eyebrow` is the only uppercase surface; no `lowercase`/`uppercase` elsewhere (`typography-conformance.test.ts`) |

## Commands

A page's primary verbs are palette rows (`PageRenderProps.commands`, or
`host.commands` for a page not yet open), each with a key where one is
natural, scoped to the page's pane. No page listens for a key the console
owns; the registry refuses those at registration. See
`ade/skills/injectable-ui.md` › Slots (`host.commands`, `host.palette`,
`PageRenderProps.commands`).

| Page | Commands (render time unless noted) | Keys | Palette source |
|---|---|---|---|
| chat (first-party) | focus composer, next / previous message, approve / deny the pending call, expand, copy, latest, switch model, stop, new chat, search conversations | `I`, `J` / `K`, `A` / `D`, `O`, `Y`, `End`, `M`, `Escape`, `N`, `/` | chats (built in) |
| workers (first-party) | refresh, search, open Compose (when the compose-ui page is registered); start / stop / restart on compose-supervised rows | `R`, `/`, `C` | workers, functions (built in) |
| traces (first-party) | search, follow turns, clear filters, close detail | `/`, `F`, `Escape` | |
| ide (page `ide`) | go to file…, search in files, show the explorer / source control / timeline, toggle the sidebar, toggle the terminal, next / previous tab, close the tab, reveal the active file, go to line…, go back / forward, next / previous change, compare the active file with…, new file…, toggle hidden files, toggle word wrap, revert the last turn; setup: open file…, open | `Ctrl+P` (Mac) / `Alt+P` (elsewhere, where `Ctrl+P` is the browser's Print), `` Ctrl+` ``, `Alt+←` / `Alt+→`, `Shift+Alt+←` / `Shift+Alt+→`, `Alt+Z` | files (`coder::search`, `#`) |
| database | focus SQL, refresh, focus tables; setup: open | `S`, `R`, `/` | tables (`database::listTables`) |
| cron | new schedule, search, refresh, focus composer; setup: open, new schedule… | `N`, `/`, `R` | schedules (`listAllSchedules`) |
| state | save; setup: open | `Mod+S` | keys (`state::list_groups` + `state::list_keys`) |
| storage | upload, refresh; setup: open | `U`, `R` | objects (`storage::listObjects`) |
| memory | reload, new bank; setup: open | `R`, `N` | banks (`memory::bank::list`), memories (`memory::list`) |
| iii-directory | new entry, filter, save; setup: open | `N`, `/`, `Mod+S` | entries (`directory::skills::list`, `directory::agents::list`) |
| canvas | new, save, delete; setup: open | `N`, `Mod+S`, `X` | canvases (`canvas::list`) |
| a2ui | new from template, undo, pin, export React; setup: open | `N`, `Z`, `P`, `E` | surfaces (`a2ui::surface::list`, per conversation) |
| browser | new session, stop, address bar, ⋮ menu (find in page, zoom, screenshot, print to PDF, device toolbar, import / copy cookies, clear browsing data, diagnostics), device toolbar (viewport presets), responsive live view (tracks the pane), read-only badge, handoff banner (confirm a paused session), Console / Network / Downloads / History panes, annotate (pin / box / arrow tools, colours, undo; pins carry the element under them), send / save / download / clear annotations, saved-sets dialog; setup: open | `N`, `X`, `L`, `Mod+F`, `Mod+=`/`Mod+-`/`Mod+0`, `C`, `Mod+Enter` | sessions, downloads (`browser::downloads::list`), history (`browser::history::list`), cookies (`browser::cookies::list`), diagnostics (`browser::doctor`), saved annotation sets (state scope `annotations`) |
| sandbox-code-runner | new, run code, refresh; setup: open | `N`, `C`, `R` | sandboxes (`sandbox::list`) |
| computer | start, stop; setup: open | `N`, `X` | sessions (`computer::sessions::list`) |
| worktree | refresh, close detail; setup: open | `R`, `Escape` | worktrees (`worktree::list`) |
| tailscale | refresh, create link, copy link, open link, stop route, sections 1-7; setup: open | `R`, `N`, `C`, `O`, `X`, `1`-`7` | |
| compose-ui (page `compose`) | refresh, filter containers, add worker…, validate compose file, sections 1-5 (Topology first); setup: open | `R`, `/`, `N`, `V`, `1`-`5` | containers (`compose::status`) |
| ade catalog (functions, triggers) | search, toggle internal, refresh, run function; setup: open | `/`, `I`, `R`, `Mod+Enter` | functions (built in) |
| eval | new evaluation, refresh history; setup: open, new evaluation… | `N`, `R` | evaluations (`api.list`) |
| github | toggle live, refresh, close detail; setup: open | `L`, `R`, `Escape` | (PR / issue data not wired in the UI yet) |
| pdf | choose file; setup: open | `O` | |
| onboarding, vscode | setup: open | | |

Every page names its `data-autofocus` target. Shared `TableRow interactive` and `List` carry the arrows, Enter and Space for the rows and lists built from them.

## Deliberate local-control exceptions

- `DirectoryPicker` and model/provider navigation use hierarchical drill-in
  and mobile sheet flows rather than a flat searchable selector.
- `SessionAddonsPicker` and evaluation comparison own multi-selection.
- `ReviewScopePicker` owns hierarchical submenu selection.
- `EmptyPane` is a persistent, always-open command palette; it uses shared
  list/panel recipes without adopting popover lifecycle.
- The ide's go-to-file overlay (`Ctrl+P`) is a palette over the shared
  `Dialog`: one field drives a listbox of the worker's fuzzy path matches
  through `aria-activedescendant`, so its rows are local (the shared `List`
  moves the focus onto its rows). Selection is the neutral wash plus edge;
  the glyph is the ide's own file-type icon set. Inside the Monaco body
  the pane reclaims the chord in the capture phase, because Monaco binds
  `Ctrl+P` to cursor-up on a Mac and cancels it before the dispatcher.
- The ide's vertical activity rail and the three local split-pane drag
  implementations (`ide`, `iii-directory`, `canvas`) stay local until
  `Toolbar` grows an orientation and the package gains `SplitPane`
  (`docs/plans/2026-09-16-worker-ui-migration.md`, package gaps).
- Graphs, charts, editors, terminals, file trees, ERDs, and canvas surfaces
  may use domain colors and direct-manipulation behavior. Their surrounding
  navigation and selection still follow the shared contract.

## Manual QA matrix

- Light and dark themes.
- 320–430 px phone, narrow split pane, wide pane, and 200% zoom.
- Touch, pointer, keyboard-only navigation, visible focus, names/roles,
  selected state, and live-region output.
- Loading, empty, unavailable, validation error, runtime error, success,
  reconnect, long names/payloads, and offline states.
- Reduced motion plus streaming, log, trace, drag, resize, and pointer-driven
  updates.
- No blue/orange selected names, tabs, chips, card fills, borders, outlines,
  or rails in either theme.
- Content tabs use the shared line treatment with 16 px icons and natural-case
  sans labels; global workspace tabs use weight 500.
- No application icons below 16 px, panel-wide mono typography, or CSS case
  transforms on human-facing controls.
- Shared overlays stay styled inside injected workers; custom portals carry
  the worker's `data-iii-ui` scope.

Automated type, token/class, selection, icon-size, typography, viewport
breakpoint, unit, worker-build (scope, tokens, lint), production-build, and
Storybook checks complement this matrix. They do not replace a manual pass in
the running Console.
