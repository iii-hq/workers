---
name: console-injectable-ui
description: Build and ship worker UI (React pages, function-trigger renderers, configuration forms, stylesheets) into a running iii console at runtime — using the @iii-dev/console-ui npm package and the iii-console-ui Rust crate. Use when a worker needs its own console page, custom message rendering, or a custom config form, with hot reload and no console rebuild.
---

# Injectable console UI

A worker can ship React pages, function-trigger renderers, configuration
forms, and stylesheets into every open console tab **at runtime** — no
console rebuild, no iframe, hot-reloaded on re-registration. This skill is
self-contained: everything needed to author, register, and debug injectable
UI from your own worker project is on this page.

## How it works (one paragraph)

The console owns three trigger types. A worker registers a `console:script`
or `console:style` trigger whose `config.path` (e.g. `mywork/page.js`) is
the asset's identity; the trigger's `function_id` names a *content function*
on the worker that the console invokes to fetch the source (`{path}` in,
`{content, content_type?}` out). The console hashes and caches the bytes,
serves them from its HTTP port (`GET /ui/<path>?v=<hash>`), and pushes an
update to every open tab over the third type, `console:assets` (tabs
subscribe; you never register that one). Scripts are ES modules the tab
`import()`s and calls `setup(host)` on; styles are `<link>` elements the tab
swaps in place. Re-registering the same path overrides it — that **is** the
hot-reload signal. Registration is deployment; disconnect is teardown.

## Install

Two packages, one per side of the wire:

- **`@iii-dev/console-ui`** (npm) — the compile-time surface of the
  console's runtime module: TypeScript types plus the component manifest.
  It is types-only by design: at runtime the console's import map serves
  the real module from the running SPA, so this package must stay
  `external` in your build (its js entry throws to make a forgotten
  external fail fast).

  ```bash
  npm install --save-dev @iii-dev/console-ui
  ```

- **`iii-console-ui`** (Rust crate) — the whole worker side for Rust
  workers: registers the content function, one trigger per asset, and the
  dev-loop file watcher.

  ```bash
  cargo add iii-console-ui
  ```

  Match the crate and package versions to the console worker you deploy
  against; the console is the runtime they both describe. (Node workers
  need no worker-side package — they hand-write the two registration
  pieces, shown below.)

## Project layout

```text
mywork/
  ui/
    page.tsx      # the script asset — default-exports setup(host)
    styles.css    # the style asset — every rule scoped
    build.mjs     # esbuild, five external specifiers
    package.json  # depends on @iii-dev/console-ui (dev)
  src/            # the worker itself (Rust or Node)
```

## 1. The script asset (`ui/page.tsx`)

Ordinary React. Import from `react` and `@iii-dev/console-ui` — both resolve
at runtime through the console's import map, so they must stay **external**
in your build. Default-export a `setup(host)` function and make every
registration through `host` (the loader attributes registrations to your
script so it can dispose them on reload):

```tsx
import { Button, EmptyState, type Host } from '@iii-dev/console-ui'

export default function setup(host: Host) {
  host.pages.register({
    id: 'mywork-manager',           // page URL: #/ext/mywork-manager
    title: 'mywork',                // nav label
    render: () => <MyPage host={host} />,
  })
  host.functionTriggers.register(createMyTriggerRenderer(host))
  host.configForms.register('mywork', MyConfigForm)
  // optional: return a teardown fn; the loader runs it on dispose
}
```

`Button`, `EmptyState`, `Dialog`, `Markdown`, … are the console's own
components, re-exported by name with typed props — at runtime they come from
the running console's single React tree, so importing them adds **zero
bytes** to your bundle. Use them instead of copying base components into
your worker.

### The shared component library

`Badge`, `Button`, `CodeEditor`, `CodeHighlight`, `Dialog` (+`DialogTrigger`,
`DialogClose`, `DialogContent`, `DialogTitle`, `DialogDescription`),
`DropdownMenu` (+`Trigger/Content/Item/Label/Separator`), `EmptyState`,
`ErrorBoundary`, `FileDiff`, `Input`, `JsonHighlight`, `Markdown`,
`MarkdownPreview`, `PageShell`/`PageHeader`/`PageBody`/`PageSidebar`/
`PageMain` (the page chrome — see below), `Select`, `Skeleton`, `StatusDot`,
`StatusPanel`, `Tabs` (+`TabsList/TabsTrigger/TabsContent`), `Tooltip`
(+`TooltipTrigger/TooltipContent`).

**The page chrome is the mandatory layout for pages.** Every registered
page composes the same five pieces, so your pane looks exactly like the
console's own screens (chat, traces) and every other worker's page:

```tsx
<PageShell>
  <PageHeader
    icon={<MyIcon />}                 // 16px glyph, faint ink
    title="mywork"                    // mono lowercase — console chrome
    description="what this page is"   // truncates first
    actions={<Button …/>}             // optional right-side controls
    onClose={onRequestClose}          // the standard ✕ (PageRenderProps)
  />
  <PageBody side={panelSide}>         {/* mirrors for right-side panes */}
    <PageSidebar>…navigation…</PageSidebar>  {/* gray, fixed width */}
    <PageMain>…workspace…</PageMain>         {/* white, flexes */}
  </PageBody>
</PageShell>
```

The pieces own the surface hierarchy (header on `--color-panel-raised`
with a hairline `--color-edge` border, sidebar on `--color-sidebar`, main
on `--color-panel`) — don't repaint those tokens yourself. No sidebar?
Put content straight into `PageMain`. Custom body internals are fine, but
keep `PageShell` + `PageHeader` (with `onClose` wired) on every page.

**`CodeEditor` is Monaco — and it is the console's one code editor.** Every
code/text editing surface (yours included) uses it: Monaco runs once inside
the console, follows the console theme in light and dark, and grows with its
content (put it inside an `overflow-auto` pane). Never bundle
`monaco-editor`, CodeMirror, or any other editor into a worker asset — it
would ship megabytes toward the per-asset size cap to duplicate what the
console already provides.

**`FileDiff` is the console's one file-diff surface** — same rule as
`CodeEditor`: never bundle a diff renderer. Pass the two full file bodies
(`oldFile`/`newFile`, each `{ name, contents }` — empty `contents` for a
created/deleted side); the console computes and renders the unified diff,
themed for both modes. `diffStyle: 'split'` and `overflow: 'scroll'` are
opt-in props.

```tsx
import { CodeEditor } from '@iii-dev/console-ui'

<CodeEditor
  value={draft}
  onChange={setDraft}
  language="markdown"      // Monaco language id: 'json', 'yaml', …
  placeholder="# notes…"
  aria-label="notes source"
/>
```

## 2. The style asset (`ui/styles.css`)

Plain CSS, **every rule scoped under your worker's wrapper attribute**:

```css
[data-iii-ui="mywork"] .mywork-ui { color: var(--color-ink); }
@keyframes mywork-flash { /* prefix keyframes names — they are global */ }
```

The console mounts every injected render inside
`<div data-iii-ui="<first path segment>" style="display:contents">`, so
scoped rules apply to your UI and nothing else. Use the console's design
tokens — `--color-bg`, `--color-ink`, `--color-ink-faint`,
`--color-ink-ghost`, `--color-accent`, `--color-accent-fg`, `--color-alert`,
`--color-ok`, `--color-warn`, `--color-panel`, `--color-paper-2`,
`--color-ring`, `--color-rule`, `--color-rule-2` (also exported as `tokens`
from `@iii-dev/console-ui`) — dark mode is a variable flip, so token-based
styles theme for free. Never hardcode theme colors.

What must NOT be in the sheet: unscoped selectors (`:root`, `html`, `body`,
`*`, bare element names) and `@font-face` — injected CSS is unlayered, so an
unscoped rule silently beats the console's fully-layered CSS document-wide.
The console lints every style on fetch (warn-only) and reports findings in
the manifest's `warnings` array; keep it empty.

## 3. The build (`ui/build.mjs`)

esbuild with the five shared specifiers external:

```js
import { build } from 'esbuild'

await build({
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: ['react', 'react-dom', 'react-dom/client',
             'react/jsx-runtime', '@iii-dev/console-ui'],
})
```

Everything else gets bundled in; keep output well under the console's 8 MiB
per-asset cap (a slot component should be tens of KiB). Three footguns:

- **A forgotten `react` external bundles a second React** — hooks resolve
  against the bundled copy's never-installed dispatcher and fail at runtime
  as a cryptic "Invalid hook call". (A forgotten `@iii-dev/console-ui`
  external fails loudly instead: the package's bundleable entry throws with
  the fix in the message.)
- **Only those five specifiers exist in the import map.** A transitive
  dependency importing any other bare react-family specifier
  (`react-dom/server`, …) fails at `import()` time, not build time.
- **Never bundle an editor** — use the shared Monaco-backed `CodeEditor`
  (above).

## 4. Registration (the worker side)

The wire contract is: one content function serving all of the worker's
assets (dispatch on `path`), one trigger per asset.

### Rust workers — the `iii-console-ui` crate

```toml
# Cargo.toml
[dependencies]
iii-console-ui = "0.1"
```

```rust
use iii_console_ui::ConsoleUi;

ConsoleUi::new("mywork")
    .script(
        "mywork/page.js",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js")),
    )
    .style(
        "mywork/styles.css",
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
registration failures at runtime stay warn-logged, not fatal. Embedding
`dist/` with `include_str!` (rebuilt from `build.rs`) keeps the worker one
self-contained binary.

### Node workers — write the two pieces directly

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

**Always register triggers through your SDK's Message path, never through
the engine's durable `register_trigger` function.** Function-path triggers
outlive your worker (a page pointing at a dead content function) and
silently vanish on engine restart with no replayer. Message-path triggers
are GC'd on disconnect and replayed by the SDK on reconnect — injected UI
dies and revives with its worker, which is the design.

Ordering never matters: register before the console is up and the engine
parks the intent, delivering it when the console arrives; console restarts
replay every live binding; engine restarts are absorbed by SDK
re-registration.

## The wire contract (what the console enforces)

| | |
|---|---|
| Trigger types | `console:script` (ESM JS), `console:style` (CSS), `console:assets` (tab subscriptions — you don't register these) |
| Trigger config | `{ "path": string }`, nothing else |
| Path rules | lowercase `[a-z0-9._-]` segments, no leading slash, no `.`/`..` segments, ≤ 512 chars; extension must match the type (`.js` / `.css`); **convention: first segment = your worker name** — it becomes the `data-iii-ui` scope and the only human-readable attribution |
| Content function | input `{ "path": string }` → output `{ "content": string, "content_type"?: string }` (`content_type` defaults from the asset kind) |
| Size cap | 8 MiB per asset — registrations over it are rejected |
| Fetch budget | 2 attempts × 3 s at live registration (a failed fetch **rejects the registration**; the error reaches your SDK's registration result); 3 × 5 s on console-restart replay (failure drops the asset until you re-register) |
| Override | same path re-registered ⇒ last writer wins, console-wide (even across workers); the superseded engine row is pruned |
| Identity/versioning | content hash (first 16 hex of sha256) — unchanged content re-registered is a no-op |
| Per-worker toggle | a worker listed in the console configuration's `injectableUi.disabledWorkers` has its registrations **accepted but held**: no serve, no manifest row, no tab pushes. Toggling back on pushes everything held to every open tab. The `console` worker itself cannot be disabled. |

## What `setup(host)` can register

All registration goes through the per-script `host`; every entry is disposed
automatically on hot reload and worker disconnect. Each `register` also
returns a remover for manual teardown.

### `host.pages.register({ id, title, render })`

A whole console page at `#/ext/<id>`, listed in the nav while registered.
Duplicate ids: last registration wins.

`render` receives `PageRenderProps` — a plain `() => <Page />` render stays
valid and simply ignores them:

- `panelSide`: `'left' | 'right'` — which side of the workspace tab the
  pane hosting your page occupies (`'right'` only for the rightmost column
  of a multi-column tab). Use it to mirror your layout so e.g. a sidebar
  hugs the outer screen edge.
- `tabId`: the hosting workspace tab's stable id (tabs persist across
  reloads) — key per-tab UI state on it. Empty string outside a workspace
  tab.
- `onRequestClose`: close the pane hosting your page (a split drops the
  column; a single-column tab detaches back to the attach affordance).
  Wire it to `PageHeader`'s `onClose` — every page header carries the
  standard ✕. Absent when the page renders outside a closable pane.

### `host.functionTriggers.register(renderer)`

Custom rendering for function-trigger messages in chat and traces:

```ts
interface FunctionTriggerRenderer {
  id: string
  isMatch(functionId: string): boolean
  tryRender(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderRunning?(message: FunctionTriggerMessage): React.ReactNode | null
  tryRenderPreview?(message: FunctionTriggerMessage): React.ReactNode | null
  FunctionIdLabel?: React.ComponentType<{ functionId: string }>
  redactRaw?(value: unknown): unknown
}
```

Injected renderers dispatch **before** the console's first-party families,
so you can override built-in rendering for your worker's functions. Return
`null` to fall through — match narrowly (your own function ids) and let
errors and everything else keep the default cards. Renderer callbacks are
fenced: a throwing `isMatch` counts as no-match, a throwing `tryRender`
degrades to an error chip, never a broken feed.

#### `redactRaw` — your card is not the only exit

However your card renders a call, the settled card also mounts a **`raw
json` tab** showing `input` and `output` verbatim, each with a copy button.
So hiding a secret inside your own rendering does not contain it: it is one
click away in the raw tab and on the clipboard.

`redactRaw` lets you declare what is secret and have the console apply it.
For a message your `isMatch` claims, the console passes the request and the
response through it **before the raw panes render and before the copy button
builds its text** (first claiming renderer that declares it wins). Keep the
knowledge of what a secret looks like in your worker — the console never
learns your patterns.

```ts
redactRaw: (value) => deepReplace(value, SECRET_PATTERN, mask)
```

Rules:

- Deep-walk the value. Secrets hide in nested arrays, in captured log lines,
  in error messages, and in object **keys**, not just in the obvious field.
  Preserve shape (objects, arrays, strings, numbers, booleans, `null`,
  `undefined`) — the value is not always an object: `FunctionTriggerCard`
  calls `redactRaw(undefined)` on every running/pending card (no `output`
  yet) and hands it a bare top-level string for a double-encoded payload.
  Guard against cycles so a self-referential value cannot hang the console.
- Pure and total: never mutate the argument, never do I/O, never throw. It
  runs inside the card's render.
- It is fenced and **fails closed**: if it throws, the pane and the clipboard
  get `[redaction failed — value withheld]`, not the raw value. A bug in your
  redactor costs the raw view, never the secret.
- It is display hygiene for the chat surface, not access control: the payload
  still travelled over the wire and still sits in the trace store, and a full
  session export is verbatim by design.

### `host.configForms.register(configurationId, component)`

Replace the schema-generated form for one configuration entry on the Workers
tab (exact id match; last registration wins). Your component receives:

```ts
interface ConfigFormProps {
  id: string
  schema: Record<string, unknown> | null   // null = value registered without a schema
  value: JsonValue
  onChange(next: JsonValue): void          // propose the full next value
  errors?: ReadonlyMap<string, string>     // JSON-pointer → message
  focusField?: readonly string[]           // deep-link focus request — honoring it is your job
}
```

The form is render-level only: dirty tracking, validation, save/reset stay
host-owned. You draw the fields and call `onChange`.

### `host.chat.registerSessionChip({ id, render })`

A small per-session status chip in the chat header's right cluster,
rendered for every open session. Your component receives:

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
fetch their own data over `host.iii`; the host passes identity only.
Feature-detect on older consoles: `host.chat?.registerSessionChip`.

### The rest of `host`

| Surface | What it is |
|---|---|
| `host.iii` | The tab's bus client: `trigger(functionId, payload?, {timeoutMs?})`, `on(functionId, handler)` (returns un-listen), `registerTrigger({type, function_id, config})` (returns un-register), `addConnectionStateListener`, `browserId`. Injected UI *acts* by invoking its own worker's functions. |
| `host.components` | The shared component library as an untyped record (same objects as the named exports). |
| `host.useTheme()` | `'light' \| 'dark'`, reactive. Extensions follow the theme, never set it. |
| `host.path` | Your script's asset path. |

Live data pattern: a page can register its *own* trigger over `host.iii`
with a handler id like `iii::<worker>-ui::events::<browserId>` (the `iii::`
prefix keeps per-event invocations out of the trace feed). The binding is
GC'd with the tab.

### Containment

Every injected render is wrapped in the scope element plus an error
boundary: a render-time crash degrades to a chip naming your script, never a
white screen. A failed `import()`, missing default export, or throwing
`setup()` logs to the browser console and your contributions simply drop out
until the next good version arrives — a broken extension never takes the
console down.

## The dev loop (hot reload)

Rebuild-on-save stays in your build tool; **re-registration stays in the
worker process** (a Message-path trigger dies with whatever connection
registered it). The re-register discipline, per changed asset:

```text
1. build tool rewrites dist/<asset>
2. worker swaps the bytes it serves, registers a FRESH trigger for the same path
3. THEN unregisters the previous handle
```

Register-first avoids a zero-trigger window (a flash-dispose in tabs). The
trailing `unregister()` is contract, not tidiness: the console prunes
superseded rows from the engine, but your SDK's local replay map only
shrinks via `unregister()` — skip it and every reconnect replays your entire
rebuild history (harmless but churny).

For Rust workers the `iii-console-ui` crate implements exactly this as a
1 s content poller, armed by the watch env var — `III_<WORKER>_UI_WATCH`,
set to `1` for `ui/dist` or to an explicit build-output directory:

```bash
cd mywork/ui && npm run watch      # esbuild --watch → dist/
III_MYWORK_UI_WATCH=1 cargo run    # poll ui/dist/, re-register on change
```

Every open tab hot-swaps the asset in place — scripts re-`import()` +
re-`setup()` (React state in your slots is lost — dispose + remount), styles
link-swap with no flash. Unchanged content is hash-deduped end to end.

Preview trick: any throwaway process can register a `console:script`
trigger for a *preview* path (e.g. `mywork/page-preview.js`) serving
experimental bytes — disconnect GCs it. Same-content re-registration is a
no-op by hash; vary the content to force a push.

## Debugging

| Surface | What you get |
|---|---|
| `console::ui-manifest` (function) or `GET <console-host>:3113/ui` | `{ disabled, assets: [{ path, kind, hash, worker, warnings }], workers: [{ worker, enabled, assets }] }` — `assets` is the authoritative loadable set; `disabled: true` means the kill switch is on |
| `GET <console-host>:3113/ui/<path>` | the served bytes (`Cache-Control: no-cache`, `ETag: "<hash>"`) |
| `engine::registered-triggers::list { trigger_type: "console:script" }` | the engine's view: trigger ids, config, worker attribution |
| Browser console | `[iii-ui] …` loader logs: import failures, cleanup throws, stylesheet load failures |

Common failures:

| Symptom | Cause |
|---|---|
| Registration rejected with a path error | path violates the rules table (wrong extension, uppercase, `..`, …) |
| Registration rejected with a fetch error | your content function threw, returned no string `content`, or timed out |
| "Invalid hook call" in the tab | your bundle contains a second React — a missing `external` |
| `import()` fails on a bare specifier | a dependency imports a react-family subpath outside the five shared specifiers |
| Styles apply on your page but not in a portal you created | DOM you portal to `document.body` must carry `data-iii-ui="<worker>"` on its root |
| Whole console restyled | your sheet has unscoped rules — check `warnings` in the manifest |
| Asset gone after console restart | replay fetch failed (worker down at replay) — re-register, or restart the worker |
| Registered without errors but never loads | the worker is toggled off — check `workers[].enabled` in the manifest / `injectableUi.disabledWorkers` in the `console` configuration entry |

Kill switch: `injectable_ui: false` in the console worker's configuration
disables the trigger types, the `/ui` + `/vendor` routes, and the loader
(manifest answers `disabled: true`).

## Testing your worker's UI

- Assert the embedded build outputs: nonempty ESM for scripts, and that
  every CSS rule is scoped under your `data-iii-ui` attribute (build tools
  may print the selector unquoted: `[data-iii-ui=mywork]`).
- e2e smoke without a browser: boot engine + console + your worker; assert
  `console::ui-manifest` lists your paths with non-empty hashes and empty
  `warnings`; `GET /ui/<path>` returns your bytes; re-register with changed
  content and assert the hash moved.
- Rendering correctness stays a browser concern — Storybook in your UI
  project against the `@iii-dev/console-ui` types is the recommended
  harness.

## Security posture (know what you're shipping)

Injected scripts run with full console-origin privileges — deliberately the
same trust level as any worker on the bus, which can already invoke any
function. There is no sandbox and the `data-iii-ui` wrapper is styling
hygiene, not isolation. Hardened deployments gate trigger-type registration
via RBAC; the console's `/ws` proxy already drops browser-originated
trigger-type registrations.
