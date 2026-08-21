# @iii-dev/console-ui

The compile-time surface of the console's injectable-UI runtime module —
types for `setup(host)`, the slot contracts, the extension engine client,
the shared component library, stable CSS recipes, and the canonical token
inventory.

**There is no bundleable runtime here, by design.** At runtime the console's
import map resolves `@iii-dev/console-ui` to `/vendor/console-ui.js`, which
re-exports the running SPA's own React tree, engine client, and components
from `window.__III_CONSOLE__`. Every worker shares the console's single copy
— nothing from this package (or React) ships inside a worker's asset, which
is what keeps injected bundles tens of KiB. The `index.js` entry throws with
instructions if a build bundles it anyway.

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
- Shared `Tooltip`, `Dialog`, `DropdownMenu`, `Select`, and `Selector` portals
  preserve an injected worker's `data-iii-ui` scope. `IconButton` combines an
  accessible label with the shared tooltip contract.
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
  synchronized.
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
  context: { type: 'file', path: '/repo/src/app.ts' },
})
```

The host reuses an existing page or places it beside chat, and delivers a
`panelContext` event to the page's `PageRenderProps`. Use the event `id` to
react to repeated clicks. Context is ephemeral; fetch large bodies from the
worker by opaque id.

and keep it external in the build (alongside the react specifiers):

```js
external: ['react', 'react-dom', 'react-dom/client',
           'react/jsx-runtime', '@iii-dev/console-ui']
```

Full authoring guide: `workers/docs/sops/injectable-console-ui.md`.

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
guards in `console/web` fail the build/tests when they drift:

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
