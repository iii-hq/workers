---
name: compose-ui
description: >-
  Explain why a compose container failed or what the supervisor is doing by
  reading its log tail; use it when a person asks about container state, a
  worker that will not come up, or wants the Compose page open beside the chat.
---

# compose-ui

The compose-ui worker puts the compose daemon in the Console. Its page (Compose,
`#/ext/compose`) opens on a topology map of the engine, the namespace, and the
containers laid out by `start_after`, and shows every container the daemon supervises with its live
state, PID, and last error; starts, stops, and restarts containers or the whole
project in dependency order; adds, updates, and removes declared worker
packages; and tails each container's log. Every lifecycle action is the
daemon's own `compose::*` function, so what the page does and what
`iii trigger compose::…` does are the same operation.

The worker itself owns two things the daemon does not expose on the bus: a
`compose-ui::changed` trigger type that fires when the daemon writes its
durable project state or the compose file changes on disk, and
`compose-ui::logs`, a read-only tail of one container's log.

## When to Use

- A person asks why a container is `failed` or stuck in `starting`:
  `compose::status` gives the state and `last_error`; `compose-ui::logs` gives
  the last lines the process wrote before it died.
- A person wants to see the fleet, restart something, or add a worker without
  leaving the chat: open the Compose page with `compose` in the command palette
  or `host.panels.open({ pageId: 'compose', context: { container } })` from
  another worker's UI.
- A worker needs to react to supervisor changes (a crash cascade, a new
  container, a compose file edit) without polling: bind
  `compose-ui::changed` with an empty config and re-read `compose::status`.

## Boundaries

- Lifecycle belongs to the daemon. Call `compose::up`, `compose::down`,
  `compose::restart`, `compose::add`, `compose::remove`, and `compose::update`
  directly; this worker adds no proxies for them.
- `compose-ui::project` reports environment variable names only, never values.
- `compose-ui::logs` reads at most 500 lines and 256 KiB from the end of one
  file and never lists or reads anything else in the state directory.
- The worker must run in the compose project's namespace (compose supplies it;
  a standalone process needs `III_NAMESPACE`).

## Functions

| Function | Purpose |
|---|---|
| `compose-ui::logs` | `{ container, lines?, file? }` → last lines of `<state_dir>/logs/<container>.log` with `size`, `truncated`, `missing`. |
| `compose-ui::project` | `{ file? }` → the compose file as declared (namespace, engine endpoint, timeouts, each container's source, version, `start_after`, environment keys, run script) plus each running container's PID and listening TCP ports. |

## Trigger types

| Type | Config | Payload |
|---|---|---|
| `compose-ui::changed` | `{}` | `{ kind: 'state' \| 'file', file, namespace, state_dir, path, captured_at }` |
