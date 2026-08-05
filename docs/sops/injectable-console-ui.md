# Injectable console UI — shipping worker UI into the running console

How a worker ships React pages, function-trigger renderers, configuration
forms, and stylesheets into every open console tab **at runtime** — no console
rebuild, no iframe, hot-reloaded on re-registration. The full design rationale
lives in the tech spec (`iii/tech-specs/2026-07-17-injectable-ui/`); this SOP
is the operational guide for worker authors, grounded in the shipped
implementation. The `state` worker is the living reference — copy it.

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
    render: () => <StateManagerPage host={host} />,
  })
  host.functionTriggers.register(createStateTriggerRenderer(host))
  host.configForms.register('state', StateConfigForm)
  // optional: return a teardown fn; the loader runs it on dispose
}
```

`Button`, `EmptyState`, `Dialog`, `Markdown`, … are the console's own
components, re-exported by name with typed props — at runtime they come from
the running console's single React tree (via `/vendor/console-ui.js`), so
importing them adds **zero bytes** to your bundle. Use them instead of
copying base components into your worker.

### 2. The style asset (`ui/styles.css`)

Plain CSS, **every rule scoped under your worker's wrapper attribute**:

```css
[data-iii-ui="state"] .state-ui { color: var(--color-ink); }
@keyframes state-ui-flash { /* prefix keyframes names — they are global */ }
```

The console mounts every injected render inside
`<div data-iii-ui="<first path segment>" style="display:contents">`, so
scoped rules apply to your UI and nothing else. Use the console's design
tokens (`--color-bg`, `--color-ink`, `--color-ink-faint`, `--color-ink-ghost`,
`--color-accent`, `--color-accent-fg`, `--color-alert`, `--color-ok`,
`--color-warn`, `--color-panel`, `--color-paper-2`, `--color-ring`,
`--color-rule`, `--color-rule-2`; also exported as `tokens` from
`@iii-dev/console-ui`) — dark mode is a variable flip on `html[data-theme]`,
so token-based styles theme for free. Never hardcode theme colors.

What must NOT be in the sheet: unscoped selectors (`:root`, `html`, `body`,
`*`, bare element names) and `@font-face` — injected CSS is unlayered, so an
unscoped rule silently beats the console's fully-layered CSS document-wide.
The console runs a warn-only lint on every `console:style` fetch
(`lint_style`, `workers/console/src/ui_assets.rs`) and puts findings in the
manifest's `warnings` array; keep it empty.

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

Injected renderers dispatch **before** the first-party families
(`useFunctionTriggerRenderers`,
`workers/console/web/src/components/function-trigger/renderer-registry.tsx`),
so you can override built-in rendering for your worker's functions. Return
`null` to fall through to the next renderer — match narrowly (your own
function ids) and let errors and everything else keep the default cards.
Renderer callbacks are fenced: a throwing `isMatch` counts as no-match, a
throwing `tryRender` degrades to an error chip, never a broken feed.

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

### `host.configForms.register(configurationId, component)`

Replace the schema-generated form for one configuration entry on the Workers
tab (exact id match; last registration wins). Your component receives:

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

The form is render-level only: dirty tracking, validation, save/reset and the
SaveBar stay host-owned. You draw the fields and call `onChange`.

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

### The rest of `host`

| Surface | What it is |
|---|---|
| `host.iii` | The tab's bus client: `trigger(functionId, payload?, {timeoutMs?})`, `on(functionId, handler)` (returns un-listen), `registerTrigger({type, function_id, config})` (returns un-register), `addConnectionStateListener`, `browserId`. Injected UI *acts* by invoking its own worker's functions. |
| shared components | The curated, pre-styled component library: `Badge`, `Button`, `Dialog`(+`Trigger/Close/Content/Title/Description`), `DropdownMenu`(+parts), `EmptyState`, `ErrorBoundary`, `Input`, `Select`, `Skeleton`, `StatusDot`, `StatusPanel`, `Tabs`(+parts), `Tooltip`(+parts), `CodeEditor`, `CodeHighlight`, `JsonHighlight`, `Markdown`, `MarkdownPreview`. Import them by name from `@iii-dev/console-ui` (typed props); `host.components` carries the same objects as an untyped record. `CodeEditor` is **Monaco** — the console's one code editor, global by contract: every editing surface uses it (see the build-footgun note above; never ship your own). For richer components, copy the pattern into your worker — small duplication across workers is the accepted cost; `@iii-dev/console-ui` is deliberately the only versioned contract. |
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
| Styles apply on your page but not in a portal you created yourself | DOM you portal to `document.body` must carry `data-iii-ui="<worker>"` on its root |
| Whole console restyled | your sheet has unscoped rules — check `warnings` in the manifest |
| Asset gone after console restart | replay fetch failed (worker down at replay) — re-register, or just restart the worker |
| Registered without errors but never loads | the worker is toggled off — check `workers[].enabled` in the manifest / `injectableUi.disabledWorkers` in the `console` configuration entry |

Kill switch: `injectable_ui: false` in the console worker's `config.yaml`
disables the trigger types, the `/ui` + `/vendor` routes, and the SPA loader
(manifest answers `disabled: true`).

Per-worker toggle: the Workers tab's **Console** entry renders a toggle
board — one bordered card per UI-shipping worker (title, description, and a
switch; active cards carry the accent border, disabled ones dim) — editing
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
- The package's declarations are themselves pinned to the real console
  components by `console/web/src/lib/console-ui-conformance.test.ts`
  (type-level + name-manifest checks) — extend all three (manifest, record,
  declaration) when promoting a new shared component.

## Status: shipped vs spec

The 2026-07-21 implementation covers the spec's protocol, loader, and three
slot kinds. The spec's composer slot (`host.composer`) shipped later as
`host.chat.registerSessionChip` — chips render in the chat header's right
cluster, not the composer toolbar. Not shipped yet (don't design against
them):

| Spec item | Status |
|---|---|
| `@iii-dev/console-build` CLI + Tailwind preset | not implemented — hand-write scoped CSS (as `state` does) or scope your own Tailwind output; there is no automatic scoping pass to save you |
| Types package | shipped as `@iii-dev/console-ui` (`packages/console-ui`) — in-repo workers consume it **workspace-linked** (out-of-repo authors install it from npm, see `workers/console/SKILL.md`); the runtime module specifier was renamed from the spec's `@iii/console` |
| Rust worker-side registration | shipped **beyond spec** as the path-linked `iii-console-ui` crate (`crates/console-ui`) — the spec's authoring doc had each worker hand-roll the content function, triggers, and watcher |
| Named typed component exports on the runtime module | shipped (beyond spec: the spec only had the `components` record) |
| Manifest `worker` attribution | always `null` |
| Per-script `Dialog` re-export with scope-stamped portals | not implemented — stamp `data-iii-ui` yourself on DOM you portal outside the wrapper |

Naming note: the renderer slot is `host.functionTriggers` and the message
role is `function-trigger` — the console web codebase deliberately does not
use the older two-word term for these.

## Security posture (know what you're shipping)

Injected scripts run with full console-origin privileges — deliberately the
same trust level as any worker on the bus, which can already invoke any
function. There is no sandbox and the `data-iii-ui` wrapper is styling
hygiene, not isolation. Hardened deployments gate trigger-type registration
via RBAC; the console's `/ws` proxy already drops browser-originated
trigger-type registrations. Details:
`iii/tech-specs/2026-07-17-injectable-ui/injection-protocol.md` § Security.
