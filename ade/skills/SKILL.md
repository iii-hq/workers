---
name: ade
description: >-
  The iii web console — chat, trace explorer, worker catalog, and the runtime
  host every worker injects its own pages, renderers, and configuration forms
  into. Read it before building or changing any console UI, and when an agent
  must drive the operator's workspace (open or close a screen, propose a
  working directory).
---

# console

The console is a single Rust binary that serves the React SPA and proxies the
engine WebSocket on one port (`http_port`, 3113 by default). It renders the
chat (on the `harness` turn loop), the OpenTelemetry trace explorer, the worker
catalog, and a tabbed workspace of floating panels the operator arranges. It is
also the delivery host for **injectable UI**: any worker registers a
`console:script` / `console:style` trigger whose `config.path` names an asset
and whose `function_id` serves the bytes, and every open tab imports the module,
calls its `setup(host)`, and hot-reloads it on re-registration. No console
rebuild, no iframe.

Reach for this worker in two situations. First, when a worker needs a face in
the console — a page, a function-call or trigger-activity renderer, a settings
form — build it against the three companion skills below rather than inventing
markup. Second, when an agent should change what the operator is looking at:
open a worker page beside the conversation, pin a chat, close a screen, or
propose that the session follow a project created elsewhere.

Prerequisites: the `configuration` worker (the console stores its own settings
and every worker's configuration entry there). Chat needs `harness` and its
stack; traces need the engine's OpenTelemetry export.

## When to Use

- A worker must ship a console page, renderer, or configuration form — read
  `console/injectable-ui` first, then `console/design-console-ui`, then the
  design system in `console/design-system`.
- You changed a worker's UI and must prove it is loadable: read
  `console::ui-manifest` and require an empty `warnings` array.
- The user should watch something in the console: `console::workspace::open`
  with `ext:<page>`, `workers`, `traces`, or a pinned `chat`.
- You created or cloned a project in another directory and the user asked to
  continue there: `console::working-directory::propose` (never just to inspect
  a file).
- You need to know what the operator currently sees: `console::workspace::list`.

## Boundaries

- The console renders; it does not own domain data. Injected UI acts by
  calling its own worker's functions over `host.iii`, and configuration values
  live in the `configuration` worker.
- `console:assets` is the tab-side live-update subscription. Workers never
  register it; register `console:script` / `console:style` only.
- Injected styles are scoped under `[data-iii-ui="<worker>"]`; an unscoped
  rule restyles the whole console and is reported as a manifest warning.
- Register asset triggers through the SDK's Message path so they are
  garbage-collected on disconnect — not through the durable
  `engine::register_trigger`.
- `workspace::open` reuses a tab that already shows the screen; it never
  duplicates panels. Selection of the active tab is per browser tab and only
  follows a function-driven activation.
- Native console UI (`ade/web`) and worker UI change in separate pull
  requests; the shared component surface is `@iii-dev/console-ui`
  (`packages/console-ui`) and its `index.d.ts` is the only API contract.

## Functions

- `console::status` — runtime knobs: `http_port`, `engine_url`, `version`; use for liveness and readiness.
- `console::ui-manifest` — every injected asset currently loadable, with path, kind, content hash, and style-lint warnings; the authoritative check after registering UI.
- `console::workspace::list` — the operator's workspace: tabs, columns, screens, and the active tab.
- `console::workspace::open` — show a screen next to the conversation (`ext:<page>`, `workers`, `traces`, or a pinned `chat` by `session_id`).
- `console::workspace::close` — remove a screen wherever it is shown; idempotent.
- `console::working-directory::propose` — ask the operator to move the session (chat and paired shell) to a directory created or cloned elsewhere.
- `console::ui-content` — the console's own content function for its injected catalog pages; internal.
- `console::on-config-change`, `console::working-directory::inject-guidance`, `console::working-directory::stamp-session` — internal wiring; never call directly.

## Reactive triggers

The console owns three trigger types. Two are the injectable-UI contract a
worker binds to ship UI; the third is tab-internal.

- `console:script` — an ESM JavaScript asset. `config: { path }` is the
  identity (`<worker>/page.js`); the trigger's `function_id` is the worker's
  content function returning `{ content, content_type? }` for `{ path }`.
- `console:style` — a CSS asset with the same contract and a `.css` path.
- `console:assets` — a tab's live-update subscription; the console registers
  it itself.

Bind the first two once per asset at worker startup, after the content
function is registered. Re-registering the same path with different bytes
hot-swaps the asset in every open tab; identical bytes are a no-op. Rust
workers use the `iii-console-ui` crate (`ConsoleUi::new("<worker>").script(..).style(..).register(&iii)`),
which also wires the `III_<WORKER>_UI_WATCH` development watcher; Node workers
register the content function and the two triggers directly.

## Skills shipped with this worker

- `console/injectable-ui` — the authoring contract for worker UI: project
  layout, `setup(host)` slots, the shared component library, scoped CSS,
  esbuild externals, Rust and Node registration, hot reload, debugging, and the
  definition of done.
- `console/design-console-ui` — responsive Console UX: pane-width (not
  viewport) breakpoints, phone drill-in flows, touch and keyboard
  accessibility, configuration and provider forms, state integrity, and the
  validation checklist.
- `console/design-system` — the iii Schematic design system: tokens, surface
  ramp, typography, radius, elevation, motion, and the canonical components.
