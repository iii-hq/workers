---
name: console-injectable-ui
description: Build, structure, and validate polished responsive worker UI (React pages, function-trigger renderers, configuration forms, and stylesheets) injected into the running iii console. Use when adding or changing a worker's console UI, especially when it must match the visual quality of database, console functions/triggers, iii-directory, and state; retain the shared header and visual system; work in narrow/mobile-sized panes; preserve state safely; hot reload; and ship without rebuilding the console.
---

# Injectable console UI

A worker can ship pages, renderers, forms, and stylesheets into every console tab
**at runtime**—no rebuild, no iframe, and hot reload. Treat `database`, console functions/triggers,
`iii-directory`, and `state` as proven patterns, not visual templates: reuse
their visual grammar and interaction mechanics while choosing the information
architecture that best fits the worker.

## How it works

A worker registers `console:script` and `console:style` triggers whose
`config.path` identifies an asset and whose `function_id` serves `{content}`
for `{path}`. The console hashes and serves those bytes, then pushes changes
to open tabs. Tabs `import()` scripts and call their default `setup(host)`;
styles load as scoped `<link>` assets. Re-registering a path hot-reloads it.
Registration is deployment; disconnect is teardown.

## Add the internal dependencies

This repository versions both sides of the contract together. Do not try to
install them from a public registry:

1. Add `<worker>/ui` to the root `pnpm-workspace.yaml`.
2. Add the compile-time UI surface to `<worker>/ui/package.json`:

   ```json
   {
     "name": "@iii-workers/mywork-ui",
     "private": true,
     "version": "0.0.0",
     "type": "module",
     "scripts": { "build": "tsc --noEmit && node build.mjs", "watch": "node build.mjs --watch" },
     "dependencies": { "@iii-dev/console-ui": "workspace:*" },
     "devDependencies": { "@types/react": "^19.2.14", "esbuild": "^0.25.0", "typescript": "^5.9.2" }
   }
   ```

3. For a Rust worker, link the worker-side registration helper in
   `<worker>/Cargo.toml`:

   ```toml
   iii-console-ui = { path = "../crates/console-ui" }
   ```

`@iii-dev/console-ui` is types-only at build time; the console serves its
runtime implementation from the active SPA. `iii-console-ui` registers the
content function, asset triggers, and development watcher. Node workers have
no worker-side helper and implement the wire contract directly.

## Project layout

```text
mywork/
  build.rs       # ensure dist assets exist before include_str!
  ui/
    page.tsx      # the script asset — default-exports setup(host)
    styles.css    # the style asset — every rule scoped
    build.mjs     # esbuild, five external specifiers
    package.json  # workspace dependency on @iii-dev/console-ui
    tsconfig.json
    src/           # page, renderer, config form, hooks, widgets
  src/
    ui.rs          # embed and register dist/page.js + dist/styles.css
```

Start from `state/ui/tsconfig.json`, `state/build.rs`, and `state/src/ui.rs`
for the mechanical files; rename worker/asset paths and keep their tests.
For UI structure, consult the references below before writing code.

## Authoring workflow

1. Read `packages/console-ui/index.d.ts`; never guess a component or prop.
2. Select only the needed slots, then model the primary object, navigation,
   actions, async states, and state that must survive navigation or reload.
3. Design wide and narrow flows deliberately; do not squeeze desktop UI.
4. Build with shared primitives and minimal scoped CSS, then type-check,
   register, inspect the manifest, and exercise the real console.

### Living references

| Need | Read | Reuse |
|---|---|---|
| Public API | `packages/console-ui/index.d.ts` | Exact exports and props |
| Shared page chrome | `console/web/src/components/ui/PageChrome.tsx` | `PageShell`, `PageHeader`, surface roles |
| Catalog/detail | `console/ui/src/catalog/widgets.tsx`, `console/ui/src/catalog/FunctionsPage.tsx`, `console/ui/src/catalog/TriggersPage.tsx`, `console/ui/styles.css` | Grouped rows, persistent hero, identity masthead, facts, tabs, contextual rail |
| Data workbench | `database/ui/src/page/index.tsx`, `database/ui/src/page/TableDataPanel.tsx`, `database/ui/styles.css` | Mode bar, schema tree, toolbars, data grid, inspector, nested container responses |
| List/detail editor | `iii-directory/ui/page.tsx`, `iii-directory/ui/src/page/browser.tsx`, `iii-directory/ui/styles.css` | `setup(host)`, container-width drill-in, dirty-draft guards, per-tab state |
| Multi-level browser | `state/ui/page.tsx`, `state/ui/src/page/browser.tsx`, `state/ui/styles.css` | One-pane-at-a-time narrow flow, live state updates, stale-request guards |
| Rust delivery | `state/src/ui.rs`, `state/build.rs` | Embedding, registration, asset tests, build freshness |

Copy delivery plumbing when it matches. Do **not** copy a reference page's
sidebar count, breakpoints, controls, or visual hierarchy without deriving
them from the new worker's content.

## Visual quality is part of correctness

Choose one dominant archetype before writing JSX. Mixing all four produces a
generic dashboard with too many panels.

| Archetype | Use for | Required shape |
|---|---|---|
| Console catalog | Many searchable objects with rich detail | Grouped list → persistent hero or breadcrumb + identity masthead + tabs; add a contextual rail only for genuinely related information |
| Database workbench | Several tools operating on one selected resource | Compact mode switcher, collapsible resource tree, one active work surface, local toolbar/status bar, optional inspector |
| Directory editor | Searchable documents with drafts or preview | List → document identity → edit/preview modes; keep draft state mounted and put save status beside the work |
| State explorer | Deep but compact hierarchy | Progressive columns on wide panes and one-level-at-a-time drill-in on narrow panes |

### Apply the shared visual grammar

- Build hierarchy with surfaces, not boxes: sidebar, panel, raised toolbar,
  hover/selected wash. Reserve 1 px edges for structural or tabular
  separation; avoid borders, shadows, or a card around every section.
- Use a restrained scale: 4/6/8 px for internal gaps, 12/14/20/24 px for
  section spacing, and the system 6 px radius. Oversized padding makes these
  dense operator tools look like marketing pages.
- Set document/hero titles around 17–18 px at weight 600; body copy around
  12.5–13 px with 1.55–1.65 line height and a 60–72ch measure; metadata around
  10–11.5 px. Author interface copy in natural sentence/title case; never use
  CSS `lowercase` or `uppercase` transforms on tabs, buttons, menus, or forms.
- Use sans for all interface chrome, labels, actions, explanations, and prose.
  Reserve mono for machine-produced ids, paths, schemas, values, payloads,
  code, and tabular data. Never make a whole panel or its controls mono.
- Repeat one restrained identity glyph in the list row, empty hero, and
  detail masthead, as console functions/triggers do. Use Lucide icons at the
  shared 16 px baseline; do not add application icon usages, component
  defaults, or root SVGs below 16 px, emoji, or a new icon dependency.
- Make list rows full-width targets with one strong primary line and at most
  one or two quieter supporting lines. Indicate selection with a surface wash,
  stronger ink, and an optional 2 px neutral edge—never accent color alone.
- Keep page actions in `PageHeader`; put resource actions in the identity
  masthead and work actions in the nearest toolbar. Show one clear primary
  action at the point of work; move rare actions into a menu.
- Use shared line `Tabs` for peer views of the same object. They have a bottom
  rule, neutral active underline, 600 weight, natural casing, and a semantic
  16 px icon by default. `SegmentedControl variant="tabs"` uses the same line
  recipe; reserve `variant="radio"` and its surface track for persisted
  mutually exclusive choices. Do not fork private boxed tab CSS.
- Use compact fact sheets or stat tiles only for useful comparisons. Prefer a
  quiet `--color-surface` group with label/value rows over a grid of large
  KPI cards.
- Put loading, empty, error, and success states where content will appear so
  the page silhouette stays stable. Use `Skeleton`, `EmptyState`, and
  `StatusPanel`; never present raw error text as the main design.
- For simple tables, compose the shared `TableViewport`/`TableFrame`/`Table`
  family. Use natural-case sans headers, horizontal row dividers, comfortable
  page density or compact chat density, and mono only for technical cells.
  Make only interactive rows hoverable. Long data grids may add sticky headers,
  aligned tabular numbers, selection, and an inspector or context rail.

### Reject generic generated UI

Do not ship card soup, gradients, glows, ornamental shadows, giant centered
headings, excessive badges, random accent colors, repeated descriptions, or
an empty canvas with controls floating in corners. Do not give navigation,
metadata, and the primary task equal visual weight. Compare the result beside
the closest reference at the same width in both themes; its structure may
differ, but density, typography, surface hierarchy, and control treatment
must feel native to the same console.

## 1. The script asset (`ui/page.tsx`)

Ordinary React. Import from `react` and `@iii-dev/console-ui` — both resolve
at runtime through the console's import map, so they must stay **external**
in your build. Default-export a `setup(host)` function and make every
registration through `host` (the loader attributes registrations to your
script so it can dispose them on reload):

```tsx
import {
  type Host,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
} from '@iii-dev/console-ui'

function MyworkPage({
  host,
  onRequestClose,
}: PageRenderProps & { host: Host }) {
  return (
    <PageShell className="mywork-ui-shell">
      <PageHeader
        icon={<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" aria-hidden><circle cx="8" cy="8" r="5" /></svg>}
        title="Mywork"
        description={host.path}
        onClose={onRequestClose}
      />
      <PageMain className="mywork-ui-main">
        {/* Compose the chosen archetype here. */}
      </PageMain>
    </PageShell>
  )
}

export default function setup(host: Host) {
  host.pages.register({
    id: 'mywork-manager',           // page URL: #/ext/mywork-manager
    title: 'Mywork',                // nav label
    render: (props) => <MyworkPage host={host} {...props} />,
  })

  // Register other slots only when their implementations exist.
  // host.functionTriggers.register(createMyTriggerRenderer(host))
  // host.configForms.register('mywork', MyConfigForm)
  // host.providerConfigForms?.register('my-provider', MyProviderConfigForm)
}
```

This is a delivery skeleton, not a finished design. Compose one archetype in
its body before evaluating the UI. Imports from the shared package add zero
bundle bytes because they resolve to the running console's React tree.

### The shared component library

The package exports page chrome; `List`/`ListItem`, `Card`, `Panel`, `Chip`,
`IconButton`, and semantic `Table` parts; line `Tabs` and `SegmentedControl`;
`Selector` and `Select`; buttons, inputs, dialogs, menus and tooltips;
status/empty/loading components; Markdown and JSON renderers; the terminal atoms (`AnsiText`,
`TerminalStream`, `TerminalCommandLine`); `CodeEditor`, `FileDiff`, and
`WorkerConfigurationDialog`. It also exports the stable `uiClasses` recipes
and canonical `tokens` inventory. Read `packages/console-ui/index.d.ts` for
the authoritative names and props.

Use `Selector` for searchable single-choice input, including grouped or
disabled options, async caller-owned filtering, loading/empty/error states,
validation, and explicitly enabled free-form creation. Use `Select` for a
small finite non-searchable list. Use the shared `Tooltip` parts, or
`IconButton` for an icon-only action; do not implement independent hover
timers, geometry, or portals. Keep a local selector only for a genuinely
different interaction such as hierarchical drill-in, multi-select, or a
persistent command palette, and document that exception.

Use `TabsList variant="line"`/`TabsTrigger` or `SegmentedControl
variant="tabs"` for content navigation. Shared tabs add a semantic icon by
default; pass an explicit icon only when the default does not express the
view, or `icon={false}` only when there is a documented space constraint.
Use `IconButton` for icon-only actions such as Refresh or Configure so the
16 px glyph retains an accessible name and tooltip.

Selection is always neutral in both themes: `--color-surface-selected`,
`--color-ink`, and optionally `--color-edge`. Reserve `--color-accent` for a
primary action, form focus, live activity, or semantic domain data.

**`PageShell` and `PageHeader` are the stable outer contract for full
pages.** They keep identity, height behavior, close affordance, and header
styling consistent with the console. Use `PageBody`, `PageSidebar`, and
`PageMain` when their navigation/workspace model fits; replace the body with
a custom structure when the domain needs columns, a canvas, or a drill-in
flow. Do not replace the outer shell and header.

Use `PageSidebar`'s declarative `collapsible`, `resizable`, `storageKey`,
width bounds, `side`, `narrow`, and `narrowBelow` props instead of shipping
local collapse DOM, drag handlers, width clamps, persistence, focus logic, or
transitions. The Console host keeps a single stable `aside`, leaves children
mounted while collapsed, synchronizes instances sharing a storage key, and
owns motion plus reduced-motion behavior. Pass `narrow` when the page already
has a drill-in state; use `narrowBelow` when only shared sidebar chrome needs
to react to its parent width. Neither responsive mode overwrites the saved
wide preference.

The pieces own the surface hierarchy (header on `--color-panel-raised`
with a hairline `--color-edge` border, sidebar on `--color-sidebar`, main
on `--color-panel`) — don't repaint those tokens yourself. No sidebar?
Put content straight into `PageMain`. Keep `onRequestClose` wired to
`PageHeader.onClose`. Keep header actions few and essential; at narrow widths,
move secondary actions into a `DropdownMenu` rather than allowing the header
to wrap or overflow.

### Responsive structure: pane width, not viewport width

An injected page may occupy a full tab, half of a split tab, or a narrow
mobile viewport. A viewport media query cannot distinguish those cases.
Observe the page body's own width (or use CSS container queries for purely
visual changes) and switch the interaction model at the width where the
content actually stops working.

When React must mount different narrow views, reuse the callback-ref
`useContainerNarrow` implementation from `database`, `iii-directory`, or
`state`: measure synchronously, observe with `ResizeObserver`, disconnect on
ref changes, and ignore zero-width hidden panes.

Apply these rules:

- Derive the threshold from the content's minimum usable width; do not copy
  `800` or `850` merely because a reference uses it.
- Prefer a drill-in sequence on narrow panes: list → detail, or scope → key
  → value. Render one primary pane at a time and provide a visible,
  labelled back control.
- Remove modes that require width. For example, collapse split edit/preview
  to one mode at a time.
- Make narrow interactive rows at least 44 px tall. Keep labels truncated or
  wrapped deliberately; never let the whole page scroll horizontally.
- Put `min-width: 0` and `min-height: 0` on nested flex/grid panes. Give only
  the content region that needs it `overflow: auto`.
- Use `panelSide` to mirror side navigation in a wide right-hand pane. Do not
  mirror reading order or a single-pane narrow flow.
- Key persisted UI state with `tabId`; treat `localStorage` as best-effort.
  Guard dirty drafts before navigation and ignore stale async responses after
  the selection changes.
- Keep editors mounted when hiding a preview/editor mode if cursor and scroll
  continuity matter. Unmount when state must reset between domain objects.
- Test keyboard focus, back navigation, reduced motion, and touch targets in
  addition to visual width.

Use the shared Monaco-backed `CodeEditor` for code or long text and
`FileDiff` for diffs. Put the editor inside an `overflow-auto` pane. Never
bundle Monaco, CodeMirror, or another editor/diff renderer into the asset.

Terminal-shaped cards (exec output, code runs, build logs) compose the shared
terminal atoms under the same rule: `TerminalCommandLine` for the `$ command`
header, `TerminalStream` for the labeled stdout/stderr pane (set `ansi` to
color it; `tone="err"` for stderr), and `AnsiText` for ANSI SGR text mapped
onto the design tokens. Never bundle an ANSI parser or carry private
terminal-rendering copies.

## 2. The style asset (`ui/styles.css`)

Plain CSS, **every rule scoped under your worker's wrapper attribute**:

```css
[data-iii-ui="mywork"] .mywork-ui-main {
  min-width: 0;
  min-height: 0;
  overflow: auto;
}
[data-iii-ui="mywork"] .mywork-ui-browser.narrow .mywork-ui-row {
  min-height: 44px;
}
@keyframes mywork-flash { /* prefix keyframes names — they are global */ }
```

The console mounts every injected render inside
`<div data-iii-ui="<first path segment>" style="display:contents">`, so
scoped rules apply to your UI and nothing else. Use the console's design
tokens. The main roles are:

Use `--color-bg/sidebar/panel/panel-raised/surface*` for hierarchy,
`--color-ink/ink-faint/ink-ghost` for text, `--color-alert/warn/ok` and their
muted variants for status, `--color-edge/rule-focus` for structure, and
`--font-sans`/`--font-mono`/`--font-code` by semantic role. Accent is not a
selected-state token.

Dark mode is a variable flip, so token-based styles theme for free. Prefer
shared components for controls, keep all UI chrome and prose in `--font-sans`,
and reserve `--font-mono` for identifiers, paths, values, payloads, and data.
Use `--font-code` only for source code, structured payloads, and editor text.
Never hardcode theme colors.

What must NOT be in the sheet: unscoped selectors (`:root`, `html`, `body`,
`*`, bare element names) and `@font-face` — injected CSS is unlayered, so an
unscoped rule silently beats the console's fully-layered CSS document-wide.
The console lints every style on fetch (warn-only) and reports findings in
the manifest's `warnings` array; keep it empty.

Do not use Tailwind utility classes in injected markup: the worker's class
names are not part of the console's compiled Tailwind output. Use the named
shared components and `uiClasses` recipes; add scoped worker CSS only for
domain-specific layout and data visualization. Use `--motion-duration-*` and
`--motion-ease-*` (or the shared motion recipe classes) for state changes.
Streaming text, rapidly updating meters, and cursor-following geometry update
without transitions. Scope custom selectors inside
`@media (prefers-reduced-motion: reduce)` too; keyframe names remain global
and must carry the worker prefix. Shared components and recipes already honor
the Console's global reduced-motion contract.

Shared `Dialog`, `DropdownMenu`, `Select`, `Selector`, `Tooltip`, and
`BottomSheet` portals preserve the worker's `data-iii-ui` scope
automatically. If custom domain UI portals directly to `document.body`, wrap
its portal root with `data-iii-ui="<worker>"`.

## 3. The build (`ui/build.mjs`)

esbuild with the five shared specifiers external:

```js
import esbuild from 'esbuild'

const options = {
  entryPoints: ['page.tsx', 'styles.css'],
  bundle: true,
  format: 'esm',
  jsx: 'automatic',
  outdir: 'dist',
  external: ['react', 'react-dom', 'react-dom/client',
             'react/jsx-runtime', '@iii-dev/console-ui'],
  logLevel: 'info',
}

if (process.argv.includes('--watch')) {
  const context = await esbuild.context(options)
  await context.watch()
} else {
  await esbuild.build(options)
}
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

This registers `<worker>::ui-content`, one Message-path trigger per asset,
and the `III_<WORKER>_UI_WATCH` watcher. It rejects invalid paths early.
Export `ui` from the worker library and call `ui::register(&iii)` after its
normal functions. Adapt `state/build.rs` so missing/stale UI sources build
before `include_str!`; preserve unrelated duties of an existing build script.

**Always register triggers through your SDK's Message path, never through
the engine's durable `register_trigger` function.** Message-path triggers are
garbage-collected on disconnect and replayed on reconnect.

Node workers implement the same contract directly: register one function
that maps `{path}` to `{content, content_type?}`, then one Message-path
`console:script` or `console:style` trigger per asset with
`config: {path}`.

## Runtime contract and slots

| | |
|---|---|
| Trigger types | `console:script` (ESM JS), `console:style` (CSS); never register the tab-only `console:assets` type |
| Trigger config | `{ "path": string }`, nothing else |
| Path rules | lowercase `[a-z0-9._-]` segments, no leading slash, no `.`/`..` segments, ≤ 512 chars; extension must match the type (`.js` / `.css`); **convention: first segment = your worker name** — it becomes the `data-iii-ui` scope and the only human-readable attribution |
| Content function | input `{ "path": string }` → output `{ "content": string, "content_type"?: string }` (`content_type` defaults from the asset kind) |
| Size cap | 8 MiB per asset — registrations over it are rejected |
| Reload | same path + changed content hash replaces the asset; unchanged content is a no-op |

All registration goes through the per-script `host`; every entry is disposed
automatically on hot reload and worker disconnect. Each `register` also
returns a remover for manual teardown.

`host.pages.register({id, title, render})` creates `#/ext/<id>` and adds it to
the nav. Its `render` receives:

- `panelSide`: `'left' | 'right'` — which side of the workspace tab the
  pane occupies; use it only to keep wide side navigation on the outer edge;
- `tabId`: the hosting workspace tab's stable id (tabs persist across
  reloads); key per-tab UI state on it;
- `onRequestClose`: close the pane hosting your page (a split drops the
  column; a single pane detaches); wire it to `PageHeader.onClose`;
- `workingDir`: the active conversation's live working directory, or
  `null`/absent; use only for filesystem-shaped pages and react to changes.

| Surface | What it is |
|---|---|
| `host.functionTriggers` | Custom chat/trace renderers. Match only the worker's function ids and return `null` to fall through. `message.description` is the harness's short activity label. Set renderer `metadata: { display: true }` only for successful rich artifacts that should remain visible while raw details are collapsed; the hint applies to the renderer that returned the winning node. If raw data contains secrets, implement a pure, total, cycle-safe `redactRaw`; the raw tab and copy action otherwise expose the original input/output. |
| `host.configForms` | Replace one configuration form. Render fields and call `onChange`; the host retains dirty tracking, validation, save, and reset. Honor `focusField`. Pass `{ layout: 'full' }` as the third `register` argument only when the form is a workbench that owns its internal scrolling; the default `contained` layout keeps the centered host column. |
| `host.providerConfigForms?` | Replace the form body for one exact `llm-router` provider id inside the chat model picker. Use it for provider-owned OAuth, device flow, or companion-app login. The host retains the provider slice, schema validation, dirty guard, save/reset, and model refresh; the component receives `{ providerId, schema, value, onChange, errors, configured, available, modelCount }`. Feature-detect for older consoles. Never solicit plaintext API keys here—direct operators to the provider's declared environment variable. |
| `host.chat?` | Optional session-chip slot. Feature-detect it for older consoles. |
| `host.iii` | The tab's bus client: `trigger(functionId, payload?, {timeoutMs?})`, `on(functionId, handler)` (returns un-listen), `registerTrigger({type, function_id, config})` (returns un-register), `addConnectionStateListener`, `browserId`. Injected UI *acts* by invoking its own worker's functions. |
| `host.components` / `host.path` | Runtime component record and the current script asset path. |

Live data pattern: a page can register its *own* trigger over `host.iii`
with a handler id like `iii::<worker>-ui::events::<browserId>` (the `iii::`
prefix keeps per-event invocations out of the trace feed). The binding is
GC'd with the tab.

Every injected render is error-bounded; import or setup failures remove the
extension contribution and appear in the browser console instead of breaking
the entire console. Scripts still run with full console-origin privileges;
the wrapper scopes styles but is not a security sandbox.

## The dev loop (hot reload)

Rebuild-on-save stays in the build tool; re-registration stays in the worker.
Serve the new bytes, register a fresh trigger for the same path, then
unregister the old handle. Register-first avoids a flash; unregistering keeps
the SDK replay map bounded. The Rust helper does this with a one-second
poller. Set `III_<WORKER>_UI_WATCH=1` for `ui/dist`:

```bash
# terminal 1, from the workers repo root
pnpm --dir mywork/ui watch

# terminal 2; the default watch path is relative to the worker cwd
cd mywork && III_MYWORK_UI_WATCH=1 cargo run
```

Every open tab hot-swaps the asset in place — scripts re-`import()` +
re-`setup()` (React state in your slots is lost — dispose + remount), styles
link-swap with no flash. Unchanged content is hash-deduped end to end.

## Debugging

| Symptom | Cause |
|---|---|
| Registration rejected with a path error | path violates the rules table (wrong extension, uppercase, `..`, …) |
| Registration rejected with a fetch error | your content function threw, returned no string `content`, or timed out |
| "Invalid hook call" in the tab | your bundle contains a second React — a missing `external` |
| `import()` fails on a bare specifier | a dependency imports a react-family subpath outside the five shared specifiers |
| Styles apply on your page but not in a custom portal | Shared portalled components preserve scope automatically; a custom `document.body` portal must carry `data-iii-ui="<worker>"` on its root |
| Whole console restyled | your sheet has unscoped rules — check `warnings` in the manifest |
| Registered but absent | inspect `workers[].enabled` and `injectableUi.disabledWorkers` in the manifest |

Inspect `console::ui-manifest` (or `GET <console-host>:3113/ui`),
`/ui/<path>`, registered triggers, and `[iii-ui]` browser logs in that order.
The manifest is authoritative; its `warnings` must be empty.

## Testing your worker's UI

Validate all four layers; a successful esbuild run alone is not enough.

1. **Static:** run `pnpm --dir <worker>/ui build`; require type-check success,
   non-empty assets, and no bundled React/editor copy.
2. **Embedding:** test accepted assets, an ESM export, and the built worker CSS
   scope (esbuild may omit selector quotes); run targeted Rust tests.
3. **Delivery:** boot engine + console + worker; require manifest paths,
   hashes, no warnings, fetchable bytes, and a changed hash after hot reload.
4. **Real rendering:** exercise the actual console, not only an isolated
   component harness. Cover at least a phone-sized pane (~320–430 px), a
   narrow split pane, and a wide pane; left and right split positions; light
   and dark themes; keyboard-only navigation; reduced motion; long names and
   payloads; loading, empty, error, success, and live-update states; dirty
   navigation; and worker reconnect.

The UI is done only when:

- `PageShell` + `PageHeader` are present and the close action works;
- the narrow flow exposes every action without horizontal page overflow;
- focus is visible, controls have names, and narrow targets are at least 44 px;
- selected rows, cards, tabs, chips, and segments remain neutral in both themes;
- content tabs use the shared line recipe, natural casing, and default 16 px
  icons; application icons are never authored below 16 px;
- human-facing chrome is sans; mono is limited to machine-readable content;
- transitions use the shared motion vocabulary and reduced motion is immediate;
- async responses cannot overwrite a newer selection or a dirty draft;
- all styles are scoped and token-based in both themes;
- disconnect/reconnect and hot reload leave no duplicate registrations;
- the manifest has no warnings and the browser console has no `[iii-ui]` errors.
