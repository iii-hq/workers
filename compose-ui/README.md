# compose-ui

The compose daemon in the Console. A **Compose** page opens on a topology map (engine, namespace group, containers laid out by `start_after`, ports and sources on the cards, upstream/downstream tracing on select) and lists every container the
daemon supervises with live state, PID, and last error; starts, stops, and
restarts containers or the whole project in dependency order; adds, updates,
and removes declared worker packages; and tails each container's log. Every
lifecycle action is the daemon's own `compose::*` function, so the page and
`iii trigger compose::…` are the same operation. The worker adds the two things
the daemon does not expose on the bus: a `compose-ui::changed` trigger type
that fires when the daemon writes its durable project state or the compose
file changes on disk, and `compose-ui::logs`, a read-only log tail.

## Install

```bash
iii compose add compose-ui
```

Compose starts it beside the Console in the project's namespace. The page
appears in the Console nav as **Compose** (`#/ext/compose`) and in the command
palette as "Open Compose"; the palette also searches containers by name.

### Companion workers

- `console` ≥ 1.9.14 hosts the injectable page.
- A running `iii compose` daemon answers `compose::*` in the same namespace;
  without one the page shows how to start it.

## Quickstart

Explain a failed container from an agent or a script:

```bash
iii trigger compose::status
iii trigger compose-ui::logs container=provider-openai lines=50
```

```json
{
  "container": "provider-openai",
  "path": "/home/me/.iii/compose/app/harness-3d528231/logs/provider-openai.log",
  "lines": ["… last 50 lines …"],
  "size": 17910,
  "truncated": true,
  "missing": false
}
```

See what the project declares and what each container listens on:

```bash
iii trigger compose-ui::project
```

```json
{
  "namespace": "my-project",
  "engine_url": "ws://127.0.0.1:49134",
  "containers": [
    { "name": "console", "source": "path", "ref": "../console", "pid": 27129, "ports": [{ "port": 3113, "address": "*" }] },
    { "name": "web", "source": "package", "ref": "api.workers.iii.dev/web", "version": "1.2.10", "ports": [] }
  ]
}
```

React to supervisor changes without polling — bind the trigger type with an
empty config and re-read `compose::status` on each event:

```ts
iii.registerTrigger({ type: 'compose-ui::changed', function_id: 'my-worker::on-compose-change', config: {} })
```

The payload is `{ kind: 'state' | 'file', file, namespace, state_dir, path, captured_at }`.

## Configuration

None. The worker locates the project through `compose::status` (compose file,
namespace, state directory) the first time a page or trigger binds, and again
if the watch is lost. Environment:

| Variable | Purpose |
|---|---|
| `III_URL` | Engine address; compose sets it. |
| `III_NAMESPACE` | Project namespace for a process started by hand; compose sets it. |
| `III_COMPOSE_UI_UI_WATCH` | `1` serves the page from `ui/dist` and hot-reloads it into open Console tabs while `pnpm --dir ui watch` runs. |

## Custom trigger types

| Type | Config | Fires |
|---|---|---|
| `compose-ui::changed` | `{}` | Once per coalesced burst (200 ms) of writes to the daemon's `state.json` (`kind: state`) or edits to the compose file (`kind: file`). |

## Run from source with compose

```yaml
containers:
  compose-ui:
    worker: path://../compose-ui
    scripts:
      run: pnpm install --ignore-workspace --ignore-scripts && pnpm build:bundle && node dist/bundle/index.mjs
```

A worker started by hand needs `III_NAMESPACE=<compose namespace>` or the
Console never sees its page.
