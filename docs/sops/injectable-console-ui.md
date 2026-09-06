# Injectable console UI — shipping worker UI into the running console

How a worker ships React pages, function-trigger renderers, trigger-activity
renderers, configuration forms, and stylesheets into every open console tab
**at runtime** — no console rebuild, no iframe, hot-reloaded on
re-registration. The full design rationale
lives in the tech spec (`iii/tech-specs/2026-07-17-injectable-ui/`); this SOP
is the operational guide for worker authors, grounded in the shipped
implementation. The `state` worker is the broad delivery reference; the
`cron` worker is the focused trigger-activity renderer reference.

| Piece | Reference |
|---|---|
| Console-side registry + trigger types | `workers/console/src/ui_assets.rs` |
| HTTP serving (`/ui`, `/ui/*`, `/vendor/*`) | `workers/console/src/server.rs` |
| Browser loader (import / link-swap / dispose) | `workers/console/web/src/lib/ui-loader.tsx` |
| Slot registries (pages, renderers, forms) | `workers/console/web/src/lib/ui-slots.ts` |
| The `@iii-dev/console-ui` runtime surface | `workers/console/web/src/lib/console-api.ts` |
| The `@iii-dev/console-ui` package (types + component manifest) | `workers/packages/console-ui/` |
| The `iii-console-ui` crate (Rust worker-side registration) | `workers/crates/console-ui/` |
| Worker reference implementation | `workers/state/src/ui.rs` + `workers/state/ui/` |
| Trigger-activity renderer reference | `workers/cron/src/ui.rs` + `workers/cron/ui/` |

> **Companion skill — keep it in sync.** `workers/console/SKILL.md` is a
> standalone skill teaching this same workflow to authors *outside* this
> repo: it consumes `@iii-dev/console-ui` via `npm install` and the
> `iii-console-ui` crate via `cargo add`, instead of the workspace/path
> links this SOP uses. By design it references no repo files, so nothing
> keeps it honest automatically — any change to this SOP, the wire
> contract, the shared component surface, or either package MUST update
> the skill in the same change.

## How it works (one paragraph)

The console worker owns three trigger types. A worker registers a
`console:script` or `console:style` trigger whose `config.path` (e.g.
`state/page.js`) is the asset's identity; the trigger's `function_id` names a
*content function* on the worker that the console invokes to fetch the source
(`{path}` in, `{content, content_type?}` out). The console hashes and caches
the bytes, serves them from its HTTP port (`GET /ui/<path>?v=<hash>`), and
pushes an update to every open tab over the third type, `console:assets`
(tabs subscribe; the console invokes each tab's handler with
`sync`/`set`/`delete` events). Scripts are ES modules the tab `import()`s and
calls `setup(host)` on; styles are `<link>` elements the tab swaps in place.
Re-registering the same path overrides it — that **is** the hot-reload signal.
Registration is deployment; disconnect is teardown.

## Quick start: add UI to your worker

Four pieces, all visible in the `state` worker.

### 0. Join the UI workspace

The workers repo root is a pnpm workspace (`workers/pnpm-workspace.yaml`)
that links the console SPA, the `@iii-dev/console-ui` package
(`packages/console-ui`), and every worker's UI project. Two steps:

1. Add your UI dir to the root `pnpm-workspace.yaml` `packages` list
   (e.g. `- mywork/ui`).
2. Depend on the shared package in your UI project's `package.json`:

```jsonc
{ "dependencies": { "@iii-dev/console-ui": "workspace:*" } }
```

`pnpm install` at the repo root links it — no publishing, no copied type
files. There is one lockfile for the whole UI workspace, at the repo root.

### 1. The script asset (`ui/page.tsx`)

Ordinary React. Import from `react` and `@iii-dev/console-ui` — both resolve
at runtime through the console's import map, so they must stay **external**
in your build. Default-export a `setup(host)` function and make every
registration through `host` (the loader attributes registrations to your
script so it can dispose them on reload):

```tsx
import { Button, EmptyState, type Host } from '@iii-dev/console-ui'

export default function setup(host: Host) {
  host.pages.register({
    id: 'state-manager',            // page URL: #/ext/state-manager
    title: 'state',                 // nav label
    configurationId: 'state',       // host adds the standard settings action
    render: () => <StateManagerPage host={host} />,
  })
  host.functionTriggers.register(createStateTriggerRenderer(host))
  // When the worker owns a trigger type:
  host.triggerRenderers?.register(createTriggerActivityRenderer())
  host.configForms.register('state', StateConfigForm)
  host.providerConfigForms?.register('my-provider', MyProviderConfigForm)
  // optional: return a teardown fn; the loader runs it on dispose
}
```

`Button`, `List`, `Card`, `Panel`, `Chip`, `Table`, `Tabs`,
`SegmentedControl`, `Selector`, `Tooltip`, `EmptyState`, `Dialog`, `Markdown`,
… are the console's own components, re-exported by name with typed props — at
runtime they come from the running console's single React tree (via
`/vendor/console-ui.js`), so importing them adds **zero bytes** to your
bundle. `uiClasses` exposes the same stable
list/card/panel/chip/table/field/motion recipes to injected markup. Use these
contracts instead of copying base components into a worker.

Use `TabsList variant="line"`/`TabsTrigger` or `SegmentedControl
variant="tabs"` for peer content views. Shared line tabs use a bottom rule,
neutral active underline, 600-weight natural-case sans labels, and semantic
16 px icons by default. Reserve `SegmentedControl variant="radio"` for a
persisted exclusive choice. Do not carry private boxed-tab CSS.

Compose a simple responsive table as `TableViewport` → `TableFrame` →
`Table`, with `TableHeader`, `TableBody`, `TableRow`, `TableHead`, and
`TableCell`. The shared visual uses natural-case sans headers and horizontal
row dividers without an outer card or border. Use comfortable density on
pages and `density="compact"` in chat. Apply mono only to technical values
inside cells; ordinary labels and explanatory copy remain sans. Hover belongs
only on `TableRow interactive`, and selected rows use the neutral selection
ramp.

### 2. The style asset (`ui/styles.css`)

Plain CSS, **every rule scoped under your worker's wrapper attribute**:

```css
[data-iii-ui="state"] .state-ui { color: var(--color-ink); }
@keyframes state-ui-flash { /* prefix keyframes names — they are global */ }
```

The console mounts every injected render inside
`<div data-iii-ui="<first path segment>" style="display:contents">`, so
scoped rules apply to your UI and nothing else. Use the canonical `tokens`
inventory exported by `@iii-dev/console-ui`: surface and ink tokens for
hierarchy, `--color-edge` for structure, semantic status tokens, font tokens,
and `--motion-duration-*`/`--motion-ease-*` for transitions. Dark mode is a
variable flip on `html[data-theme]`, so token-based styles theme for free.
Never hardcode theme colors.

Elevation is three shared tokens, never a hand-rolled stack:
`--shadow-raised` for a card sitting on a panel, `--shadow-floating` for
menus, popovers, and sheets, and `--shadow-lift` for an instrument surface
that must read as lifted off the canvas with a crisp edge — the chat composer
is the reference. Each token is a complete `box-shadow` value (inset top
highlight, inset ring, 1 px edge, and drops for `lift`) whose ingredients
(`--iii-ui-lift-*`) are re-tinted by the dark theme, so worker CSS writes
`box-shadow: var(--shadow-lift);` and nothing else: no border or extra drop
beside it, and never a literal shadow color. Promote a new elevation by adding
a token to `console/web/src/index.css` and `packages/console-ui/token-names.mjs`,
not by copying a stack into a worker sheet.

For related content that needs emphasis inside an existing card, use
`CardHighlight` or `uiClasses.cardHighlight`. It uses
`--color-card-highlight` and is always borderless and shadowless. It is not a
hover, selection, focus, status, or standalone-card treatment.

Expandable worker cards use the public `CollapsibleCard` composition:

```tsx
import {
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
} from '@iii-dev/console-ui'

<CollapsibleCard>
  <CollapsibleCardTrigger className="p-3">
    Activity summary
  </CollapsibleCardTrigger>
  <CollapsibleCardContent>
    <div className="border-t border-edge p-3">Activity details</div>
  </CollapsibleCardContent>
</CollapsibleCard>
```

The trigger owns keyboard and ARIA behavior. Content stays mounted while
collapsed so local state survives, and the auto-height transition uses the
Console motion tokens, including automatic reduced-motion handling. Keep
padding inside the content child so the animated grid can fully collapse.

Selection is neutral in both themes: `--color-surface-selected`, stronger
`--color-ink`, and an optional `--color-edge`. Do not change selected names,
tabs, chips, cards, or rows to `--color-accent`; reserve accent for a primary
action, form focus, live activity, or semantic domain data.

What must NOT be in the sheet: unscoped selectors (`:root`, `html`, `body`,
`*`, bare element names) and `@font-face` — injected CSS is unlayered, so an
unscoped rule silently beats the console's fully-layered CSS document-wide.
The console runs a warn-only lint on every `console:style` fetch
(`lint_style`, `workers/console/src/ui_assets.rs`) and puts findings in the
manifest's `warnings` array; keep it empty.

Injected Tailwind utility names are not part of the Console build. Compose
the named components and public `uiClasses` recipes, then add scoped CSS only
for worker-specific layout and data presentation. Shared motion recipes
already honor reduced motion. Custom transitions use the public motion tokens;
streaming text, rapidly updating meters, and cursor-following geometry update
without transitions. Scope custom reduced-motion overrides under the worker
attribute and prefix keyframe names because keyframes remain global.

Use `Selector` for searchable single-choice input, including grouped,
disabled, async, loading, empty, error, validation, and declared free-form
states. Keep `Select` for a small finite list. Use shared `Tooltip` parts or
`IconButton` instead of local hover timers and portal geometry. A local
selector is justified only by a materially different interaction such as
hierarchical drill-in, multi-select, or a persistent command palette; record
the exception in the Console UI conformance inventory.

### 3. The build (`ui/build.mjs`)

esbuild with the five shared specifiers external
(`workers/state/ui/build.mjs`):

```js
const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: ['react', 'react-dom', 'react-dom/client',
             'react/jsx-runtime', '@iii-dev/console-ui'],
}
```

Everything else gets bundled in; keep output well under the console's 8 MiB
per-asset cap (a slot component should be tens of KiB). Two footguns:

- **A forgotten `react` external bundles a second React** — hooks resolve
  against the bundled copy's never-installed dispatcher and fail at runtime
  as a cryptic "Invalid hook call" with nothing pointing at the cause. (A
  forgotten `@iii-dev/console-ui` external fails loudly instead: the
  package's bundleable entry throws with the fix in the message.)
- **Only those five specifiers exist in the import map.** A transitive
  dependency importing any other bare react-family specifier
  (`react-dom/server`, …) fails at `import()` time, not build time.
- **Never bundle an editor.** Every code/text editing surface — in the
  console and in injected worker UI alike — is the shared Monaco-backed
  `CodeEditor` from `@iii-dev/console-ui` (Monaco runs once, inside the
  console, themed by the design tokens in both themes). Bundling
  `monaco-editor`, CodeMirror, or any other editor into a worker asset
  ships megabytes toward the 8 MiB cap to duplicate what the console
  already provides.

Rust workers embed `dist/` with `include_str!` and rebuild it from
`build.rs` (see `workers/state/build.rs`) so the worker stays one
self-contained binary.

### 4. Registration (the worker side)

The wire contract is: one content function serving all of the worker's
assets (dispatch on `path`), one trigger per asset. Rust workers don't
hand-roll it — the shared **`iii-console-ui`** crate
(`workers/crates/console-ui`) is the whole worker side. Workers in this
repo link it directly by path so it versions with the console worker here
(out-of-repo workers install it instead — see `workers/console/SKILL.md`):

```toml
# <worker>/Cargo.toml
[dependencies]
iii-console-ui = { path = "../crates/console-ui" }
```

```rust
use iii_console_ui::ConsoleUi;

ConsoleUi::new("state")
    .script(
        "state/page.js",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js")),
    )
    .style(
        "state/styles.css",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css")),
    )
    .register(&iii);
```

One call registers everything: the content function (`<worker>::ui-content`,
flagged `internal: true` so it stays console plumbing rather than
discoverable API), one Message-path trigger per asset (MIME type derived
from the asset kind), and the `III_<WORKER>_UI_WATCH` dev watcher. Each
default derives from the worker name and has a builder override
(`.content_function_id(…)`, `.watch_env(…)`, `.watch_default_dir(…)`). The
builder panics on paths the console would reject (wrong extension,
uppercase, `..` segments, duplicates) — an authoring mistake fails your
first unit test instead of warn-logging against a running engine; trigger
registration failures at runtime stay warn-logged, not fatal.

Node workers write the two pieces directly (there is no Node helper yet):

```ts
iii.registerFunction('mywork::ui-content', async ({ path }) => {
  const file = ASSETS[path]     // { 'mywork/page.js': 'dist/page.js', … }
  if (!file) throw new Error(`unknown ui asset: ${path}`)
  return { content: await readFile(file, 'utf8') }
})
const trigger = iii.registerTrigger({
  type: 'console:script',
  function_id: 'mywork::ui-content',
  config: { path: 'mywork/page.js' },
})
```

**Always use the SDK Message path, never `engine::register_trigger`** (the
crate does). Function-path triggers are durable: they outlive your worker (a page pointing
at a dead content function) and silently vanish on engine restart with no
replayer. Message-path triggers are GC'd on disconnect and replayed by the
SDK on reconnect — injected UI dies and revives with its worker, which is the
design.

Ordering never matters: register before the console is up and the engine
parks the intent, delivering it when the console arrives; console restarts
replay every live binding; engine restarts are absorbed by SDK re-registration.

## The wire contract (what the console enforces)

| | |
|---|---|
| Trigger types | `console:script` (ESM JS), `console:style` (CSS), `console:assets` (tab subscriptions — you don't register these) |
| Trigger config | `{ "path": string }`, nothing else |
| Path rules | lowercase `[a-z0-9._-]` segments, no leading slash, no `.`/`..` segments, ≤ 512 chars; extension must match the type (`.js` / `.css`); **convention: first segment = your worker name** — it becomes the `data-iii-ui` scope and the only human-readable attribution |
| Content function | input `{ "path": string }` → output `{ "content": string, "content_type"?: string }` (`content_type` defaults from the asset kind) |
| Size cap | 8 MiB per asset — registrations over it are rejected |
| Fetch budget | 2 attempts × 3 s at live registration (a failed fetch **rejects the registration**; the error reaches your SDK's registration result); 3 × 5 s on console-restart replay (failure drops the asset with a `warn!` until you re-register) |
| Override | same path re-registered ⇒ last writer wins, console-wide (even across workers — a `warn!` names both trigger ids); the superseded engine row is pruned |
| Identity/versioning | content hash (first 16 hex of sha256) — unchanged content re-registered is a no-op |
| Per-worker toggle | a worker listed in the console configuration's `injectableUi.disabledWorkers` has its registrations **accepted but held**: no serve, no manifest row, no tab pushes. Toggling back on pushes everything held to every open tab — no worker restart. The `console` worker itself cannot be disabled. |

## What `setup(host)` can register

All registration goes through the per-script `host`; every entry is disposed
automatically on hot reload and worker disconnect. Each `register` also
returns a remover for manual teardown.

### `host.pages.register({ id, title, render })`

A whole console page at `#/ext/<id>`, listed in the nav while registered.
Duplicate ids: last registration wins (a `console.warn` names both scripts).

`render` receives `PageRenderProps` — a plain `() => <Page />` render stays
valid and simply ignores them:

- `panelSide`: `'left' | 'right'` — which side of the workspace tab the
  pane hosting the page occupies (`'right'` only for the rightmost column
  of a multi-column tab; a single-column tab is `'left'`). Pages that care
  mirror their layout so a sidebar hugs the outer screen edge (the shell
  explorer does).
- `tabId`: the hosting workspace tab's stable id (tabs persist across
  reloads) — the key for per-tab UI state (the shell explorer keys its
  open files/expanded folders on it). Empty string outside a workspace
  tab.
- `onRequestClose`: close the pane hosting the page (a split drops the
  column; a single-column tab detaches back to the attach affordance).
  Wire it to `PageHeader`'s `onClose` — every page header carries the
  standard ✕. Absent when the page renders outside a closable pane.
- `panelContext`: the latest ephemeral event sent to this page through
  `host.panels.open()`. Its monotonic `id` changes for every click, including
  repeated identical payloads; `context` is worker-defined JSON.
- `setDirty`: report unsaved work. Call `setDirty('main.rs')` (or `true`)
  while the page holds something the user would lose and `setDirty(false)`
  once it is saved or discarded; closing the pane, its workspace, or the
  browser tab then asks first. Absent on older consoles, so call it as
  `setDirty?.(…)`. The host clears the entry when its pane or workspace tab is removed; unmounting a background tab does not clear it.
- `commands`: contribute commands while mounted — rows in the command
  palette (`⌘K`) under the page's name, and keys that fire only while focus
  is inside this pane. Register in an effect and return its remover:
  `useEffect(() => commands?.register([...]), [commands])`. Absent on older
  consoles; feature-detect. See [Commands](#commands-the-keyboard-reaches-every-page).

#### The page chrome — MANDATORY layout for pages

Every injected page composes the same five `@iii-dev/console-ui`
components, so panes stay visually identical across workers AND the
console's own screens (chat, traces):

```tsx
<PageShell>
  <PageHeader
    icon={<MyIcon />}                 // 16px glyph, faint ink
    title="My worker"                 // sans, human-readable title
    description="What this page is"   // natural case; truncates first
    actions={<Button …/>}             // optional right-side controls
    onClose={onRequestClose}          // the standard ✕
  />
  <PageBody side={panelSide}>         {/* mirrors for right-side panes */}
    <PageSidebar
      label="projects"
      side={panelSide}
      collapsible
      resizable
      storageKey="my-worker:projects"
      defaultWidth={260}
      minWidth={200}
      maxWidth={420}
      narrowBelow={700}
      narrowMode="drawer"            // only when main stays visible behind nav
      header={<>Projects</>}
      collapsedActions={<IconButton label="New project" … />}
    >
      …navigation…
    </PageSidebar>
    <PageMain>…workspace…</PageMain>         {/* white, flexes */}
  </PageBody>
</PageShell>
```

The pieces own the surface hierarchy (header `--color-panel-raised` +
hairline `--color-edge` border; sidebar `--color-sidebar`; main
`--color-panel`) — don't repaint those tokens per worker. A page without
a sidebar just puts its content straight into `PageBody`/`PageMain`; a
page with custom internals (the directory page's drill-in browser) may
own its body but MUST keep `PageShell` + `PageHeader`. The shell
explorer (`workers/shell/ui/src/page/index.tsx`) is the reference
composition.

When a page's worker has configuration, set `configurationId` on its
`host.pages.register` entry. The Console then adds the standard settings icon
to `PageHeader` and opens that worker inside the global Settings modal. Do not
add a second settings icon, mount a worker-local configuration dialog, or
navigate to the old Workers-screen editor. Pages without a configuration
entry omit the property. Treat `configurationId` as the stable form-family id,
not an instance name: a worker registered as `browser-team-a` still declares
`configurationId: 'browser'`.

`PageSidebar` is implemented by the Console host and resolves through the
shared `/vendor/console-ui.js` module, so importing its behavior adds no bytes
to a worker bundle. Use its declarative API instead of local collapse DOM,
pointer handlers, width clamps, `localStorage`, focus management, or motion.
It keeps one `aside` and its children mounted while collapsed; the host owns
the 220 ms width transition, content fade/offset, reduced-motion behavior,
accessible toggle, pointer/keyboard resize, and best-effort persistence.
Instances with the same `storageKey` share one preference. Pass `narrow` when
the page's drill-in state already knows the pane is narrow, or `narrowBelow`
when the shared sidebar may observe its `PageBody` parent. Narrow navigation is
`narrowMode="inline"` by default: the sidebar becomes a full-width navigation
screen, hides collapse/resize affordances, and temporarily ignores the saved
wide collapsed preference. This is the required mode for primary catalog,
list/detail, and hierarchy navigation such as functions, triggers, database,
directory, and state (`scopes → keys → value`). The page owns only the domain
route—what level is active and what Back does—not private sidebar mechanics.
Compose each navigation level with `List`, optional `ListGroup` /
`ListGroupLabel`, and `ListItem` (`selected`, `leading`, `label`,
`description`, `trailing`). Those shared rows provide the full-width card
target, neutral selection, keyboard traversal, focus treatment, and mobile
touch height; do not fork worker-specific row/button chrome.

Use `narrowMode="drawer"` only when navigation is secondary and `PageMain`
must remain mounted and unchanged behind it, such as a short section switcher.
That opt-in keeps a rail whose toggle opens an overlay with scrim and Escape
handling. Do not use the drawer to flatten a tree or replace a mobile
list/detail flow. Neither narrow mode overwrites the saved wide preference;
drag resize and wide↔narrow changes remain instant.

All human-facing chrome uses sans and authored sentence/title case; do not use
CSS case transforms on tabs, buttons, menus, fields, or labels. Reserve mono
for machine-readable ids, paths, values, payloads, source, terminal output,
and tabular data. Application icons use a 16 px baseline globally;
icon-only actions use `IconButton`, which retains an accessible label and the
shared tooltip. Do not author application icons below 16 px.

### Commands: the keyboard reaches every page

The console is keyboard-first: `⌘K` finds and takes any action, every row
shows its key, and hovering a control spells the same key. A page joins
that through commands. There are two registration points and one rule:
**a command exists only while its worker is connected.** Both paths ride
the same teardown every other registration uses, so nothing extra is needed
for that rule to hold.

```ts
interface PageCommand {
  id: string                 // unique within the page; namespaced `<pageId>.<id>`
  title: string              // the palette row reads "Shell: Open file…"
  detail?: string
  keywords?: string[]
  shortcut?: string | { mac: string[]; other: string[] }   // 'Mod+S', 'G L'
  firesWhileTyping?: boolean // default false: the caret in a field wins
  enabled?: () => boolean    // asked when the palette opens and before run
  run: () => void
}
```

**`host.commands.register(pageId, commands)`** — setup time, worker level.
Rows for a page that may not be open yet ("Shell: open file…"). Lives
exactly as long as the script: removed with the page and every other
registration when the worker's assets go. `run` usually calls
`host.panels.open({ pageId, context })` so the page opens and acts on the
context. Keys are **not** honoured here: the console's global keymap stays
the console's.

**Typing surfaces that are not form fields must stand down the bare keys.**
The dispatcher already yields to a caret in an `input`, `textarea`, `select`,
or contentEditable node. Anything else that consumes raw keystrokes — a code
editor's gutters and widgets, a read-mode diff, a drawing surface — gives the
dispatcher a plain element as the event target, and a bare page binding
(a `t`, a digit) fires mid-thought. Declare the surface:

```tsx
<div className="my-editor-body" data-keybindings-standdown="">…</div>
```

Every keystroke originating inside a `data-keybindings-standdown` element
counts as typing: bindings without `firesWhileTyping` stand down, modifier
chords still work. Put it on the smallest container that holds the whole
interactive surface (the editor body, the diff card), never on the page root —
the page chrome around it should keep answering the navigation keys.

**`PageRenderProps.commands.register(commands)`** — render time, page
level. Lives while the page is mounted in a pane. Keys are honoured and
scoped to that pane: they fire only while focus is inside it, so two panes
of the same page never both answer. Register from an effect:

```tsx
function Page({ commands }: PageRenderProps) {
  useEffect(
    () =>
      commands?.register([
        { id: 'open', title: 'Open file…', shortcut: 'P', run: openQuickOpen },
        { id: 'find', title: 'Search in files', shortcut: 'F', run: focusSearch },
        { id: 'save', title: 'Save', shortcut: 'Mod+S', firesWhileTyping: true,
          enabled: () => dirty, run: save },
      ]),
    [commands, dirty],
  )
}
```

Keys a page may take: anything the console does not already use. The
console keeps `Mod+K` and its modifier chords (Ctrl on a Mac, Alt elsewhere:
`Ctrl+T`, `Ctrl+W`, `Ctrl+[`/`]`, `Ctrl+{`/`}`, `Ctrl+1`-`9`, `Ctrl+\`,
`Ctrl+,`, and the `Ctrl+G` go-to prefix), and the browser owns its own
(`Mod+W`, `Mod+T`, `Mod+1`…); those are refused at registration with a
`console.warn` and the row stays, without a key. Bare single letters, digits
and chords (`Q L`) are the page's to take; the typing guard keeps them out
of fields unless the command says `firesWhileTyping`, a
`data-keybindings-standdown` surface swallows them entirely, and a field can
hand specific commands back with `data-keybindings-allow="<pageId>.<id>"`.

Focus follows the keyboard: opening a page from `⌘K`, a go-to chord or
`host.panels.open` moves focus into its pane — the first `[data-autofocus]`
element inside it, else the pane root — so the next keystroke is already
the page's. Mark the element a page wants focused on arrival with
`data-autofocus`. `Ctrl+{` / `Ctrl+}` (mac; `Alt+` elsewhere) step focus
across panes.

**`host.palette.registerSource(source)`** — a live source: rows computed per
query by the worker, the way an editor's quick open lists files as you type.
Registered at setup, so it exists only while the worker is connected.

```ts
host.palette?.registerSource({
  id: 'tables',
  title: 'Tables',            // the group label
  kind: 'item',               // 'file' for real files (answers `#`), 'item' otherwise
  prefix: '#',                // optional: a one-character mode that selects this source alone
  minQuery: 2,                // asked without its prefix from this many characters
  async search(query, { workingDir, conversationId, signal }) {
    const tables = await host.iii.trigger('database::listTables', { query })
    if (signal.aborted) return []
    return tables.slice(0, 30).map((table) => ({
      id: table.name,
      title: table.name,
      detail: table.database,
      run: () => host.panels.open({ pageId: 'database', context: { table: table.name } }),
    }))
  },
})
```

The bare palette is a navigator — no prefix searches only pages,
workspaces, chats and workers — so a source whose kind is `item` or `file`
is asked under its prefix or the matching filter chip, never from the bare
query. The palette asks every source that fits, debounced, and drops an
answer to a query it has moved past (`signal`). A source that throws
contributes nothing; the others still answer. Rows open the page with a
context the page already understands through `panelContext`.

**`host.palette.open({ query })`** opens the palette on a query. A prefix
lands it in that mode: `>` or `/` commands, `#` files, `@` chats. The shell's
"Open file…" row calls `host.palette.open({ query: '#' })` and hands over.

An empty query opens on the ten rows used most recently in this browser;
several words match in any order.

Definition of done for a page: its primary verbs are commands, each with a
key where one is natural; the data a user jumps to by name is a source; the
element the page wants focused on arrival carries `data-autofocus`; and
nothing in the page listens for a key the console owns.

### `host.panels.open({ pageId, context })`

Open contextual worker content beside chat without teaching the console what
that content means:

```tsx
function ScreenshotResult({ host, screenshotId }: Props) {
  return (
    <button
      onClick={() =>
        host.panels?.open({
          pageId: 'browser',
          context: { type: 'screenshot', screenshotId },
        })
      }
    >
      inspect screenshot
    </button>
  )
}

function BrowserPage({ panelContext }: PageRenderProps) {
  useEffect(() => {
    if (panelContext) showContext(panelContext.context)
  }, [panelContext?.id])
  // …
}
```

The console owns placement: it reuses and activates an already-mounted page;
otherwise it fills an empty column or inserts the page beside chat. It never
replaces an existing pane. If the active tab is full, it creates a fresh
chat + context split. The console stores context before mounting the page, so
the first render receives the event that opened it.

Context is JSON-only and ephemeral: it is not persisted with workspace tabs.
Use opaque ids for large or sensitive bodies and let the page fetch the data
from its worker on demand. Feature-detect `host.panels` when a worker bundle
must remain compatible with older consoles.

### `host.functionTriggers.register(renderer)`

Custom rendering for function-trigger messages in chat and traces:

```ts
interface FunctionTriggerRenderer {
  id: string
  isMatch(functionId: string): boolean
  tryRender(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderRunning?(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderPreview?(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderDisplay?(message: FunctionTriggerMessage): React.ReactNode | null
  FunctionIdLabel?: React.ComponentType<{ functionId: string }>
  metadata?: { display?: boolean; displayAction?: 'expand' }
  redactRaw?(value: unknown): unknown
}
```

Injected renderers dispatch **before** the first-party families
(`useFunctionTriggerRenderers`,
`workers/console/web/src/components/function-trigger/renderer-registry.tsx`),
so you can override built-in rendering for your worker's functions. Return
`null` to fall through to the next renderer. The host calls `tryRender*` only
after that renderer's `isMatch(functionId)` returns true — match narrowly (your
own function ids) and let errors and everything else keep the default cards.
Renderer callbacks are fenced: a throwing `isMatch` counts as no-match, a
throwing `tryRender` degrades to an error chip, never a broken feed.

`FunctionTriggerMessage.description` is the short user-facing action supplied
by the harness `agent_trigger` wrapper. The host shows it in the collapsed
activity row and reveals the concrete function id when the row is expanded;
historic messages without it retain the function-id fallback.

Set `metadata: { display: true }` when a successful custom result is a
first-class chat artifact (file-change summaries, screenshots, images). The
host keeps that winning renderer's non-null node visible while the raw
request/response remain collapsed. Metadata is tied to the renderer that
actually produced the node: a focused renderer can return `null` for errors
or unsupported output and safely fall through to a general renderer. Do not
mark ordinary terminal/status cards for display.

Use `tryRenderDisplay` when that inline artifact needs a compact receipt while
`tryRender` supplies its complete detail body. Add
`metadata.displayAction: 'expand'` to make a single continuous collapsible
card: the receipt remains mounted as the header and the detail body expands
underneath it with the host-owned transition, terminal tab, and raw JSON tab.
In this mode the receipt must not render its own outer card or interactive
controls because the host owns the surface, padding, and focus target. Omit
`displayAction` when the receipt owns another action such as opening a child
session.

#### `redactRaw` — your card is not the only exit

Whatever your card draws, the settled card **always** mounts a `raw json` tab
that renders `message.input` / `message.output` verbatim, with a copy button
per pane. If your rendering redacts a secret (a capability id, a token, a
path), the raw tab and its clipboard hand it over anyway — one click away.

`redactRaw` is how the WORKER declares what is secret and the CONSOLE applies
it: for a message your `isMatch` claims, the card runs the request and the
response through it **before rendering the raw panes and before building the
text the copy button copies** (the first claiming renderer that declares it
wins). The console stays ignorant of what your secrets look like — no worker
pattern belongs in shared console code.

- It gets an arbitrary JSON-ish value (object, array, string, number,
  boolean, `null`, `undefined`) and returns the redacted copy. Deep-walk it:
  ids hide in nested arrays, in log lines, in error messages, and in object
  KEYS. `sandbox-code-runner/ui/src/lib/shared.tsx` (`redactRuntimeIdsDeep`)
  is the reference implementation — a shape-preserving walk with cycle
  protection.
- Pure and total: never mutate the input, never do I/O, never throw.
- It runs during the host card's render, so it is fenced — and fails
  **closed**: a throw renders `[redaction failed — value withheld]` in place
  of the value, in the pane and on the clipboard. Degrading to the raw value
  would surrender exactly what the method exists to protect.
- It covers the card's raw panes only. A session export dumps the transcript
  verbatim by design; do not treat `redactRaw` as an access control — the
  payload still crosses the wire and lands in the trace store.

### `host.triggerRenderers?.register(renderer)`

Customize a normalized trigger activity. This is separate from
`host.functionTriggers`: match the inner trigger type (`cron`, `state`, or a
worker-defined value), never the shared function `engine::register_trigger`.

```ts
interface TriggerActivityRenderer {
  id: string
  isMatch(triggerType: string): boolean
  // Source section inside the generic detail view (compatibility/base hook).
  tryRender(activity: TriggerActivityMessage): React.ReactNode | null
  // Optional complete expanded Terminal tab.
  tryRenderDetails?(activity: TriggerActivityMessage): React.ReactNode | null
  // Optional compact clickable timeline content; no interactive children.
  tryRenderDisplay?(activity: TriggerActivityMessage): React.ReactNode | null
  // Optional raw registration/fire redaction; pure, total, cycle-safe.
  redactRaw?(value: unknown): unknown
}

interface TriggerActivityMessage {
  id: string
  kind: 'registration' | 'fired' | 'retirement'
  triggerType: string
  config?: unknown
  label?: string
  action?: string // registration metadata.action: what the event means
  delivery:
    | { kind: 'notify' }
    | { kind: 'call'; functionId: string }
  lifecycle: {
    state: 'active' | 'retired'
    once: boolean
    maxFires?: number
    expiresAt?: number
    fires: number
  }
  payload?: unknown
  // See packages/console-ui/index.d.ts for the remaining optional fields.
}
```

Register through the script host and feature-detect for older consoles:

```tsx
export default function setup(host: Host) {
  host.triggerRenderers?.register({
    id: 'cron/page.js#trigger-activity',
    isMatch: (triggerType) => triggerType === 'cron',
    tryRender: (activity) => {
      const config = parseCronConfig(activity.config)
      return config ? <CronSource config={config} /> : null
    },
  })
}
```

Use the smallest hook that fits. `tryRender` owns only source interpretation;
`tryRenderDetails` owns the complete readable detail; `tryRenderDisplay` owns
the compact row content. The host retains the disclosure interaction,
per-slot fallbacks, and Raw JSON tab after `redactRaw`. A once trigger that
fires and is automatically retired remains one activity; never duplicate it
with a worker-rendered unbind notice.

For harness bindings, `label` names the binding while `metadata.action`
describes the future event (for example, `new Explorer message received`).
The action is available in binding data before the first fire and is persisted
as `activity.action`, but registration and active-binding UI show the label.
The default UI reveals action only on a fired row: a status mark plus action,
falling back to label/source; clicking it opens the detail already expanded.
Worker displays should likewise read action only when `kind === 'fired'`.
The Raw JSON tab retains the original registration metadata for inspection.

Each renderer slot dispatches in registration order and the first non-null
node wins.
Return `null` for a different type or an unrecognized config. A throwing
matcher is treated as no match; render failures are error-bounded. The same
renderer should tolerate all three `kind` values because source identity does
not change as the binding moves through its lifecycle.

See
[`console/web/docs/custom-trigger-components.md`](../../console/web/docs/custom-trigger-components.md)
for the complete authoring guide and test matrix.

### `host.configForms.register(configurationId, component)`

Provide the settings interface for one stable configuration family in the
global Settings modal (exact id match first; last registration wins). Workers
whose runtime entry can be renamed with `III_CONFIG_NAME` register
`metadata: { ui_form: '<default-id>' }` with `configuration::register`; the
Console then reuses this family form for that runtime id. The Console does not
generate a fallback form from JSON Schema: every configurable worker must
register a deliberate interface. Your component receives:

```ts
interface ConfigFormProps {
  id: string
  schema: Record<string, unknown> | null   // null = value registered without a schema
  value: JsonValue
  onChange(next: JsonValue): void          // propose the full next value
  errors?: ReadonlyMap<string, string>     // JSON-pointer → message (client + server merged)
  focusField?: readonly string[]           // deep-link focus request — honoring it is your job
}
```

The form is render-level only: dirty tracking, schema validation, save/reset
and the SaveBar stay host-owned. You draw deliberate fields and call
`onChange`; the schema remains a validation contract, never a UI generator.
Use the shared `SettingsSection`, `SettingsList`, `SettingsRow`, and `Switch`
components for ordinary settings, with `Tabs` for useful peer sections and a
purpose-built collection editor when operators add repeated entries. Never
fall back to a raw JSON textarea. Forms render inside the canonical route
`#/configuration/workers/<configurationId>` and must remain usable at narrow
pane widths.

#### Configuration UI standard

Configuration forms use the Console's controls; a worker must not create a
second visual implementation of an ordinary input, select, switch, button,
field row, or resource drill-in. Import these pieces from
`@iii-dev/console-ui`:

- `SettingsSection` groups one subject; `SettingsList` groups related values.
- `SettingsRow` presents read-only values and compound actions.
- `SettingsField` is the default for editable values. It owns the label,
  description/error IDs, `aria-invalid`, `aria-describedby`, deep-link
  `data-field`, and the standard `fit`/`compact`/`default`/`full` control
  widths. Use `fit` with `layout="inline"` for intrinsic controls such as a
  `Switch`.
- `Input`, `Select`, `Selector`, `Switch`, `Button`, and `IconButton` own
  control appearance and interaction. `Select` and `Selector` accept `id`,
  `name`, and `data-field`; never substitute a styled native `<select>`.
- `RawValueInput` displays an environment template or future/opaque scalar
  without coercing it. Conversion to a literal happens only through its
  explicit `onUseLiteral` action. If the opaque value is not editable as a
  string, keep it in a `SettingsField` and put the conversion button there so
  validation remains associated through `aria-invalid`/`aria-describedby`.
- `SettingsDeck` owns one-level collection navigation and focus transfer.
  Use `List`/`ListItem` inside a shared `Panel` for the overview rows.

The normal editable-row pattern is:

```tsx
<SettingsSection
  title="Connection pool"
  description="Bound concurrency for this connection."
>
  <SettingsList>
    <SettingsField
      id="pool-max"
      field="databases.primary.pool.max"
      label="Maximum connections"
      description="Upper bound for open connections."
      error={errors?.get('/databases/primary/pool/max')}
      controlSize="compact"
      renderControl={(controlProps) => (
        <Input
          {...controlProps}
          type="number"
          value={poolMax}
          onChange={setPoolMax}
        />
      )}
    />
  </SettingsList>
</SettingsSection>
```

`renderControl` must pass its received props to the interactive control. This
is what keeps labels clickable, errors announced, and a host-provided
`focusField` discoverable. Serialize the host path exactly as
`focusField.map(String).join('.')` and use the same dotted value in `field`.
Consume a focus request once, after conditional/deck content is mounted:

```tsx
const handledFocus = useRef('')
const focusKey = focusField?.length
  ? JSON.stringify(focusField.map(String))
  : ''

useEffect(() => {
  if (!focusKey) {
    handledFocus.current = ''
    return
  }
  if (handledFocus.current === focusKey || !rootRef.current) return
  const path = (JSON.parse(focusKey) as string[]).join('.')
  const target = rootRef.current.querySelector<HTMLElement>(
    `[data-field="${CSS.escape(path)}"]`,
  )
  if (!target) return
  handledFocus.current = focusKey
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'center' })
}, [focusKey, activeItemId])
```

Worker CSS may set layout, width, or a mono font for
machine-readable values; it must not replace a shared control's background,
border, radius, height, chevron, focus ring, disabled state, or typography.
Responsive behavior is based on the configuration pane's container, not only
the browser viewport; Back, section actions, row actions, and field controls
retain at least a 44 px target in a narrow split pane. Mark a standalone
empty-state action with `data-settings-narrow-action` to opt it into the same
container-responsive target.

Use a deck when a collection item has a meaningful sub-form. The overview and
detail are mutually exclusive at every width: selecting a row pushes the
detail over the configuration surface; Back returns to the overview and
restores focus to the originating row. Do not keep a squeezed desktop
master/detail layout or stack the list above the active editor on narrow panes.
If an action can remove the originating row, mark the preferred surviving
overview action with `data-settings-deck-fallback`; the deck focuses it when
the original control no longer exists.

```tsx
<SettingsDeck
  open={activeId !== null}
  title={activeItem?.label ?? 'Connection'}
  description="Connection settings"
  backLabel="Connections"
  backAriaLabel="Back to connections"
  overview={
    <SettingsSection
      title="Connections"
      action={<Button data-settings-deck-fallback>Add</Button>}
    >
      <Panel>
        <List role="group" aria-label="Configured connections">
          {items.map((item) => (
            <ListItem
              key={item.id}
              data-field={`connection-${item.id}`}
              label={item.label}
              description={item.summary}
              onClick={() => setActiveId(item.id)}
            />
          ))}
        </List>
      </Panel>
    </SettingsSection>
  }
  detail={activeItem ? <ConnectionSettings item={activeItem} /> : null}
  onBack={() => setActiveId(null)}
/>
```

Keep deck selection by a stable domain key, clear it when that item disappears,
and reset secret-reveal state when the active item changes. A deep link to a
detail must open that item before focusing the exact control; set
`autoFocusDetail={false}` only while the form owns that more specific focus.
Back changes navigation only—it never resets or saves the host-owned draft.

Every mutation starts from the current object and changes only its known key.
Preserve unknown root/nested siblings, unknown enum values, adapter payloads,
and `${ENV}` templates. Defaults may be displayed without materializing them
into the draft. Variant changes may remove fields owned exclusively by the old
variant, but must not discard opaque data silently. Treat an opaque top-level
configuration exactly like an opaque nested block: show/edit the raw string or
offer an explicit conversion; never normalize it to `{}` merely to render the
typed form.

At minimum, test: initial overview, open and Back, focus restoration, Add and
Remove, rename/collision behavior where applicable, a `focusField` into the
detail, wide and 320–430 px layouts, keyboard-only operation, 44 px narrow
targets, error associations, templates/unknown values, secret masking, and
preservation of unknown siblings. Database is the reference for a resource
deck; Cron is the reference for a small `SettingsField` form.

### `host.providerConfigForms.register(providerId, component)`

Replace the provider editor opened from the chat model picker (exact
`llm-router` provider id; last registration wins). This is the preferred
surface for provider-owned authentication nuances such as OAuth, a device
flow, or importing a login from a companion app. Feature-detect it when a
worker must support older consoles: `host.providerConfigForms?.register`.

```ts
interface ProviderConfigFormProps {
  providerId: string
  schema: Record<string, unknown> | null
  value: JsonValue
  onChange(next: JsonValue): void
  errors?: ReadonlyMap<string, string>
  configured?: boolean
  available?: boolean
  modelCount: number
}
```

The console still owns the authoritative `llm-router.providers[providerId]`
slice, schema validation, dirty guard, save/reset, and model refresh. The
provider owns only the form body and may call its own login/refresh functions
through `host.iii`. Never render a plaintext API-key field: tell operators to
use the provider's declared environment variable instead.

### `host.chat.registerSessionChip({ id, render })`

A small per-session status chip in the chat header's right cluster (beside
the export button and status dot), rendered for every open session. Your
component receives:

```ts
interface SessionChipProps {
  sessionId: string
  modelId?: string        // resolved model id, when known
  contextWindow?: number  // model context window (tokens), from the catalog
}
```

Duplicate ids: last registration wins. The id `context` is special: while a
`context` chip is registered, the console hides its built-in estimate-based
context meter — a worker with real per-turn numbers owns the surface. Chips
fetch their own data over `host.iii` (subscribe to your worker's triggers,
hydrate with a function call on mount); the host passes identity only.
Feature-detect on older consoles: `host.chat?.registerSessionChip`.

### `host.chat.registerComposerAction({ id, render })`

An icon-sized action in the composer's toolbar, rendered just before the
attach button on every composer layout (the narrow row and the wide right
cluster). Use it for input affordances that belong next to the caret:
dictation, a snippet picker, a template. Your component receives:

```ts
interface ComposerActionProps {
  sessionId: string | null  // null while the conversation has no session yet
  isStreaming: boolean      // the session is producing a turn
}
```

Render a single `IconButton` (16 px glyph, `label` as the accessible name)
sized to match the attach button; hand text back through
`host.chat.compose({ text })` and never submit on the user's behalf.
Duplicate ids: last registration wins. Feature-detect on older consoles:
`host.chat?.registerComposerAction`, and fall back to a session chip when it
is absent.

### `host.chat` conversation helpers

Newer Consoles expose two optional, non-rendering helpers on the same
feature-detected namespace:

- `host.chat?.selectConversation?.(sessionId)` switches the active
  conversation. Call it only after an explicit operator action such as opening
  a schedule's transcript; never steal focus on mount or during background
  refresh.
- `host.chat?.composerModel?.(conversationId?)` returns the live model id for
  that conversation, including an unsaved composer draft, or `null` when none
  is selected. Treat it as a read-only snapshot and keep a fallback for older
  Consoles.

Both methods are optional independently of `registerSessionChip`; check the
specific method before use.

- `host.chat?.compose?.({ text?, files? })` hands a draft to the active
  conversation's composer the way a drop or a paste would: files become
  attachments, text is appended, the caret lands in the composer. Call it
  from an explicit action (a "Send to chat" button, a command); never on
  mount. The browser page sends an annotated picture plus its notes this
  way.

### Annotations: pins with notes over a picture

`AnnotationLayer` is the shared primitive for marking up a captured
picture — a browser frame, a desktop frame, a screenshot in a transcript.
The page owns the list; the layer is a pure view of it. A pin sits at
fractions of the picture so the same set renders over the live view, the
exported PNG and a later reload. Pins are buttons: Delete removes, arrows
nudge, Shift+arrows nudge more. With `onNote` the selected pin opens a
callout beside it that edits the note in place (a new pin takes the caret;
Enter or Escape closes it), so the note is written where the pin is, not
in a list elsewhere. A pin may carry a `label`, what it points at when the
page can tell (the browser page resolves the element under a dropped pin
through `browser::pick::resolve`); the callout shows it under the note and
the chat text appends it in parentheses. Beyond pins, a mark can be a
`rect` or an `arrow` (`kind`, `x2`/`y2`, `color`): the page passes `tool`
and the `onAddShape` / `onResizeShape` / `onEndShape` trio and the layer
draws the shape on drag, dropping one smaller than `MIN_SHAPE_SIZE`;
`undoAnnotation` drops the newest mark. Exports paint every mark
(`paintMark`) and the chat text prefixes non-pin marks (`2. box: ...`).
A set can be saved to the `state` worker (scope `annotations`) so it
outlives the session: the browser page's Save action persists it, a
palette source lists saved sets by subject, and a dialog previews one over
its stored picture with send / download / delete.
`AnnotationList` renders the same
notes as rows for a page that wants a list too.

```tsx
<AnnotationLayer annotations={pins} image={{ width, height }} active={annotating}
  selectedId={selectedId} onAdd={add} onSelect={select} onMove={move}
  onRemove={remove} onNote={note}>
  <img src={frame.dataUrl} alt="frozen view" className="h-full w-full object-contain" />
</AnnotationLayer>
```

Rules a page follows: freeze the picture while annotating (pins drift over
a live frame); keep pins until they are sent or cleared; the actions (send,
download, clear) sit in the page header next to the mode toggle, not in a
separate pane; send through `host.chat.compose` as a stack of attachments
the reader can flip through — the whole view with the pins painted on
(`renderAnnotatedImage`) plus one crop per pin (`renderAnnotationCrop`,
named by number and note) — with the numbered notes as the text; register
`annotate`, `send-annotations`, `download-annotations` and
`clear-annotations` as commands (`C`, `Mod+Enter`); Escape ends the mode.

### The rest of `host`

| Surface | What it is |
|---|---|
| `host.iii` | The tab's bus client: `trigger(functionId, payload?, {timeoutMs?})`, `on(functionId, handler)` (returns un-listen), `registerTrigger({type, function_id, config})` (returns un-register), `addConnectionStateListener`, `browserId`. Injected UI *acts* by invoking its own worker's functions. |
| `host.panels` | Optional compatibility-gated contextual navigation: `open({ pageId, context })` places/reuses a registered page beside chat and sends it JSON context. |
| shared UI | Typed, zero-bundle-cost components include page chrome; `List`/`ListItem`, `Card`, `CollapsibleCard`, `CardHighlight`, `Panel`, `Chip`, `IconButton`; line `Tabs` and `SegmentedControl`; `Selector` and `Select`; buttons, inputs, dialogs, menus, tooltips; status/empty/loading surfaces; Markdown/JSON; `CodeEditor`, `FileDiff`; and terminal atoms. `uiClasses` supplies stable recipes and `tokens` supplies the canonical CSS-variable inventory. Import from `@iii-dev/console-ui`; `host.components` mirrors React components as an untyped compatibility record. Promote repeated cross-worker behavior here instead of carrying private copies. Monaco, diff, ANSI parsing, selector behavior, tooltip geometry, and portal scope are single shared contracts. |
| `host.useTheme()` | `'light' \| 'dark'`, reactive. Extensions follow the theme, never set it. |
| `host.path` | Your script's asset path. |

Live data pattern: a page can register its *own* trigger over `host.iii` (the
state page binds a `state`-type trigger with a
`iii::<worker>-ui::events::<browserId>` handler id — the `iii::` prefix keeps
per-event invocations span-suppressed and out of the trace feed). The binding
is GC'd with the tab.

### Containment

Every injected render is wrapped in the scope element plus an
`ErrorBoundary`: a render-time crash degrades to a chip naming your script
(`extension crashed · state/page.js`), never a white screen. A failed
`import()`, missing default export, or throwing `setup()` logs to the browser
console and your contributions simply drop out until the next good version
arrives — a broken extension never takes the console down.

Shared `Dialog`, `DropdownMenu`, `Select`, `Selector`, `Tooltip`, and
`BottomSheet` portals carry the current `data-iii-ui` scope automatically.
Only custom domain portals mounted directly under `document.body` must stamp
that attribute themselves.

## The dev loop (hot reload)

Rebuild-on-save stays in your build tool; **re-registration stays in the
worker process** (a Message-path trigger dies with whatever connection
registered it, so a registering build-tool would tie your UI's lifetime to
the watcher). The re-register discipline, per changed asset:

```text
1. build tool rewrites dist/<asset>
2. worker swaps the bytes it serves, registers a FRESH trigger for the same path
3. THEN unregisters the previous handle
```

Register-first avoids a zero-trigger window (a flash-dispose in tabs). The
trailing `unregister()` is contract, not tidiness: the console prunes
superseded rows from the *engine*, but your SDK's local replay map only
shrinks via `unregister()` — skip it and every reconnect replays your entire
rebuild history (harmless but churny).

For Rust workers the `iii-console-ui` crate implements exactly this as a
1 s content poller (`spawn_watcher`, `workers/crates/console-ui/src/lib.rs`),
armed by the watch env var — `III_<WORKER>_UI_WATCH`, set to `1` for
`ui/dist` or to an explicit build-output directory:

```bash
cd workers/state/ui && pnpm watch          # esbuild --watch → dist/
III_STATE_UI_WATCH=1 cargo run             # poll ui/dist/, re-register on change
```

Every open tab hot-swaps the asset in place — scripts re-`import()` +
re-`setup()` (React state in your slots is lost — dispose + remount, Vite
without react-refresh), styles link-swap with no flash. Unchanged content is
hash-deduped end to end.

Tricks:

- **Preview without restarting the owner**: any throwaway process can
  register a `console:script` trigger for a *preview* path (e.g.
  `state/page-preview.js`) serving experimental bytes — disconnect GCs it.
  Same-content re-registration is a no-op by hash; vary the content to force
  a push.
- The console worker's **debug build serves `web/dist` from disk live**
  (rust-embed debug mode) — console SPA changes need no console restart. A
  release-build worker that `include_str!`s its assets needs a worker rebuild
  (or its watch env) for UI changes.

## Debugging

| Surface | What you get |
|---|---|
| `console::ui-manifest` (function) or `GET :3113/ui` | `{ disabled, assets: [{ path, kind, hash, worker, warnings }], workers: [{ worker, enabled, assets }] }` — `assets` is the authoritative loadable set (held assets of toggled-off workers are excluded); `workers` summarizes every worker with registered assets, including disabled ones. `disabled: true` means the kill switch is on. `worker` on an asset is currently always `null`; the path prefix is the attribution. |
| `GET :3113/ui/<path>` | the served bytes (`Cache-Control: no-cache`, `ETag: "<hash>"`) |
| `engine::registered-triggers::list { trigger_type: "console:script" }` | engine's view: trigger ids, config, `worker_name` joined from the content function |
| Browser console | `[iii-ui] …` loader logs: import failures, cleanup throws, stylesheet load failures |

Common failures:

| Symptom | Cause |
|---|---|
| Registration rejected with a path error | path violates the rules table above (wrong extension, uppercase, `..`, …) |
| Registration rejected with a fetch error | your content function threw, returned no string `content`, or timed out (2 × 3 s budget) |
| "Invalid hook call" in the tab | your bundle contains a second React — a missing `external` |
| `import()` fails on a bare specifier | a dependency imports a react-family subpath outside the five shared specifiers |
| Styles apply on your page but not in a custom portal | Shared portalled components preserve scope; a custom `document.body` portal must carry `data-iii-ui="<worker>"` on its root |
| Whole console restyled | your sheet has unscoped rules — check `warnings` in the manifest |
| Asset gone after console restart | replay fetch failed (worker down at replay) — re-register, or just restart the worker |
| Registered without errors but never loads | the worker is toggled off — check `workers[].enabled` in the manifest / `injectableUi.disabledWorkers` in the `console` configuration entry |

Kill switch: `injectable_ui: false` in the console worker's `config.yaml`
disables the trigger types, the `/ui` + `/vendor` routes, and the SPA loader
(manifest answers `disabled: true`).

Per-worker toggle: global Settings → **Console** renders a toggle
board — one row per UI-shipping worker (title, description, and a switch;
enabled cards use the neutral selected recipe, disabled ones dim) — editing
`injectableUi.disabledWorkers` in the `console` configuration entry. Saving
applies live: the console worker subscribes to `configuration:updated` for
its own entry and pushes `delete`/`set` to every tab. The board is itself
injected UI (a `configForms` override shipped by the console worker —
`workers/console/ui/` + `workers/console/src/ui.rs`, registered through the
`iii-console-ui` crate), which is why the `console` worker is absent from
its own board: the registry refuses to disable it.

## Testing

- **The registration machinery is tested once, in the crate**
  (`workers/crates/console-ui/src/lib.rs` tests: path dispatch, unknown-path
  errors, content types, watch-env parsing, path validation). The content
  function's wire shape (`UiContentInput`/`UiContentResult`) lives there
  too. In your worker, assert what only it knows: the embedded build
  outputs (`workers/state/src/ui.rs` tests — nonempty ESM, scoped CSS) and
  that the builder accepts your asset list (constructing it runs the path
  validation).
- **Assert the built CSS is scoped** (the state worker's
  `embedded_styles_are_scoped` test; note esbuild prints the selector
  unquoted: `[data-iii-ui=state]`).
- **e2e smoke** without a browser: boot engine + console + your worker;
  assert `console::ui-manifest` lists your paths with non-empty hashes and
  empty `warnings`; `GET :3113/ui/<path>` returns your bytes; re-register
  with changed content and assert the hash moved.
- Rendering correctness stays a browser concern — Storybook in your worker's
  UI project against the workspace-linked `@iii-dev/console-ui` types is the
  recommended harness.
- For a trigger-activity renderer, test exact type matching, malformed-config
  fallthrough, every activity kind, generic fallback, and that the worker
  does not duplicate host-owned delivery or lifecycle status.
- Exercise light and dark themes at 320–430 px, narrow split-pane, and wide
  widths; keyboard and touch; reduced motion; loading/empty/error/success;
  streaming or rapidly updating data; and long content. Selected rows, cards,
  tabs, chips, and segments must remain neutral in both themes.
- Verify content tabs use the shared line recipe, natural-case sans labels,
  and default 16 px icons; global workspace tabs use weight 500. Verify there
  are no application icons below 16 px or panel-wide mono chrome.
- Record reusable-primitive coverage and justified local exceptions in
  `workers/docs/sops/console-ui-conformance.md`.
- The package's declarations are themselves pinned to the real console
  components by `console/web/src/lib/console-ui-conformance.test.ts`
  (type-level + name-manifest checks) — extend all three (manifest, record,
  declaration) when promoting a new shared component.

## Status: shipped vs spec

The implementation covers the spec's protocol, loader, and runtime slot
registry. The spec's composer slot (`host.composer`) shipped in two parts:
`host.chat.registerSessionChip` for status chips in the chat header's right
cluster, and `host.chat.registerComposerAction` for icon actions in the
composer toolbar itself. Not shipped yet (don't design against them):

| Spec item | Status |
|---|---|
| `@iii-dev/console-build` CLI + Tailwind preset | not implemented — hand-write scoped CSS (as `state` does) or scope your own Tailwind output; there is no automatic scoping pass to save you |
| Types package | shipped as `@iii-dev/console-ui` (`packages/console-ui`) — in-repo workers consume it **workspace-linked** (out-of-repo authors install it from npm, see `workers/console/SKILL.md`); the runtime module specifier was renamed from the spec's `@iii/console` |
| Rust worker-side registration | shipped **beyond spec** as the path-linked `iii-console-ui` crate (`crates/console-ui`) — the spec's authoring doc had each worker hand-roll the content function, triggers, and watcher |
| Named typed component exports on the runtime module | shipped (beyond spec: the spec only had the `components` record) |
| Manifest `worker` attribution | always `null` |
| Scope-preserving shared portals | shipped for `Dialog`, `DropdownMenu`, `Select`, `Selector`, `Tooltip`, and `BottomSheet`; stamp only custom domain portals |
| Page commands and pane-scoped keys | shipped **beyond spec** as `host.commands` + `PageRenderProps.commands`; chat and shell are the first consumers |
| Live palette sources | shipped **beyond spec** as `host.palette.registerSource` + `host.palette.open`; the shell's Files source is the reference |

Naming note: function messages use `host.functionTriggers` and the
`function-trigger` role. Normalized trigger lifecycle activities use the
separate `host.triggerRenderers` slot and `TriggerActivityMessage` contract.

## Security posture (know what you're shipping)

Injected scripts run with full console-origin privileges — deliberately the
same trust level as any worker on the bus, which can already invoke any
function. There is no sandbox and the `data-iii-ui` wrapper is styling
hygiene, not isolation. Hardened deployments gate trigger-type registration
via RBAC; the console's `/ws` proxy already drops browser-originated
trigger-type registrations. Details:
`iii/tech-specs/2026-07-17-injectable-ui/injection-protocol.md` § Security.
