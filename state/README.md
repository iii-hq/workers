# state

Distributed key-value state storage with scope-based organization and an
observable change event. Values are addressed by a `scope` (namespace) and a
`key`, shared across every worker connected to the engine, and persisted
through a pluggable adapter (`kv` or `redis`). Callers reach the store
through `state::set`, `state::get`, `state::delete`, `state::update`,
`state::compare-and-set`, `state::list`, `state::list_keys` and
`state::list_groups` (the [Functions](#functions) table lists every function
the worker provides), and after every
successful mutation the worker calls `state::on-changed` with a
`state:created`, `state:updated`, or `state:deleted` event, so downstream
functions observe data changes without polling. This worker is the
standalone migration of the engine's legacy built-in state service.

## Install

Workers can reserve private scopes with `state::claim-namespace`. Their internal
`<worker>::state::compare-and-set` accessor accepts `durable: true` to flush the
adapter before acknowledging the operation. It also flushes an unmatched retry,
so a cleanup that previously changed memory but failed to persist can be retried
safely. The default is `false`; the public state API is unchanged. Durability
follows the configured adapter's existing flush guarantees.
The private response includes `durable: true` only after a requested flush
succeeds, allowing callers to detect workers that do not support the option.

```bash
iii call compose::add worker=state
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Functions

| Function | Input | Returns | Fires |
|---|---|---|---|
| `state::set` | `{ scope, key, value }` (`data` accepted as an alias for `value`) | `{ old_value, new_value }` | `state:created` (new key) or `state:updated` |
| `state::get` | `{ scope, key }` | the value, or `null` | — |
| `state::delete` | `{ scope, key }` | the deleted value, or `null` | `state:deleted` (even when the key did not exist) |
| `state::update` | `{ scope, key, ops }` — ordered atomic ops: `set`, `merge`, `increment`, `decrement`, `append`, `remove` | `{ old_value, new_value, errors }` | `state:created` or `state:updated` |
| `state::compare-and-set` | `{ scope, key, expected?, value }` — writes `value` only when the stored value equals `expected`; without `expected`, only when the key is absent | `{ swapped, current? }` — `current` is the stored value when the swap did not happen | `state:created` or `state:updated` when it swaps |
| `state::barrier` | `{ event, condition_config: { id, expect, key_from?, carry? } }` — records one arrival at the barrier `id`: `expect` is a count or the list of arrival keys, `key_from` a JSON pointer to the arrival key in `event` (default `/key`), `carry` a JSON pointer to the value to keep (default: the whole event) | `{ decision: "allow", payload, reason }` exactly once, on the arrival that completes the set (`payload` carries every arrival's value); `{ decision: "skip", reason }` otherwise | — |
| `state::claim-namespace` | `{ functions_prefix, scopes }` — `functions_prefix` must equal the calling worker's name | `{ claimed, scopes, functions }` — the scopes leave the public `state::*` functions and `state::on-changed`; `claimed` is `false` when the worker already owned them all; `functions` names its accessors (next row) | — |
| `<prefix>::state::get`, `<prefix>::state::list`, `<prefix>::state::compare-and-set` | as `state::get`, `state::list` and `state::compare-and-set`, limited to the scopes the worker `<prefix>` claimed; `compare-and-set` also takes `durable` (see [Install](#install)) | as their public counterparts; `compare-and-set` adds `durable` | — (registered by the first `state::claim-namespace` of `<prefix>`, internal) |
| `state::list` | `{ scope }` | flat array of every value in the scope | — |
| `state::list_keys` | `{ scope }` | `{ keys }` — the keys stored in the scope, adapter order (additive; no builtin counterpart — added for the console state UI, whose per-item navigation `state::list`'s values-only shape cannot drive) | — |
| `state::list_groups` | `{}` | `{ groups }` — sorted, deduplicated scope names | — |
| `state::ui-content` | `{ path }` | `{ content, content_type }` — content function for the injected console UI (internal; see [Console UI](#console-ui)) | — |
| `state::on-changed` | the change event (see [Observing changes](#observing-changes)) | the same event | — (observe it; private: only this worker calls it) |
| `state::on-config-change` | the `configuration:updated` event of the worker's entry (ignored) | `{ ok: true }` — internal: the target of the worker's observer of `configuration::on-updated`; it re-reads the entry and applies it (see [Configuration](#configuration)) | — |

The `harness_binding` and `harness_binding_owner` scopes are reserved
control-plane state: public functions reject direct access, omit them from
group listings, and never emit their bookkeeping writes as state events.

## Console UI

The worker ships its console UI into any running console (the injectable-UI
protocol, `iii/tech-specs/2026-07-17-injectable-ui`) — three contributions,
one module each under `ui/src/`:

| Module | Slot | What it does |
|---|---|---|
| `ui/src/page/` | `host.pages` | the state-manager page (page `state-manager`): browse scopes → keys → edit one value as JSON and save it back with `state::set` |
| `ui/src/configuration/` | `host.configForms` | deliberate form for the `state` entry in global Settings (adapter picker, persistence, change-event gate, limits) |
| `ui/src/function-call-message/` | `host.functionCalls` | how `state::set` / `state::get` calls render in chat and the traces span view (overrides the console's built-in family for those two ids; errors and other `state::*` ids fall through) |

`ui/page.tsx` is the esbuild entry: a thin `setup(host)` composing the three
registrations plus the shared scoped stylesheet (`ui/src/lib/styles.ts`).

The page is **live**: it observes `state::on-changed` once per tab (no
filters = every scope/key, `deliver: 'output'`) with a per-tab handler as
the target (`iii::state-ui::events::<browserId>`), and applies the
created/updated/deleted events in place — new scopes and keys appear as
they are written, an open item editor updates in place when clean, and
unsaved edits are never clobbered (a "changed on the server" notice offers
the incoming value instead). The observer is disposed with the page; the
`iii::` prefix keeps the per-event invocations out of the trace feed.

- Build: esbuild over `ui/page.tsx` and `ui/styles.css` (`cd ui && pnpm
  build`; `build.rs` does this automatically) with `react` and
  `@iii-dev/console-ui` external — they resolve through the console's
  import map at runtime. Types come from the workspace-linked
  `packages/console-ui` (repo-root pnpm workspace).
- Deployment: boot observes `console::ui::asset` twice, with args
  `{ kind: 'script', path: 'state/page.js' }` and
  `{ kind: 'style', path: 'state/styles.css' }`, both targeting
  `state::ui-content`; the console fetches, hashes, and serves the assets,
  and drops them when this worker disconnects. The
  stylesheet is scoped under `[data-iii-ui="state"]` and mounted as a
  `<link>` (styles-before-scripts on boot, link-swap on change).
- Dev loop: run `cd ui && pnpm watch` and start the worker with
  `III_STATE_UI_WATCH=1` — it polls `ui/dist/` and replaces a changed
  asset's observer, hot-swapping it in every open console tab.

## Observing changes

`state::on-changed` is the change event. After every committed write the
worker calls it with the event, and its handler returns that event, so
observers see it as the call's `input` and `output`.

| Function | Args | Delivery | Event |
|---|---|---|---|
| `state::on-changed` | none; filter on `input.scope`, `input.key`, `input.event_type` | iii delivers every call to each observer whose filters match | `{ type: "state", event_type, scope, key, old_value, new_value }` |

```typescript
iii.registerFunction('orders::on-status-change', (event: StateEvent) => {
  // event: { type, event_type, scope, key, old_value, new_value }
})

const observer = iii.observe(
  {
    function_id: 'state::on-changed',
    filters: { 'input.scope': { $eq: 'orders' }, 'input.key': { $eq: 'status' } },
    deliver: 'output',
  },
  'orders::on-status-change',
)
await observer.ready()
```

The target receives the event:

```json
{
  "type": "state",
  "event_type": "state:updated",
  "scope": "orders",
  "key": "status",
  "old_value": { "status": "pending" },
  "new_value": { "status": "shipped" }
}
```

- `event_type` is one of `state:created`, `state:updated`, `state:deleted`;
  `old_value` is `null` for created keys and `new_value` is `null` for
  deleted keys.
- Filters read any field of the event: `input.scope`, `input.key`,
  `input.event_type`, `input.old_value.*`, `input.new_value.*`. With no
  filters an observer receives every change.
- With the default `deliver` the target receives the `Observed` envelope:
  its `caller.function_id` is the `state::*` function that made the write
  and `caller.parent` the worker (and function) that called it. Filters on
  `caller.parent.worker_name` select or skip one writer's changes.
- Every committed write emits: `state::set`, `state::update`,
  `state::delete` (even of a missing key) and a swapped
  `state::compare-and-set`. Private namespaces never emit, and
  `events_enabled: false` silences the function.
- Delivery is asynchronous: targets run after the write completes, and a
  target's failure never rolls the write back. With nobody observing,
  nothing is built or called.
- The function is private: any call from another worker is refused with
  `FORBIDDEN`, so its observers see real changes only. The observer's
  metadata reaches the target as the call's metadata sidecar.

## Configuration

| Field | Default | Description |
|---|---|---|
| `adapter` | `kv` | Storage adapter: `kv` (in-process; `store_method: file_based` — the default, persisting under `file_path`, default `data/state` relative to `III_COMPOSE_DIR` — or `in_memory`, volatile) or `redis` (`redis_url`, default `redis://localhost:6379`). Restart-tier: a runtime change is logged and takes effect at the next worker start. |
| `events_enabled` | `true` | Call `state::on-changed` after every committed write. Applied live. |
| `max_value_bytes` | unset (no limit) | Reject `state::set` writes whose JSON-serialized value exceeds this many bytes (`VALUE_TOO_LARGE`). Minimum `1`. Applied live. |
| `save_interval_ms` | `5000` | Persistence flush cadence (ms) for the file-backed `kv` adapter. `100`–`3600000`. Applied live by respawning the adapter's save loop (hot-retune; no-op for in-memory/redis). |

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → state**) or seed it once via the
worker's config block in `config.yaml` on first boot. `events_enabled` and
`max_value_bytes` apply on the next write, `save_interval_ms` hot-retunes
the save loop, and `adapter` takes effect on the next restart. An entry or
seed written by a release before 0.26 names the `events_enabled` knob
differently; the worker rewrites the stored entry to the field names above
when it boots, before it reads it.

### Requires removing the legacy built-in state service

The legacy built-in state service also owns the `state::*` functions. Two owners of the same surface on one engine collide —
whichever registers last wins — so this worker requires it to be
absent: omit it from the engine's `config.yaml` (a config that doesn't list
a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if the legacy built-in is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`state::*`.

**Store migration:** if the builtin used a file-based `kv` store, point this
worker's `adapter.config.file_path` at the builtin's existing directory
(the default engine config used `./data/state_store.db`). The on-disk format
(one rkyv `.bin` file per scope) is identical, so the existing data loads
as-is — no export/import step.

## Latency

The builtin ran in-process inside the engine, so a `state::*` call cost
microseconds; as a standalone worker every call crosses the engine⇄worker
WebSocket, which puts round-trips in the low-millisecond range. That
order-of-magnitude delta is inherent to the migration and applies to every
externalized builtin. A formal benchmark was waived by project decision
(2026-07-06).

## Parity vs builtin

| Behavior | Builtin | This worker |
|---|---|---|
| Function ids | `state::set/get/delete/update/list/list_groups` | same (exact) |
| `set` input | `{scope, key, value}` (`data` alias) | same |
| Events | `state:created`/`updated`/`deleted`, payload `{type:"state", event_type, scope, key, old_value, new_value}` | same, as the input and output of `state::on-changed` |
| Selecting events | `scope` / `key` equality, `condition_function_id` | observer filters on any event field (`input.scope`, `input.key`, `input.event_type`, …) |
| Per-subscriber metadata | forwarded to handlers via call_with_metadata | the observer's `metadata`, delivered by iii as the sidecar |
| Store adapters | kv (in_memory/file_based), redis, bridge | kv, redis (bridge not ported — see docs/adr/0001) |
| kv on-disk format | rkyv `.bin` per scope | identical — builtin data loads as-is |
| `save_interval_ms` | default 5000ms, floor 100ms, hot-retune | same |
| `max_value_bytes` | guards `set` only, `VALUE_TOO_LARGE` | same (code is the message prefix) |
| Error codes | coded `ErrorBody` (`SET_ERROR`, ...) | code as message prefix (SDK handler errors carry a message) |
| Durability | store dies with the ENGINE process | store dies with the WORKER process (ADR 0001; file_based/redis unchanged) |
| Latency | in-process µs | engine⇄worker WS round-trip (low ms) — formal benchmark waived by project decision 2026-07-06 |
| Telemetry (`track_state_*`) | engine-internal counters | none — out of scope for this migration; lands with the shared worker observability story |
