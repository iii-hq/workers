---
name: state
tags: state, key-value, storage, observe, reactive
description: >-
  Shared, adapter-backed key/value store addressed by scope and key, with an
  observable `state::on-changed` event so other functions run on every
  create, update, or delete without polling.
---

# state

The `state` worker is a server-side key/value store. Values are addressed by a `scope` (namespace) and a `key`, shared across every worker connected to the engine, and persisted through a pluggable adapter (`kv` or `redis`). Callers reach the core store through six `state::*` functions called with `iii.call({ function_id: 'state::...', payload })`. The worker also exposes `state::list_keys` and the internal `state::ui-content` function. Install it with `iii call compose::add worker=state`; it replaces the engine's legacy built-in state service, which must be removed from the engine config first.

State does not push updates to SDK clients. After every successful mutation the worker calls `state::on-changed` with a `state:created`, `state:updated`, or `state:deleted` event; functions that observe it react to data changes without polling. The `kv` adapter (default) supports `file_based` persistence (the default) or a volatile `in_memory` mode; `redis` proxies to a Redis backend. The function surface is identical across adapters.

## When to Use

- Two or more functions need shared state without standing up a separate database.
- A counter or per-entity document needs atomic partial updates instead of read-modify-write.
- A write in one function should cause side effects elsewhere (cache invalidation, audit logs, notifications, projections).
- You want a specific `scope`/`key` watched and reacted to only when that slot changes.

## Boundaries

- Schema-free by design — every value is opaque JSON. Use the `configuration` worker when entries need a registered JSON Schema and validation.
- Reads (`state::get`, `state::list`, `state::list_groups`) never emit `state::on-changed`; only `set`/`update`/`delete` and a swapped `compare-and-set` do.
- Delivery to observers is asynchronous and does not roll back the write on a target's failure; a delete of a missing key still emits `state:deleted` with a null old value.
- Stream-shaped, broadcast-to-subscribers data belongs in the engine's stream surface, not here.
- The store lives in the worker process: the `kv` `in_memory` backend is lost when the worker stops. The default `file_based` backend (and `redis`) survives restarts.

## Functions

- `state::set` — write or replace the value at a `scope`/`key`; emits `state:created` or `state:updated`.
- `state::get` — read one value by `scope` and `key`.
- `state::delete` — remove a key and emit `state:deleted` with the prior value.
- `state::update` — apply ordered atomic ops (`set`, `merge`, `increment`, `decrement`, `append`, `remove`) to the stored value.
- `state::list` — enumerate every value stored in a scope.
- `state::list_groups` — enumerate which scopes currently contain data.
- `state::on-changed` — the change event: observe it, never call it (private to the state worker).

## Observing changes

Observe `state::on-changed` when a function should run automatically after a value changes — without polling `state::get`. It takes no args: select changes with filters on `input.scope`, `input.key` and `input.event_type` (any event field works, including `input.new_value.*`).

Reach for it when:

- A write in one worker should drive side effects in another (audit logs, cache invalidation, notifications, projections).
- You want only some changes: filters select them, for example `{ 'input.event_type': { $eq: 'state:deleted' } }`.

If you only need the new value inside the same function that wrote it, use the mutator's returned `old_value` / `new_value` instead of observing.

### How to observe

1. Register a handler: `iii.registerFunction('orders::on-status-change', handler)`.
2. Observe the change event with it:

```typescript
iii.observe(
  {
    function_id: 'state::on-changed',
    filters: {
      'input.scope': { $eq: 'orders' },  // omit to match every scope
      'input.key': { $eq: 'status' },    // omit to match every key in the scope
    },
    deliver: 'output',
  },
  'orders::on-status-change',
)
```

Filters take explicit operators (`$eq`, `$in`, `$ne`, …). To watch several specific keys, use `{ 'input.key': { $in: ['status', 'total'] } }`. Mutations that emit: `state::set`, `state::update`, `state::delete`, a swapped `state::compare-and-set`.

With `deliver: 'output'` the handler receives the event `{ type: "state", event_type, scope, key, old_value, new_value }`, where `event_type` is one of `state:created`, `state:updated`, or `state:deleted`. With the default `deliver` it receives the `Observed` envelope instead: `output` is the event and `caller.parent` the worker that wrote. Pass `{ metadata }` as the third argument of `iii.observe` to hand the handler a sidecar with every event.
