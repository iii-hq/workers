# @iii-dev/console-ui

The compile-time surface of the console's injectable-UI runtime module —
types for `setup(host)`, the slot contracts, the extension engine client,
the shared component library, stable CSS recipes, and the canonical token
inventory.

**The root entry has no bundleable runtime, by design.** At runtime the
console's import map resolves `@iii-dev/console-ui` to
`/vendor/console-ui.js`, which re-exports the running SPA's own React tree,
engine client, and components from `window.__III_CONSOLE__`. Every worker
shares the console's single copy — no component (or React) ships inside a
worker's asset, which is what keeps injected bundles tens of KiB. The
`index.js` entry throws with instructions if a build bundles it anyway. The
`/format` and `/hooks` subpaths are the deliberate exception: React-free
helpers that bundle into the worker asset (see below).

## Using it in a worker UI

The package is linked through the repo's pnpm workspace — no publishing, no
copying types around:

```jsonc
// <worker>/ui/package.json
{ "dependencies": { "@iii-dev/console-ui": "workspace:*" } }
```

```tsx
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  List,
  ListItem,
  Selector,
  SegmentedControl,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  type Host,
  uiClasses,
} from '@iii-dev/console-ui'
```

Use the shared contracts for repeated Console interactions:

- `List`/`ListItem`, `Card`, `CollapsibleCard`, `CardHighlight`, `Panel`, `Chip`, `Badge`, `IconButton`, and the `Table`
  family provide the common structural language. `uiClasses` exposes
  equivalent stable recipes for semantic markup that does not need another
  React wrapper. `CardHighlight` is the borderless neutral inset for related
  content that needs emphasis inside a card; it is not a hover, selection, or
  status treatment. `CollapsibleCard` composes with `CollapsibleCardTrigger`
  and `CollapsibleCardContent` for an accessible auto-height transition that
  honors reduced motion and keeps worker-local content mounted. `Badge` is the
  shared rounded status label; use its
  `default`, `ok`, `accent`, `warn`, or `alert` variant instead of recreating
  status-pill colors in worker CSS.
- Compose simple tables as `TableViewport` → `TableFrame` → `Table`, then use
  the semantic header/body/row/head/cell parts. Tables use natural-case sans
  headers, horizontal row dividers, responsive overflow, and no outer card or
  border. Use `density="compact"` in chat; reserve mono for technical cell
  values such as identifiers, paths, types, and code.
- `TabsList variant="line"`/`TabsTrigger` and `SegmentedControl
  variant="tabs"` switch peer content views with a bottom rule, neutral active
  underline, 600-weight natural-case labels, and semantic 16 px icons by
  default. Use `SegmentedControl variant="radio"` for persisted exclusive
  choices. Selected rows, cards, tabs, chips, and segments remain neutral in
  both themes; accent is not a selection token.
- `Selector` is the searchable single-choice control, with grouped/disabled
  options, caller-owned async filtering, loading/empty/error/validation
  states, and optional free-form creation. `Select` is for small finite lists.
  `ModelPicker` is the Console-owned responsive model catalog picker for chat
  and injected profile editors; worker UIs provide the catalog and selection.
- Shared `Tooltip`, `Dialog`, `ConfirmDialog`, `DropdownMenu`, `Select`,
  `Selector`, and `BottomSheet` portals preserve an injected worker's
  `data-iii-ui` scope. `IconButton` combines an accessible label with the
  shared tooltip contract. `useConfirm()` (render its `dialog`, then `await
  confirm({ … })`) or `ConfirmDialog` replaces `window.confirm` — a lint
  error in worker UI: cancel owns initial focus, Escape cancels, and unsaved
  items can be listed under the description.
- `AnnotationLayer` is the one way to put numbered pins with notes on a
  picture: the note is written in a callout beside the pin; the page owns
  the list and sends it on through `host.chat.compose`. `AnnotationList`
  is the same notes as rows.
- `ImageViewer` is the one full-screen image surface: wheel and pinch zoom
  about the pointer, drag pans, double-click toggles fit and actual size,
  `+`/`-`/`0`/`1` and arrows on the keyboard, Escape closes and focus returns
  to the opener. Wrap the thumbnail in `ImageThumbnailButton` and pass a
  caption that is an attachment name or a relative path.
- `PageSidebar` owns sidebar collapse motion, stable focus/ARIA, pointer and
  keyboard resize, best-effort persistence, and container breakpoints in the
  Console host. Workers provide navigation content and declarative limits;
  they do not ship gesture, storage, or transition implementations. Children
  remain mounted while collapsed, and instances sharing a `storageKey` stay
  synchronized. Narrow navigation is full-width by default for list/detail
  and hierarchy drill-in; `narrowMode="drawer"` is an explicit opt-in for
  secondary navigation that overlays an unchanged main workspace.
- Human-facing chrome uses sans and authored sentence/title case. Mono is only
  for machine-readable identifiers, paths, values, payloads, code, and tabular
  data. Application icons use a 16 px baseline; do not author icons below
  16 px.
- `tokens` names the CSS variables workers may use, including the
  `--motion-duration-*` and `--motion-ease-*` vocabulary. Shared motion
  recipes honor reduced motion; high-frequency streaming updates should be
  immediate.

Function-trigger renderers receive the harness's optional user-facing
`message.description`. A renderer can declare `metadata: { display: true }`
to keep a successful rich artifact (for example a screenshot or file-change
summary) visible in the chat flow; return `null` for unsupported/error shapes
to fall through to the next renderer.

Renderers can also open their worker's registered page with contextual JSON:

```tsx
host.panels?.open({
  pageId: 'shell',
  context: { type: 'file', path: '/repo/src/app.ts', line: 12, endLine: 40 },
})
```

(The shell's `file` context takes an optional `line`/`endLine` window; the
page opens the file and selects those lines.) The reverse direction goes
through `host.chat?.compose({ text, inline: true })`: an inline compose
appends to the end of the draft's last line instead of starting a paragraph,
which is how the shell's "Reference in chat" selection action hands a
`#file(path:from-to)` token to the composer. `CodeEditor` offers that bar
through its `selectionActions` prop and lands on a referenced window through
`revealLines(from, to)` on its handle.

The host reuses an existing page or places it beside chat, and delivers a
`panelContext` event to the page's `PageRenderProps`. Use the event `id` to
react to repeated clicks. Context is ephemeral; fetch large bodies from the
worker by opaque id.

Everything above is imported from the package root, which stays external in
the build — `buildWorkerUi` (see *Building a worker UI* below) does that for
all six import-map specifiers.

Two subpaths DO bundle (React-free helpers and hooks the Console itself
uses): `@iii-dev/console-ui/format` — `formatRelative`, `formatDuration`,
`formatBytes`, `errorMessage`/`errorCode`, `copyText` — and
`@iii-dev/console-ui/hooks` — `useContainerNarrow`, `usePaneState`,
`useCopyFlash`, `useWorkerLive` (fetch + trigger bindings + visible-tab poll).
Newer shared components: `Eyebrow` (the mono caps label; `uiClasses.eyebrow`
is the class form), `SearchField`, `Toolbar`/`StatusBar` (36 px raised and
28 px quiet strips with an `end` slot), `MetaRow`/`ActionLine` (a card's
metadata strip and its `icon`-led action lines — pass a Lucide element, not a
glyph string), `BottomSheet`, `Kbd`/`KeyCombo`, `LiveRegion`, and
`<Tooltip label="…">` as the one-line form of the trigger/content
composition. `setup(host)` may return a disposer; the loader runs it first on
hot reload and disconnect. Icons come from `lucide-react` (external, shared
with the console) — never inline SVG copies.

Full authoring guide: `workers/docs/sops/injectable-console-ui.md`.

## Building a worker UI

Every worker's `ui/build.mjs` is the shared esbuild driver:

```js
import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'

await buildWorkerUi({ scope: 'state' })
```

`scope` is the `data-iii-ui` value the console wraps the render in — the first
segment of the asset path, normally the worker name. The driver bundles
`page.tsx` + `styles.css` into `dist/` (`--watch` rebuilds on change,
unminified), keeps the six import-map specifiers external (only the exact
`@iii-dev/console-ui` root — `/format` and `/hooks` bundle), and after each
build fails on an unscoped selector, an unprefixed `@keyframes`, `@font-face`,
an asset over 8 MiB or a `var(--color-…)` token the console does not define,
then lints the source against the design rules below.

Options: `entryPoints` (default `['page.tsx', 'styles.css']`), `outdir`
(`'dist'`), `root` (`process.cwd()`; pass `import.meta.dirname` when invoked
from elsewhere), `keyframePrefixes` (`[scope, `${scope}-ui`]`),
`allowUnscopedSelectors` (selector prefixes that are global on purpose, e.g. a
portal root), `strictTokens` (default `true`; `false` warns instead), `lint`
(`false` skips the lint; `{ strict, disable, allow }` tunes it — see below),
`minify` (`!watch`), `watch`, `plugins`, `extraExternal`, `define`. Custom
builds import `workerUiExternals`, `workerUiExternalsPlugin`, `assertScoped`
and `checkTokens` from the same module. `tsconfig.json` extends
`@iii-dev/console-ui/tsconfig.worker-ui.json`.

### Lint rules

`@iii-dev/console-ui/lint-worker-ui` — `lintWorkerUi({ root, scope, strict,
disable, allow })` — scans `styles.css`, `page.tsx` and `src/**` (never
`dist/` or tests) and returns `{ errors, warnings }` of `{ rule, file, line,
excerpt, hint }`. The driver prints the findings grouped by rule after every
non-watch build and fails on errors; `strict: true` promotes every warning,
`disable: ['rule']` drops a rule, `allow: { rule: ['substring', /re/] }`
ignores matching excerpts, and a `lint-allow <rule>` comment on the line
above a finding does the same in place. CLI: `node
packages/console-ui/lint-worker-ui.mjs <worker>/ui [--strict] [--json]`, or
`--all` from the repo root for one row per worker.

| Rule | Level | Checks |
| --- | --- | --- |
| `no-window-dialogs` | error | `window.confirm/alert/prompt(` — use `useConfirm()`/`ConfirmDialog` |
| `icon-size` | error | Lucide `size`, `<svg width/height>` or `size-3`/`w-3 h-3` classes below 16 px |
| `accent-selection` | error | `var(--color-accent…)` in a rule whose selector is a selected/active/current state (focus rules excepted) |
| `no-inline-svg` | warning | `<svg` in a `.tsx` outside `icons.tsx`/`icons/` — import from `lucide-react` |
| `radius` | warning | `border-radius` other than `0`, `6px`, `9999px`, `50%`, `var(--radius-*)`, `inherit` |
| `font-family` | warning | anything but `var(--font-…)`/`inherit` |
| `font-size` | warning | below `11px`/`0.6875rem` |
| `case-transform` | warning | `text-transform` (or `textTransform:`) uppercase/lowercase/capitalize — use the Eyebrow recipe |
| `focus-stroke` | warning | `:focus`/`:focus-visible` outline, box-shadow or border in accent — use `--color-rule-focus` |
| `shadow` | warning | `box-shadow` that is not `var(--shadow-*)`, `none` or a token inset/1–2 px stroke |
| `hex-color` | warning | `#hex`/`rgb()`/`hsl()` literals in CSS (custom properties on the scope root are fine) |
| `motion-literal` | warning | `transition`/`animation` with a literal `ms`/`s` duration — use `--motion-duration-*` |
| `keyframes-shared` | warning | `@keyframes …spin/pulse/shimmer/fade` — use `uiClasses.spin`/`uiClasses.pulse` |
| `viewport-media` | warning | `@media (max-width|min-width …)` — use `@container` |
| `tailwind-in-worker` | warning | `className` strings with 3+ Tailwind utilities |

## Transcript annotation renderers

Workers can feature-detect
`host.chat?.registerTranscriptRenderer?.({ id, render })` to render a detail
for an assistant-origin envelope:

```ts
origin.<id> = {
  type: 'console.transcript',
  version: 1,
  summary: 'safe plain-text summary',
  data: { /* worker-owned JSON */ },
}
```

The renderer receives only `{ version, summary, data }`, never the surrounding
transcript or complete origin. Lookup is exact by `id`, with last registration
winning; asset replacement, worker disconnect, and per-worker disable dispose
the renderer automatically. The host scopes and error-boundary-wraps it. A
missing, disabled, disconnected, or incompatible renderer leaves the safe
summary visible. Ordinary origin fields do not create rows.

## Keeping it honest

The declarations are hand-modeled on the console's real components; two
guards in `ade/web` fail the build/tests when they drift:

- `src/lib/console-ui-conformance.test.ts` — type-level check that every
  declared component export is satisfied by the real component, plus runtime
  checks that the curated `components` record matches `component-names.mjs`
  and the public token/class manifests match the Console stylesheet.
- `src/lib/selection-conformance.test.ts` — protects neutral selection from
  accidental accent text, border, outline, or ring regressions.
- `src/lib/icon-size-conformance.test.ts` — prevents application icon usages,
  component defaults, and root SVGs below 16 px from re-entering Console or
  checked-in worker UI.
- `src/lib/typography-conformance.test.ts` — keeps human-facing shared chrome
  sans and prevents CSS case transforms from returning to common recipes.
- `scripts/generate-vendor-shims.mjs` — evaluates the generated shim, so a
  bad export name fails the console build, never a browser tab.

Declared props are the *supported authoring surface*: the real components
may accept more (Radix pass-through), and those extras carry no
compatibility promise.
