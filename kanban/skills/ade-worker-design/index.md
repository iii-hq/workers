---
name: ade-worker-design
description: >-
  Design and build the UI an iii worker injects into the ADE console — pages,
  function and trigger-activity renderers, configuration forms and scoped
  stylesheets — against the console's host contract, its design system and the
  record-shaped interaction patterns. Read it before adding or changing any
  worker console UI.
---

# ade-worker-design

The ADE console is the host every worker can inject UI into at runtime: a
worker registers a `console:script` and a `console:style` asset, every open
tab imports the module, calls its `setup(host)`, and hot-reloads it on
re-registration. No console rebuild, no iframe. This skill is the designer's
half of building such a worker: what the host accepts, how the console looks
and behaves, and which interaction pattern fits which kind of data. The
worker's service half — package shape, functions, configuration, delivery,
compose wiring — is the `iii-node` skill and belongs to the Backend Engineer.

## The three references

Read them in this order, fully rather than skimmed:

- `console-injectable-ui` — the authoring contract: the wire contract and
  asset rules, the `setup(host)` slots (pages, panels, function-trigger and
  trigger-activity renderers, configuration forms, provider forms, chat
  slots), `host.iii` for live data, build with exactly five externals,
  scoping, hot reload, debugging, testing and the delivery checks.
- `console-design` — the iii Schematic design system: surface ramp, tokens,
  typography, radius, motion, icons, selection, responsive rules and the
  component grammar every injected page must compose from.
- `patterns` — recipes for record-shaped UIs: a board with lanes and drag and
  drop, a record screen that opens as its own pane, activity timelines with
  threaded comments, creation dialogs, chat cards for agent calls, settings
  forms and live updates. Pick the pattern before writing JSX.

Fetch any of them with `directory::skills::get { "id": "kanban/ade-worker-design/<name>" }`.

## Boundaries

- The console renders; the worker owns the data. Injected UI acts only by
  calling its own worker's functions over `host.iii`, and configuration values
  live in the `configuration` worker (`kanban/iii-node/configuration`).
- Components, props, slots and host methods come from
  `@iii-dev/console-ui`'s `index.d.ts`, never from memory. If it is not
  declared there, it does not exist.
- Every rule in the worker stylesheet is scoped under
  `[data-iii-ui="<worker>"]`; an unscoped rule restyles the whole console.
- Asset triggers go through the SDK Message path, never the durable
  `engine::register_trigger`, and never `console:assets`.
- Done means the UI was seen in the running console at phone, narrow-split
  and wide widths, in both themes, with the manifest free of warnings — not
  that the build passed.
