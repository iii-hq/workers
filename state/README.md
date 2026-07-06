# state

Distributed key-value state storage with scope-based organization and
reactive change triggers. Values are addressed by a `scope` (namespace) and a
`key`, shared across every worker connected to the engine, and persisted
through a pluggable adapter (`kv` or `redis`). Callers reach the store
through six functions — `state::set`, `state::get`, `state::delete`,
`state::update`, `state::list`, `state::list_groups` — and this worker also
registers the `state` trigger type, which fires `state:created`,
`state:updated`, or `state:deleted` after every successful mutation so
downstream functions can react to data changes without polling. This worker
is the standalone migration of the engine's built-in `iii-state`.

## Install

```bash
iii worker add state
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Functions

| Function | Input | Returns | Fires |
|---|---|---|---|
| `state::set` | `{ scope, key, value }` (`data` accepted as an alias for `value`) | `{ old_value, new_value }` | `state:created` (new key) or `state:updated` |
| `state::get` | `{ scope, key }` | the value, or `null` | — |
| `state::delete` | `{ scope, key }` | the deleted value, or `null` | `state:deleted` (even when the key did not exist) |
| `state::update` | `{ scope, key, ops }` — ordered atomic ops: `set`, `merge`, `increment`, `decrement`, `append`, `remove` | `{ old_value, new_value, errors }` | `state:created` or `state:updated` |
| `state::list` | `{ scope }` | flat array of every value in the scope | — |
| `state::list_groups` | `{}` | `{ groups }` — sorted, deduplicated scope names | — |

## Trigger type

This worker always registers the `state` trigger type. Bind a function to it
with:

| Field | Required | Default | Description |
|---|---|---|---|
| `scope` | no | any scope | Only fire for writes in this scope. |
| `key` | no | any key | Only fire for writes to this key. |
| `condition_function_id` | no | — | Function invoked first with the event; only an explicit `false` return skips the handler (null/no result passes, an error skips and logs). |

Trigger delivery is asynchronous: handlers run after the write completes and
a handler failure never rolls the write back. Duplicate trigger ids replace
the previous binding silently (builtin parity).

```typescript
iii.registerTrigger({
  type: 'state',
  function_id: 'orders::on-status-change',
  config: { scope: 'orders', key: 'status' },
})
```

The handler receives the event payload:

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

`event_type` is one of `state:created`, `state:updated`, `state:deleted`;
`old_value` is `null` for created keys and `new_value` is `null` for deleted
keys.

## Configuration

| Field | Default | Description |
|---|---|---|
| `adapter` | `kv` | Storage adapter: `kv` (in-process; `store_method: in_memory` or `file_based` with `file_path` and `save_interval_ms`) or `redis` (`redis_url`, default `redis://localhost:6379`). Restart-tier: a runtime change is logged and takes effect at the next worker start. |
| `triggers_enabled` | `true` | Globally enable/disable state change-trigger fan-out. Applied live. |
| `max_value_bytes` | unset (no limit) | Reject `state::set` writes whose JSON-serialized value exceeds this many bytes (`VALUE_TOO_LARGE`). Minimum `1`. Applied live. |
| `save_interval_ms` | `5000` | Persistence flush cadence (ms) for the file-backed `kv` adapter. `100`–`3600000`. Applied live by respawning the adapter's save loop (hot-retune; no-op for in-memory/redis). |

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → state**) or seed it once via the
worker's config block in `config.yaml` on first boot. `triggers_enabled` and
`max_value_bytes` apply on the next write, `save_interval_ms` hot-retunes
the save loop, and `adapter` takes effect on the next restart.

### Requires removing the built-in `iii-state` worker

The built-in `iii-state` worker also owns the `state` trigger type and the
`state::*` functions. Two owners of the same surface on one engine collide —
whichever registers last wins — so this worker requires `iii-state` to be
absent: omit it from the engine's `config.yaml` (a config that doesn't list
a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if `iii-state` is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`state`.

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
| Events | `state:created`/`updated`/`deleted`, payload `{type:"state", event_type, scope, key, old_value, new_value}` | same |
| Trigger config | `{scope?, key?, condition_function_id?}`; only explicit `false` blocks | same |
| Duplicate trigger id | silent replace | same |
| Store adapters | kv (in_memory/file_based), redis, bridge | kv, redis (bridge not ported — see docs/adr/0001) |
| kv on-disk format | rkyv `.bin` per scope | identical — builtin data loads as-is |
| `save_interval_ms` | default 5000ms, floor 100ms, hot-retune | same |
| `max_value_bytes` | guards `set` only, `VALUE_TOO_LARGE` | same (code is the message prefix) |
| Error codes | coded `ErrorBody` (`SET_ERROR`, ...) | code as message prefix (SDK handler errors carry a message) |
| Durability | store dies with the ENGINE process | store dies with the WORKER process (ADR 0001; file_based/redis unchanged) |
| Latency | in-process µs | engine⇄worker WS round-trip (low ms) — formal benchmark waived by project decision 2026-07-06 |
| Telemetry (`track_state_*`) | engine-internal counters | none (see NOT in plan) |
