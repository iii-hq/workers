# state

Distributed key-value state storage with scope-based organization and
reactive change triggers. Values are addressed by a `scope` (namespace) and a
`key`, shared across every worker connected to the engine, and persisted
through a pluggable adapter (`kv` or `redis`). Callers reach the store
through six functions — `state::set`, `state::get`, `state::delete`,
`state::update`, `state::list`, `state::list_groups` — and this worker also
registers the `state` trigger type, which fires `state:created`,
`state:updated`, or `state:deleted` after every successful mutation so
downstream functions can react to data changes without polling. On iii 0.25
engines the same events can also be observed through the `state::on-changed`
function. This worker is the standalone migration of the engine's legacy
built-in state service.

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
iii trigger compose::add worker=state
```

`iii trigger compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Functions

| Function | Input | Returns | Fires |
|---|---|---|---|
| `state::set` | `{ scope, key, value }` (`data` accepted as an alias for `value`) | `{ old_value, new_value }` | `state:created` (new key) or `state:updated` |
| `state::get` | `{ scope, key }` | the value, or `null` | — |
| `state::delete` | `{ scope, key }` | the deleted value, or `null` | `state:deleted` (even when the key did not exist) |
| `state::update` | `{ scope, key, ops }` — ordered atomic ops: `set`, `merge`, `increment`, `decrement`, `append`, `remove` | `{ old_value, new_value, errors }` | `state:created` or `state:updated` |
| `state::list` | `{ scope }` | flat array of every value in the scope | — |
| `state::list_keys` | `{ scope }` | `{ keys }` — the keys stored in the scope, adapter order (additive; no builtin counterpart — added for the console state UI, whose per-item navigation `state::list`'s values-only shape cannot drive) | — |
| `state::list_groups` | `{}` | `{ groups }` — sorted, deduplicated scope names | — |
| `state::ui-content` | `{ path }` | `{ content, content_type }` — content function for the injected console UI (internal; see [Console UI](#console-ui)) | — |
| `state::on-changed` | the change event (see [Observing changes](#observing-changes-iii-025)) | `null` | — (observe it; only this worker may invoke it) |

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
| `ui/src/configuration/` | `host.configForms` | deliberate form for the `state` entry in global Settings (adapter picker, persistence, trigger gate, limits) |
| `ui/src/function-trigger-message/` | `host.functionTriggers` | how `state::set` / `state::get` triggers render in chat and the traces span view (overrides the console's built-in family for those two ids; errors and other `state::*` ids fall through) |

`ui/page.tsx` is the esbuild entry: a thin `setup(host)` composing the three
registrations plus the shared scoped stylesheet (`ui/src/lib/styles.ts`).

The page is **live**: it registers one tab-scoped `state` trigger binding
(empty config = every scope/key) whose function_id is a per-tab handler
(`iii::state-ui::events::<browserId>`), and applies the
created/updated/deleted events in place — new scopes and keys appear as
they are written, an open item editor updates in place when clean, and
unsaved edits are never clobbered (a "changed on the server" notice offers
the incoming value instead). The binding is GC'd with the tab; the `iii::`
prefix keeps the per-event invocations out of the trace feed.

- Build: esbuild over `ui/page.tsx` and `ui/styles.css` (`cd ui && pnpm
  build`; `build.rs` does this automatically) with `react` and
  `@iii-dev/console-ui` external — they resolve through the console's
  import map at runtime. Types come from the workspace-linked
  `packages/console-ui` (repo-root pnpm workspace).
- Deployment: boot registers a `console:script` trigger for `state/page.js`
  and a `console:style` trigger for `state/styles.css`, both with
  `function_id` `state::ui-content`; the console fetches, hashes, and
  serves the assets, and disposes them when this worker disconnects. The
  stylesheet is scoped under `[data-iii-ui="state"]` and mounted as a
  `<link>` (styles-before-scripts on boot, link-swap on change).
- Dev loop: run `cd ui && pnpm watch` and start the worker with
  `III_STATE_UI_WATCH=1` — it polls `ui/dist/` and re-registers a changed
  asset's trigger, hot-swapping it in every open console tab.

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

## Observing changes (iii 0.25)

On an engine with observe (iii 0.25 and later) the same events are the input
of `state::on-changed`, an auto-delivered function: observe it with no args
and select with filters instead of a trigger config.

```typescript
iii.registerFunction('orders::on-status-change', ({ input, caller }) => {
  // input: { type, event_type, scope, key, old_value, new_value }
})

const observer = iii.observe(
  {
    function_id: 'state::on-changed',
    filters: { 'input.scope': { $eq: 'orders' }, 'input.key': { $eq: 'status' } },
  },
  'orders::on-status-change',
)
await observer.ready()
```

- The input is the document a `state` trigger handler receives (above), so
  filters read `input.scope`, `input.key`, `input.event_type`,
  `input.old_value.*` and `input.new_value.*`. `deliver: 'input'` hands the
  target that document alone instead of the `Observed` envelope.
- The envelope's `caller` is this worker; `caller.function_id` is the
  `state::*` function that made the write and `caller.parent` the worker
  (and function) that called it.
- Every committed write emits, as for the trigger type: `state::set`,
  `state::update`, `state::delete` and a swapped `state::compare-and-set`.
  Private namespaces never emit, and `triggers_enabled: false` silences
  both paths.
- The worker calls the function in-process after each write, which is what
  activates its observers. With nobody observing it, nothing is built or
  sent.
- Only this worker emits it. Any other invocation (another worker, `iii
  trigger`, an observer activation) is refused with `FORBIDDEN`, so an
  observer with the default `on: 'success'` sees real changes only;
  `on: 'error'` / `'complete'` observers see refused invocations as errors.
- `condition_function_id` has no counterpart: filters replace it.
- The `state` trigger type keeps working next to it, on 0.24 and 0.25
  engines. The two paths are independent: a function both bound with
  `registerTrigger` and targeted by an observer runs twice per change. On a
  0.24 engine `observe()` fails with `UNSUPPORTED`; bind the trigger type
  there.

## Configuration

| Field | Default | Description |
|---|---|---|
| `adapter` | `kv` | Storage adapter: `kv` (in-process; `store_method: file_based` — the default, persisting under `file_path`, default `data/state` relative to `III_COMPOSE_DIR` — or `in_memory`, volatile) or `redis` (`redis_url`, default `redis://localhost:6379`). Restart-tier: a runtime change is logged and takes effect at the next worker start. |
| `triggers_enabled` | `true` | Globally enable/disable state change-trigger fan-out, and `state::on-changed` emission. Applied live. |
| `max_value_bytes` | unset (no limit) | Reject `state::set` writes whose JSON-serialized value exceeds this many bytes (`VALUE_TOO_LARGE`). Minimum `1`. Applied live. |
| `save_interval_ms` | `5000` | Persistence flush cadence (ms) for the file-backed `kv` adapter. `100`–`3600000`. Applied live by respawning the adapter's save loop (hot-retune; no-op for in-memory/redis). |

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → state**) or seed it once via the
worker's config block in `config.yaml` on first boot. `triggers_enabled` and
`max_value_bytes` apply on the next write, `save_interval_ms` hot-retunes
the save loop, and `adapter` takes effect on the next restart.

### Requires removing the legacy built-in state service

The legacy built-in state service also owns the `state` trigger type and the
`state::*` functions. Two owners of the same surface on one engine collide —
whichever registers last wins — so this worker requires it to be
absent: omit it from the engine's `config.yaml` (a config that doesn't list
a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if the legacy built-in is still active, so a stale config
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
| Trigger `metadata` | forwarded to handlers via call_with_metadata | not forwarded (iii-sdk 0.20 TriggerRequest has no metadata field; same limitation as the http worker) |
| Store adapters | kv (in_memory/file_based), redis, bridge | kv, redis (bridge not ported — see docs/adr/0001) |
| kv on-disk format | rkyv `.bin` per scope | identical — builtin data loads as-is |
| `save_interval_ms` | default 5000ms, floor 100ms, hot-retune | same |
| `max_value_bytes` | guards `set` only, `VALUE_TOO_LARGE` | same (code is the message prefix) |
| Error codes | coded `ErrorBody` (`SET_ERROR`, ...) | code as message prefix (SDK handler errors carry a message) |
| Durability | store dies with the ENGINE process | store dies with the WORKER process (ADR 0001; file_based/redis unchanged) |
| Latency | in-process µs | engine⇄worker WS round-trip (low ms) — formal benchmark waived by project decision 2026-07-06 |
| Telemetry (`track_state_*`) | engine-internal counters | none — out of scope for this migration; lands with the shared worker observability story |
