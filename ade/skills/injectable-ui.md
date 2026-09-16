---
name: console-injectable-ui
description: The authoring and delivery contract for worker UI injected into the running iii console — project layout, setup(host) and every slot, the runtime wire contract, Rust and Node registration, the shared build driver and its lint, scoped CSS, the shared hooks/format/icon packages, the dev loop, debugging, testing layers and the definition of done. Use when adding or changing a worker's console UI. Visual rules and every number live in ade/design-system; responsive UX and configuration forms live in ade/design-console-ui.
---

# Injectable console UI

A worker ships pages, function and trigger-activity renderers, forms, and
stylesheets into every console tab **at runtime** — no console rebuild, no
iframe, hot reload. This skill is the delivery contract. Looks, tokens and
every number: `ade/design-system` (link to `ade/design-system` › Numbers,
never restate one). Behavior across widths and forms: `ade/design-console-ui`.

## How it works

A worker registers `console:script` and `console:style` triggers whose
`config.path` identifies an asset and whose `function_id` serves `{content}`
for `{path}`. The console hashes and serves those bytes, then pushes changes
to open tabs. Tabs `import()` scripts and call their default `setup(host)`;
styles load as scoped `<link>` assets. Re-registering a path hot-reloads it.
Registration is deployment; disconnect is teardown.

## Project layout

```text
mywork/
  build.rs        # ensure dist assets exist before include_str!
  ui/
    page.tsx      # the script asset — default-exports setup(host)
    styles.css    # the style asset — every rule scoped
    build.mjs     # buildWorkerUi({ scope }) — the shared driver
    package.json  # workspace dependency on @iii-dev/console-ui
    tsconfig.json # extends the shared worker tsconfig
    src/          # page, renderers, config form, widgets
  src/
    ui.rs         # embed and register dist/page.js + dist/styles.css
```

Both sides of the contract are versioned in this repository; nothing is
installed from a registry.

1. Add `<worker>/ui` to the root `pnpm-workspace.yaml` `packages` list.
2. `ui/package.json` (copy `state/ui/package.json`): private, `"type":
   "module"`, scripts `build: tsc --noEmit && node build.mjs` and `watch:
   node build.mjs --watch`, `"dependencies": { "@iii-dev/console-ui":
   "workspace:*" }`, and `@types/react`, `@types/react-dom`, `esbuild`,
   `lucide-react`, `react`, `react-dom`, `typescript` as `"catalog:"`
   devDependencies — the workspace `catalog:` in `pnpm-workspace.yaml` pins
   every toolchain version once.
3. `ui/tsconfig.json` is `{ "extends": "@iii-dev/console-ui/tsconfig.worker-ui.json", "include": ["page.tsx", "src"] }`.
4. `ui/build.mjs` is the whole driver call: `import { buildWorkerUi } from '@iii-dev/console-ui/build-worker-ui'`, then `await buildWorkerUi({ scope: 'mywork' })`.
5. A Rust worker adds `iii-console-ui = { path = "../crates/console-ui" }` to `<worker>/Cargo.toml`.

`@iii-dev/console-ui`'s root is types-only at build time; the console serves
its runtime from the running SPA. `iii-console-ui` registers the content
function, the asset triggers, and the development watcher; Node workers
implement the wire contract directly. Copy `state/build.rs` and
`state/src/ui.rs`; rename worker and asset paths and keep their tests.

## Authoring workflow

1. Read `packages/console-ui/index.d.ts`, `hooks.d.mts` and `format.d.mts`;
   never guess a component, hook or prop.
2. Select only the needed slots; model the primary object, navigation,
   actions, async states, and what must survive navigation or reload.
3. Choose one archetype (below); design wide and narrow flows separately.
4. Build with shared primitives and minimal scoped CSS; build (the driver
   scopes, checks tokens, lints), register, inspect the manifest, exercise
   the real console.

### Living references

| Need | Read | Reuse |
|---|---|---|
| Public API | `packages/console-ui/index.d.ts`, `hooks.d.mts`, `format.d.mts` | Exact exports and props |
| Shared page chrome | `ade/web/src/components/ui/PageChrome.tsx` | `PageShell`, `PageHeader`, surface roles |
| Migrated page, strict lint | `browser/ui/page.tsx`, `browser/ui/src/page/`, `browser/ui/styles.css` | Shared hooks/format/icons, `Toolbar`, `Eyebrow`, overlays, `lint: { strict: true }` |
| Migrated list/detail editor | `iii-directory/ui/page.tsx`, `iii-directory/ui/src/page/`, `iii-directory/ui/styles.css` | `SearchField`, `MetaRow`/`ActionLine`, `Kbd`/`KeyCombo`, dirty-draft guards, per-tab state |
| Migrated workbench | `ide/ui/page.tsx`, `ide/ui/src/page/`, `ide/ui/build.mjs` | `keyframePrefixes`, `allowUnscopedSelectors` for vendor CSS, `CodeEditor`/`FileDiff`, terminal atoms |
| Minimal template | `state/ui/page.tsx`, `state/ui/src/page/browser.tsx`, `state/ui/styles.css` | Smallest complete page + renderer + config form |
| Trigger-activity renderer | `cron/ui/src/trigger-activity/`, `cron/src/ui.rs` | `host.triggerRenderers` and the canonical small settings form |
| Rust delivery | `state/src/ui.rs`, `state/build.rs` | Embedding, registration, asset tests, build freshness |

Copy delivery plumbing when it matches; never copy a reference's sidebar
count, thresholds, controls, or visual hierarchy without deriving them from
the new worker's content. The migration recipe and the order of the remaining
workers: `docs/plans/2026-09-16-worker-ui-migration.md`.

### Archetypes

Choose one dominant archetype before writing JSX; mixing them produces a
generic dashboard. Shapes, wide/narrow flows and references are in
`ade/design-console-ui` › Archetypes.

- Console catalog — many searchable objects with rich detail.
- Database workbench — several tools on one selected resource.
- Directory editor — searchable documents with drafts or preview.
- State explorer — deep but compact hierarchy.
- Settings flow — one configuration entry, host-owned persistence.
- Terminal/instrument — one live surface with a toolbar and status bar.

## 1. The script asset (`ui/page.tsx`)

Ordinary React. `react`, `@iii-dev/console-ui` and `lucide-react` resolve at
runtime through the console's import map, so they stay **external** (the
driver does this). Default-export `setup(host)` and make every registration
through `host`: the loader attributes registrations to the script and
disposes them on reload. `setup` may return a disposer (`SetupFn` in
`index.d.ts`); the loader runs it and the registrations LIFO.

```tsx
import { type Host, PageHeader, PageMain, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { Boxes } from 'lucide-react'

function MyworkPage({ host, onRequestClose }: PageRenderProps & { host: Host }) {
  return (
    <PageShell className="mywork-ui-shell">
      <PageHeader icon={<Boxes />} title="Mywork" description={host.path} onClose={onRequestClose} />
      <PageMain className="mywork-ui-main">{/* the chosen archetype */}</PageMain>
    </PageShell>
  )
}

export default function setup(host: Host) {
  host.pages.register({
    id: 'mywork-manager',       // page URL: #/ext/mywork-manager
    title: 'Mywork',            // nav label
    configurationId: 'mywork',  // host adds the standard settings action
    render: (props) => <MyworkPage host={host} {...props} />,
  })
}
```

Imports from the shared package and from `lucide-react` add zero bundle bytes.

### Slots

Every `register` returns a remover and is disposed automatically on hot
reload and worker disconnect. Namespaces marked `?` are absent on older
consoles: feature-detect them.

| Surface | What it is |
|---|---|
| `host.pages` | `register({ id, title, configurationId?, render })` creates `#/ext/<id>` and a nav entry. `render` receives `PageRenderProps` (below). Set `configurationId` when the worker has a configuration entry; the host places the one settings action in `PageHeader`. Never mount `WorkerConfigurationDialog` yourself. |
| `host.functionTriggers` | Chat/trace renderers. Match only the worker's function ids; return `null` to fall through. `message.description` is the harness's short activity label. `metadata: { display: true }` keeps a successful rich artifact visible while raw details stay collapsed. If raw data can contain secrets, implement a pure, total, cycle-safe `redactRaw`; the raw tab and copy action otherwise expose the original input/output. |
| `host.triggerRenderers?` | Layered trigger presentation; see below. |
| `host.configForms` | The deliberate form for one configuration entry in global Settings. There is no schema-generated fallback: every configurable worker registers one. The host owns dirty tracking, validation, save, reset and the SaveBar; honor `focusField`. `{ layout: 'full' }` only for a workbench that owns its scrolling. Form anatomy and primitives: `ade/design-console-ui` › Configuration forms. |
| `host.providerConfigForms?` | Replace the form body for one exact `llm-router` provider id inside the model picker; provider-owned OAuth, device flow or companion login. Never solicit a plaintext API key. Props: `ProviderConfigFormProps`. |
| `host.chat?` | `registerSessionChip`, `registerTurnSummary?`, `registerComposerAction?`, `registerTranscriptRenderer?`, `compose?`, `openDraft?`, `selectConversation?`, `composerModel?`, `requestWorkingDirectoryChange?`, `requestThinkingLevelChange?`. Feature-detect each method. |
| `host.panels?` | `open({ pageId, context })` places or reuses a registered page beside chat and delivers `panelContext`. Pass opaque ids; fetch bodies from the worker. |
| `host.overlays?` | `register({ id, render })` — a floating layer over the workspace (the browser's live preview). Fall back to the page when absent. |
| `host.palette?` | `registerSource({ id, title, kind, prefix?, minQuery?, search })` adds live rows to the command palette; `open({ query? })`. |
| `host.commands?` | `register(pageId, commands)` — palette rows for a page that may not be open yet (`run` usually calls `panels.open`). A mounted page contributes keys through `PageRenderProps.commands`. |
| `host.iii` | The tab's bus client: `trigger(functionId, payload?, { timeoutMs? })`, `on(functionId, handler)`, `registerTrigger({ type, function_id, config })`, `addConnectionStateListener`, `browserId`. Injected UI *acts* by invoking its own worker's functions. |
| `host.components`, `host.path`, `host.useTheme`, `host.uiClasses`, `host.workspace?`, `host.screen?` | Runtime component record, the current asset path, theme, class recipes, recent directories, visible-screen lease. |

`PageRenderProps`: `panelSide` (`'left' | 'right'`, only to keep wide side
navigation on the outer edge); `tabId` and `paneId?` (key persisted UI state
and per-instance resources on `paneId` — the same page can be open in two
columns of one tab — falling back to `tabId` on older consoles);
`onRequestClose?` (wire to `PageHeader.onClose`); `workingDir?` (the active
conversation's live directory, for filesystem-shaped pages only);
`panelContext?` and `conversationId?`; `setDirty?` (report unsaved work) and
`commands?` (palette rows and pane-scoped keys, registered from an effect).

Live data: a page registers its own trigger over `host.iii` with a handler id
like `iii::<worker>-ui::events::<browserId>` (the `iii::` prefix keeps it out
of the trace feed; the binding is GC'd with the tab) — `useWorkerLive` from
`@iii-dev/console-ui/hooks` wraps fetch + bindings + visible-tab poll. Every
injected render is error-bounded: import or setup failures drop the
extension's contribution and log to the browser console. Scripts run with full
console-origin privileges; the wrapper scopes styles, it is not a sandbox.

### Trigger renderers: layered ownership

```ts
interface TriggerActivityRenderer {
  id: string
  isMatch(triggerType: string): boolean
  tryRender(activity: TriggerActivityMessage): React.ReactNode | null
  tryRenderDetails?(activity: TriggerActivityMessage): React.ReactNode | null
  tryRenderDisplay?(activity: TriggerActivityMessage): React.ReactNode | null
  redactRaw?(value: unknown): unknown
}
```

`TriggerActivityMessage.kind` is `registration`, `fired`, or `retirement`;
the model carries `triggerType`, opaque `config`, optional `label` and
`action`, delivery, lifecycle and payload/outcome fields. Match `triggerType`
(many sources share `engine::register_trigger`), parse config without
throwing, return `null` per slot to fall through. `tryRender` is the
source-specific section inside the generic detail view; `tryRenderDetails`
replaces the whole expanded Terminal tab and must carry the lifecycle and
delivery facts the host no longer adds; `tryRenderDisplay` is the compact
timeline content inside the host's disclosure button — non-interactive, one
line, truncation-safe; `redactRaw` is pure, non-mutating, total, cycle-safe,
and a throw fails closed. The host owns the click target, expanded state,
motion, isolation, per-slot fallback and the Raw JSON tab; a once firing and
its automatic retirement are one activity.

For harness registrations, `label` names the binding and `metadata.action`
describes the future event:

```json
{ "trigger_type": "on-message", "config": { "scope": "explorer" },
  "label": "explorer-messages", "metadata": { "action": "new Explorer message received" } }
```

Registration and active-binding surfaces show `label`; show `action` only
when `activity.kind === 'fired'`. Action affects presentation only.

### Shared components, hooks, format, icons

`packages/console-ui/index.d.ts` is the only list of runtime exports: page
chrome, `List`/`ListItem`, cards, `Panel`, `Chip`/`Badge`, `IconButton`, the
`Table` family, line `Tabs`/`SegmentedControl`, `Select`/`Selector`, inputs,
`Dialog`/`ConfirmDialog`/`DropdownMenu`/`Tooltip`/`BottomSheet`, status and
empty states, `Eyebrow`, `SearchField`, `Toolbar`/`StatusBar`,
`MetaRow`/`ActionLine`, `Kbd`/`KeyCombo`, `LiveRegion`, Markdown/JSON/code
renderers, the terminal atoms (`AnsiText`, `TerminalStream`,
`TerminalCommandLine`), `CodeEditor`, `FileDiff`, `ImageViewer`,
`ModelPicker`, `DirectoryPicker`, and the settings primitives
(`SettingsSection`/`SettingsList`/`SettingsRow`/`SettingsField`,
`RawValueInput`, `SettingsDeck`). `<Tooltip label="…">` is the one-line
tooltip. Confirmation is `useConfirm()` (render `dialog`, `await confirm({ … })`)
or `ConfirmDialog` — never `window.confirm`. `uiClasses` holds the stable
class recipes (`list*`, `tree*`, `card*`, `panel*`, `chip`, `table*`,
`tabs*`, `field*`, `settings*`, `eyebrow`, `toolbar`/`toolbarEnd`,
`statusbar`, `spin`, `pulse`) and `tokens` the CSS variable inventory. When
to use each: `ade/design-system` › Shared components.

Two subpaths **bundle** (React-free code the console itself uses):

- `@iii-dev/console-ui/hooks` — `useContainerNarrow({ below? })` (attach
  `ref` to the pane root; `narrow` while the pane is below the shared default
  from `ade/design-system` › Numbers or your `below`; synchronous first
  measure, resizes observed, zero widths ignored), `usePaneState(key,
  initial)` (`localStorage`-mirrored, best effort), `useCopyFlash(text, ms?)`,
  `useWorkerLive({ iii, triggers, fetch, pollMs?, handlerId })`.
- `@iii-dev/console-ui/format` — `formatRelative`, `formatDuration`,
  `formatBytes`, `errorMessage`, `errorCode`, `copyText`.

Import these instead of keeping a local copy; the migration plan lists the
copies still to be replaced.

Icons are `lucide-react`, an external shared with the console: `import { X }
from 'lucide-react'`. Never hand-write SVG icons (the lint flags them) and
never add another icon dependency. Sizes: `ade/design-system` › Numbers.
Never bundle Monaco, CodeMirror, a diff renderer, or an ANSI parser; use
`CodeEditor`, `FileDiff`, and the terminal atoms.

## 2. The style asset (`ui/styles.css`)

Plain CSS, **every top-level rule scoped under the worker's wrapper
attribute** — the console mounts each render inside
`<div data-iii-ui="<first path segment>" style="display:contents">`:

```css
[data-iii-ui="mywork"] .mywork-ui-main {
  min-width: 0;
  min-height: 0;
  overflow: auto;
}
@keyframes mywork-flash { /* keyframe names are global: prefix them */ }
```

- Colors, fonts, radius, shadows and motion are tokens: `var(--color-…)`,
  `var(--font-sans|mono|code)`, `var(--radius-…)`, `var(--shadow-…)`,
  `var(--motion-duration-…)`/`var(--motion-ease-…)`. `checkTokens` fails the
  build on a token the console does not define. Which token means what:
  `ade/design-system` › Tokens.
- Keyframe names carry the worker prefix (`keyframePrefixes`, default
  `[scope, "<scope>-ui"]`); spin/pulse are `uiClasses.spin`/`uiClasses.pulse`.
- Responsive layout is `@container` on the pane (every `PageShell` is a
  container), never a viewport `@media`; the viewport breakpoint is reserved
  for the console's phone chrome
  (`ade/web/src/lib/viewport-breakpoint-conformance.test.ts`).
- No unscoped selectors (`:root`, `html`, `body`, `*`, bare elements), no
  `@font-face`: injected CSS is unlayered and would beat the console's
  layered stylesheet document-wide. `assertScoped` refuses the build; the
  console's fetch-time lint reports leftovers in the manifest's `warnings`.
- No Tailwind utility classes in injected markup — worker class names are not
  in the console's compiled output. Shared components and `uiClasses` first;
  scoped CSS only for domain layout and data visualization.
- Scope `@media (prefers-reduced-motion: reduce)` overrides too (shared
  recipes already honor it); streaming, rapidly updating and pointer-following
  values update without transitions.
- Shared `Dialog`, `DropdownMenu`, `Select`, `Selector`, `Tooltip`, and
  `BottomSheet` portals preserve the worker scope. A custom `document.body`
  portal stamps `data-iii-ui="<worker>"` on its root (and lists it in
  `allowUnscopedSelectors` if its rules live outside the scope).

## 3. The build (`ui/build.mjs`)

`buildWorkerUi` (`packages/console-ui/build-worker-ui.mjs`, typed in
`build-worker-ui.d.mts`) is the one esbuild driver:

| Option | Default | Purpose |
|---|---|---|
| `scope` | required | The `data-iii-ui` value — first asset path segment, normally the worker name |
| `entryPoints` | `['page.tsx', 'styles.css']` | Extra scripts each need their own `console:script` trigger and a default `setup` |
| `outdir`, `root` | `'dist'`, `process.cwd()` | Pass `root: import.meta.dirname` when invoked from elsewhere |
| `keyframePrefixes` | `[scope, "<scope>-ui"]` | Allowed `@keyframes` name prefixes |
| `allowUnscopedSelectors` | `[]` | Selector prefixes that are global on purpose (a portal root, vendor CSS such as `.xterm`) |
| `strictTokens` | `true` | Unknown design token fails the build (`false` warns) |
| `lint` | `{}` | `false` skips the design-rule lint; `{ strict, disable, allow }` tunes it |
| `watch`, `minify` | `--watch` flag, `!watch` | Watch rebuilds unminified for readable traces |
| `plugins`, `extraExternal`, `define` | | Passed to esbuild |

After every build it checks each asset against the 8 MiB cap, runs
`assertScoped` on every sheet and `checkTokens` on everything; a non-watch
build then runs `lintWorkerUi` on the source. A failed check exits 1.

Six specifiers stay external because the console's import map serves them:
`react`, `react-dom`, `react-dom/client`, `react/jsx-runtime`,
`@iii-dev/console-ui`, `lucide-react`. The driver matches them **exactly**
(`workerUiExternalsPlugin`) because esbuild's `external` list would also
externalize `@iii-dev/console-ui/format` and `/hooks`, which must bundle.
Everything else bundles in. Only those six exist in the import map: a
dependency importing another bare react-family specifier (`react-dom/server`)
fails at `import()` time. A custom pipeline (`editor`, `canvas`) imports
`workerUiExternals`, `assertScoped`, `checkTokens` and, from
`@iii-dev/console-ui/lint-worker-ui`, `lintWorkerUi`/`formatLint`, and runs
the same checks; dropping the `react` external there bundles a second React
("Invalid hook call"), dropping `@iii-dev/console-ui` throws at once with the fix.

### Lint

`lintWorkerUi({ root, scope, strict, disable, allow })` scans `styles.css`,
`page.tsx` and `src/**` (never `dist/` or tests); errors fail the build,
warnings print. `strict: true` promotes warnings (`browser`, `iii-directory`,
`ide`; every migrated worker turns it on); `disable: ['rule']` drops a rule;
`allow: { rule: ['substring', /re/] }` ignores matching excerpts; a
`lint-allow <rule>` comment on the finding's line or the one above does the
same in place — always with a reason.

| Rule | Level | Flags |
|---|---|---|
| `no-window-dialogs` | error | `window.confirm/alert/prompt(` — use `useConfirm()`/`ConfirmDialog` |
| `icon-size` | error | Lucide `size`, `<svg width/height>` or `size-3`/`w-3 h-3` classes below the icon baseline |
| `accent-selection` | error | `var(--color-accent…)` in a selected/active/current rule (focus excepted) |
| `no-inline-svg` | warning | `<svg` in a `.tsx` outside `icons.tsx`/`icons/` — import from `lucide-react` |
| `radius` | warning | `border-radius` other than `0`, the system radius, full rounding, `var(--radius-*)`, `inherit` |
| `font-family` | warning | anything but `var(--font-…)`/`inherit` |
| `font-size` | warning | below the UI text floor |
| `case-transform` | warning | `text-transform`/`textTransform:` — use `Eyebrow`/`uiClasses.eyebrow` |
| `focus-stroke` | warning | `:focus`/`:focus-visible` outline, box-shadow or border in accent — use `--color-rule-focus` |
| `shadow` | warning | `box-shadow` that is not `var(--shadow-*)`, `none` or a token inset/hairline |
| `hex-color` | warning | `#hex`/`rgb()`/`hsl()` literals (custom properties on the scope root are fine) |
| `motion-literal` | warning | `transition`/`animation` with a literal `ms`/`s` duration |
| `keyframes-shared` | warning | `@keyframes …spin/pulse/shimmer/fade` — use `uiClasses.spin`/`uiClasses.pulse` |
| `viewport-media` | warning | `@media (max-width|min-width …)` — use `@container` |
| `tailwind-in-worker` | warning | `className` strings with several Tailwind utilities |

The baseline, floor and radius the rules check are the values in
`ade/design-system` › Numbers. CLI: `node
packages/console-ui/lint-worker-ui.mjs <worker>/ui [--strict] [--json]`;
`--all` from the repo root prints one row per worker.

## 4. Registration (the worker side)

One content function serving all of the worker's assets (dispatch on
`path`), one trigger per asset. Rust workers use the `iii-console-ui` crate:

```rust
use iii_console_ui::ConsoleUi;

ConsoleUi::new("mywork")
    .script("mywork/page.js", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js")))
    .style("mywork/styles.css", include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css")))
    .register(&iii);
```

This registers `<worker>::ui-content`, one Message-path trigger per asset,
and the `III_<WORKER>_UI_WATCH` watcher; it panics on a path the console
would reject. Export `ui` from the worker library and call `ui::register(&iii)`
after its normal functions; adapt `state/build.rs` so missing or stale UI
sources build before `include_str!`. Node workers register one function
mapping `{path}` to `{content, content_type?}`, then one Message-path
`console:script` or `console:style` trigger per asset with `config: {path}`.

**Always register through the SDK's Message path, never the engine's durable
`register_trigger`:** Message-path triggers are garbage-collected on
disconnect and replayed on reconnect.

## Runtime contract

| | |
|---|---|
| Trigger types | `console:script` (ESM JS), `console:style` (CSS); never register the tab-only `console:assets` type |
| Trigger config | `{ "path": string }`, nothing else |
| Path rules | lowercase `[a-z0-9._-]` segments, no leading slash, no `.`/`..` segments, ≤ 512 chars; extension must match the type (`.js` / `.css`); **convention: first segment = worker name** — it becomes the `data-iii-ui` scope and the only human-readable attribution |
| Content function | input `{ "path": string }` → output `{ "content": string, "content_type"?: string }` |
| Size cap | 8 MiB per asset — larger registrations are rejected (the driver fails first) |
| Reload | same path + changed content hash replaces the asset; unchanged content is a no-op |

## The dev loop (hot reload)

Rebuild-on-save stays in the build tool; re-registration stays in the worker
(new trigger first, then unregister the old handle). The Rust helper polls
once a second when `III_<WORKER>_UI_WATCH=1` (`ui/dist`) or names a directory:

```bash
pnpm --dir mywork/ui watch                      # terminal 1, repo root
cd mywork && III_MYWORK_UI_WATCH=1 cargo run    # terminal 2
```

Every open tab hot-swaps the asset: scripts re-`import()` + re-`setup()` (slot
React state is lost), styles link-swap with no flash; unchanged content is
hash-deduped end to end.

## Debugging

| Symptom | Cause |
|---|---|
| Build exits 1 naming a selector | an unscoped rule, unprefixed `@keyframes` or `@font-face` — `assertScoped` |
| Build exits 1 on `unknown token` | a `var(--color-…)` the console does not define — check `token-names.mjs`, or declare it on the scope root |
| Build exits 1 on `design-rule error(s)` | a lint error (or a warning under `strict`) — fix it or `lint-allow` it with a reason |
| Registration rejected with a path error | path violates the rules table (wrong extension, uppercase, `..`, …) |
| Registration rejected with a fetch error | the content function threw, returned no string `content`, or timed out |
| "Invalid hook call" in the tab | a second React in the bundle — a custom build dropped an external |
| `import()` fails on a bare specifier | a dependency imports a react-family subpath outside the six shared specifiers |
| Styles apply on the page but not in a custom portal | a custom `document.body` portal must carry `data-iii-ui="<worker>"` on its root |
| Whole console restyled | unscoped rules reached the console — check `warnings` in the manifest |
| Registered but absent | inspect `workers[].enabled` and `injectableUi.disabledWorkers` in the manifest |

Inspect `console::ui-manifest` (or `GET <console-host>:3113/ui`),
`/ui/<path>`, registered triggers, and `[iii-ui]` browser logs in that order.
The manifest is authoritative; its `warnings` must be empty.

## Testing

Validate all four layers; a green build alone is not enough.

1. **Static:** `pnpm --dir <worker>/ui build` — type-check, scoped and
   token-checked assets, lint clean (strict where enabled), no bundled
   React, editor or ANSI parser.
2. **Embedding:** accepted assets, an ESM export, the built CSS scope
   (esbuild may omit selector quotes); targeted Rust tests.
3. **Delivery:** boot engine + console + worker; manifest paths, hashes, no
   warnings, fetchable bytes, a changed hash after hot reload.
4. **Real rendering:** the actual console at a phone-sized, a narrow split
   and a wide pane (`ade/design-system` › Numbers), both split positions,
   both themes, keyboard only, reduced motion, long content, every async and
   live-update state, dirty navigation, reconnect — the matrix in
   `ade/design-console-ui` › Validate. For `host.triggerRenderers` add exact
   type match, malformed-config fallthrough, every slot and activity kind,
   non-interactive compact display, complete-detail lifecycle fidelity,
   action fallback, fail-closed redaction, and disable/disconnect fallback.

## Definition of done

- `PageShell` + `PageHeader` present, close action wired;
- every configurable page declares `configurationId`; every configuration
  entry has a purpose-built `host.configForms` form;
- controls, hooks, formatters and icons come from `@iii-dev/console-ui`, its
  `/hooks` and `/format` subpaths, and `lucide-react` — no local copies;
- the narrow flow exposes every action without horizontal overflow; focus is
  visible, controls have names, targets meet `ade/design-system` › Numbers;
- selection is neutral in both themes; styles are scoped and token-based;
- the build passes `strictTokens` and a clean lint (`strict` once migrated);
- async responses cannot overwrite a newer selection or a dirty draft;
- reconnect and hot reload leave no duplicate registrations; the manifest has
  no warnings and the browser console has no `[iii-ui]` errors.
