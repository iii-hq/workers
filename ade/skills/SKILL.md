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
engine WebSocket on one port (`http_port`, 3113 by default; `console::status`
returns the actual `url`). It renders the
chat (on the `harness` turn loop), the OpenTelemetry trace explorer, the worker
catalog, and a tabbed workspace of floating panels the operator arranges. It is
also the delivery host for **injectable UI**: any worker observes
`console::ui::asset` with args `{ kind: 'script' | 'style', path }` naming an
asset and its content function as the target that serves the bytes, and every
open tab imports the module, calls its `setup(host)`, and hot-reloads it when
the asset changes. No console rebuild, no iframe.

Reach for this worker in two situations. First, when a worker needs a face in
the console — a page, a function-call or observer-activity renderer, a settings
form — build it against the three companion skills below rather than inventing
markup. Second, when an agent should change what the operator is looking at:
open a worker page beside the conversation, pin a chat, close a screen, or
propose that the session follow a project created elsewhere.

Prerequisites: the `configuration` worker (the console stores its own settings
and every worker's configuration entry there). Chat needs `harness` and its
stack; traces need the engine's OpenTelemetry export.

## When to Use

- A worker must ship a console page, renderer, or configuration form — read
  `ade/injectable-ui` (the delivery contract) first, then
  `ade/design-console-ui` (responsive UX and forms), with
  `ade/design-system` open for every visual rule and number.
- A worker's records should be mentionable in chat (`@calendar(id="…")`:
  composer autocomplete, pills with a hover preview, agents receiving the
  record resolved): read `ade/mentions` — two functions and a metadata
  descriptor, no console change.
- You changed a worker's UI and must prove it is loadable: read
  `console::ui-manifest` and require an empty `warnings` array.
- The user should watch something in the console: `console::workspace::open`
  with `ext:<page>`, `workers`, `traces`, or a pinned `chat`.
- You created or cloned a project in another directory and the user asked to
  continue there: `console::working-directory::propose` (never just to inspect
  a file).
- You need to know what the operator currently sees: `console::workspace::list`.

## Chat-owned Context tracking

Users can ask in chat to pin release progress, a goal, a status summary or a checklist.
Use `console::context::list`, `console::context::save` and `console::context::remove`.
Discover the contracts before calling; Harness stamps the current `session_id`.
A save stores a structured snapshot, not executable code, an extension or a monitor.
Default to `conversation`; use `project` only when the user explicitly asks. Project
identity is the session's canonical `metadata.fs_scope.root`; other conversations
using that directory see the same project snapshots. List first and use an existing
id to edit or move scope. Save replaces the whole snapshot: preserve the fields not
being edited. Include a source label/HTTP(S) URL when available, and read the source
before saving numeric progress. Update relevant snapshots when consulting their source
or changing the tracked work. Do not poll or fetch unrelated sources. If refresh fails,
keep the previous snapshot and tell the user it is stale. Confirm the saved scope.
Users may edit, change scope, refresh or remove through chat. The Context panel's
Tracking section shows the saved progress, checklist, source and last-updated time;
its Update/Remove actions prepare a chat draft, never send or mutate silently.
Snapshots persist in `<data_dir>/context-tracking.json`, with atomic writes.

## Boundaries

- The console renders; it does not own domain data. Injected UI acts by
  calling its own worker's functions over `host.iii`, and configuration values
  live in the `configuration` worker.
- `console::ui::on-assets-changed` is the tab-side live-update feed. Workers
  never observe it; they observe `console::ui::asset` only.
- Injected styles are scoped under `[data-iii-ui="<worker>"]`; an unscoped
  rule restyles the whole console and is reported as a manifest warning.
- Observe `console::ui::asset` over the worker's own connection: its
  observers leave with the worker, and the console drops the asset.
- `workspace::open` reuses a tab that already shows the screen; it never
  duplicates panels. Selection of the active tab is per browser tab and only
  follows a function-driven activation, which every activating `open` stamps.
  A page that asks for a panel of its own passes `relative_to` with its own
  screen id, so the panel lands beside the page, in whichever tab shows it,
  instead of beside the chat.
- An injected page placing a screen for the operator in front of it calls
  `host.panels.openScreen({ screen, relativeTo?, direction?, sizes? })`
  (built-in screens like `traces` included): local and optimistic, no bus
  round trip. Feature-detect it and fall back to `console::workspace::open`.
- Native console UI (`ade/web`) and worker UI change in separate pull
  requests; the shared component surface is `@iii-dev/console-ui`
  (`packages/console-ui`) and its `index.d.ts` is the only API contract.

## Functions

- `console::status` — runtime knobs: `url`, `http_port`, `engine_url`, `version`; use for liveness and readiness, and open ADE pages on this machine (browser sessions, curl) at `url` (`<url>/#/worker/<scope>/<page-id>`) instead of assuming port 3113. `url` is loopback when the console listens on every interface: to show the operator a page, use `console::workspace::open`, not a pasted link.
- `console::ui-manifest` — every injected asset currently loadable, with path, kind, content hash, and style-lint warnings; the authoritative check after registering UI.
- `console::workspace::list` — the operator's workspace: tabs, columns, screens, column widths, and the active tab.
- `console::workspace::open` — show a screen next to the conversation (`ext:<page>`, `workers`, `traces`, or a pinned `chat` by `session_id`). It lands right of the chat panel; `relative_to` names another mounted screen to sit beside and `direction` (`right`/`left`) picks the side. `sizes` sets the tab's column widths in the same write — read `workspace::list` first for the widths that are up.
- `console::workspace::close` — remove a screen wherever it is shown; idempotent.
- `console::working-directory::propose` — ask the operator to move the session (chat and paired shell) to a directory created or cloned elsewhere.
- `console::ui-content` — the console's own content function for its injected catalog pages; internal.
- `console::on-config-change`, `console::working-directory::inject-guidance`, `console::working-directory::stamp-session` — internal wiring; never call directly.
- `console::mentions::trace::search`, `console::mentions::trace::get` — the `@trace(id="<trace_id>")` chat mention provider over `engine::traces::list`; internal. Agents read a mentioned trace through `engine::traces::tree` (see `ade/mentions`).

## Observable functions

The console hosts four private functions others observe. One is the
injectable-UI contract a worker observes to ship UI; one is tab-internal; two
report changes.

- `console::ui::asset` — one asset per observer. Args
  `{ kind: 'script' | 'style' | 'module', path }`; `path` is the identity
  (`<worker>/page.js`, at most 512 bytes) and the observer's target is the
  worker's content function, returning `{ content, content_type? }` for
  `{ path }`. Once the observer is active the console calls the content
  function through it. A later observer of the same path supersedes the
  earlier one (`UI_ASSET_SUPERSEDED`); a content fetch that fails rejects the
  observer (`UI_ASSET_FETCH_FAILED`). The function itself is never called.
- `console::ui::on-assets-changed` — a tab's live-update feed, no args. Each
  new observer receives `{ op: 'sync', assets }` through itself; every later
  change is `{ op: 'set', asset }` or `{ op: 'delete', path }`. The console's
  tabs observe it themselves.
- `console::workspace::on-changed` — called after every workspace layout
  write (`open`, `close`, a browser's own edit), and once through each new
  observer when it is accepted. No args, empty event: a page that watches the
  layout observes it and re-reads `console::workspace::list`.
- `console::compose::on-changed` — called when the Compose daemon writes
  `state.json` or the compose file changes. No args; filter on `input.kind`,
  `input.file` or `input.namespace`.

Observe `console::ui::asset` once per asset at worker startup, after the
content function is registered. To change an asset, observe it again and
dispose the old observer: different bytes hot-swap the asset in every open
tab; identical bytes are a no-op. Rust workers use the `iii-console-ui` crate
(`ConsoleUi::new("<worker>").script(..).style(..).register(&iii)`), which also
wires the `III_<WORKER>_UI_WATCH` development watcher; Node workers register
the content function and observe `console::ui::asset` directly:

```ts
iii.observe(
  { function_id: 'console::ui::asset', args: { kind: 'script', path: 'my-worker/page.js' } },
  'my-worker::ui-content',
)
```

## Skills shipped with this worker

- `ade/injectable-ui` — the delivery contract for worker UI: project
  layout, `setup(host)` and every slot, the wire contract, Rust and Node
  registration, the shared build driver and its lint, scoped CSS, the
  hooks/format/icon packages, hot reload, debugging, and the definition of done.
- `ade/design-console-ui` — responsive UX and forms: archetypes, pane-width
  (not viewport) behavior, phone drill-in and bottom sheets, state integrity,
  configuration and provider forms, and the validation matrix.
- `ade/design-system` — the iii Schematic design system: tokens, surface
  ramp, typography, radius, elevation, motion, the canonical components, and
  the one numbers table the other two skills link to.

The migration of the remaining worker UIs onto the shared package is tracked
in `docs/plans/2026-09-16-worker-ui-migration.md`.
