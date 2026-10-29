# Observer bindings — design

How an agent's `harness::observe` becomes a delivered invocation, and what the
harness keeps that the engine's observer does not.

Audience: anyone changing `functions/observe.rs`, `functions/observer_deliver.rs`,
`functions/observers_list.rs`, `bindings/*`, or the spawn/send seam. The
[integration contract](README.md) is the consumer-facing view;
[reactive-observers.md](reactive-observers.md) is the agent-facing reference;
this is the internal one.

## What the engine holds, and what the harness holds

An iii observer holds its source (function, namespace, args, filters), its
target, a metadata sidecar delivered with every activation, and an optional
policy (`max_fires`, `expires_in_ms`, `max_in_flight`). It belongs to the
connection that created it: a harness or engine restart loses it.

An agent observer needs more: an owner session, a lifecycle counted in
*delivered* calls (a condition that answers "not yet" must not spend a `once`),
the capability frozen when the agent observed, conditions, a payload template,
deadlines that tell the owner when a wake dies unfired, and durability across
restarts. The harness keeps all of it in a binding record, and the engine
observer carries only a pointer to it.

## The model

```
a call of the observed function
      │  (iii delivers the Observed envelope; sidecar { "__binding": id })
      ▼
harness::observer::deliver         ← the ONE target of every agent observer
      │
      ├── resolve the binding record (durable)
      ├── deleting-owner guard, stale spawn-target check
      ├── declared conditions (typed decision)
      ├── claim               (once / max_fires, CAS, before dispatch)
      ├── project             (payload template + event_into)
      └── dispatch            (harness::send for a wake, or the bound function)
                                         │
                                  observer_fired record
```

- **A wake** is `target: harness::send` into the owner session.
- **A mechanical reaction** is `target: <any allowed non-harness function>`.

There is no spawn shape: an observer never starts an agent.

The delivery hop is an ordinary, catalog-hidden function (`metadata.internal`).
It is not private, because a private function cannot be an observer's target.
It reads the event from the envelope's `output` — for an event source, the
event itself — and falls back to `input` when the output was not captured.

## The binding record

Durable in the state worker under the private `harness_binding/<binding_id>`
scope (see `src/state.rs`). Public `state::*` functions cannot read or mutate
it; the harness uses the state worker's hidden CAS primitives.

```jsonc
{
  "id": "sub_<32hex>",              // the binding id, and its observer's id
  "observer_id": "sub_<32hex>",     // set once this process created the observer
  "source": {
    "function_id": "state::on-changed",
    "namespace": "default",
    "args": null,
    "filters": { "input.scope": { "$eq": "run-x" } }
  },
  "owner": { "session_id": "<observer>", "root_session_id": null },
  "target": {
    "function_id": "harness::send",
    "payload": { },                 // template; default {}
    "event_into": "/event"          // JSON pointer; default "/event"
  },
  "conditions": [ { "function_id": "state::barrier", "config": { } } ],
  "lifecycle": { "once": true, "max_fires": null, "expires_at": null },
  "capability": { "allow": [ ], "deny": [ ], "expose": "…" },
  "dedup_key": { },                 // the canonical observe request
  "fires": 0,
  "created_at": 0
}
```

The record is as durable as the state worker's adapter: the default kv store is
in-memory unless a store method is configured, so a STATE worker restart drops
records even though a HARNESS restart does not. The binding pass then finds the
delivery observers without a record and disposes them.

`capability` is the session's dispatch policy **frozen when it observed**. A
delivered call is checked against it, never against a policy that widened
later.

The observer's metadata is exactly `{"__binding": "<id>"}`. Owner, target,
conditions, lifecycle and capability live in the record; nothing an agent can
write into metadata is read back. The binding id doubles as the observer id,
which is what lets the orphan sweep work from `engine::observers::list`, a
listing that never returns metadata.

## Observing

`harness::observe` validates the request (`args` and `filters` must be objects;
an absolute `lifecycle.expires_at` is refused by name), applies the per-shape
`once` default, resolves `harness::on-timer`'s `in_ms` to `at`, refuses an
already-settled Compose operation, resolves and authorizes the target and the
conditions, and resolves the source namespace home-first (the harness's own,
then `default`, then Compose's), probing `engine::functions::info`. Then, under
the topology lock:

1. **Dedup** — an identical canonical request in the same session returns the
   standing observer.
2. **Reserve** — the record is written (CAS-backed, with the 64-per-session
   cap) BEFORE the observer exists, so a call that arrives early still resolves
   its record. Its deadline timer is armed.
3. **Observe** — `iii.observe_with(source, harness::observer::deliver,
   id(binding id) + metadata({__binding}) + expires_in(deadline + 60 s))`.
   The handle is kept in `Deps::observer_handles`. The engine's expiry is a
   backstop; the harness's own timer retires the binding first and tells the
   owner. The harness never sets `max_fires` on the observer: it counts
   delivered calls itself.
4. **Attach** — the observer id is CAS-attached to the record. A fast one-shot
   that delivered and retired before the attach disposes its observer.

Then, outside the lock, the call waits up to 5 s for the observer to become
active: a refusal (`INVALID_ARGS`, `INVALID_FILTER`, `FORBIDDEN`,
`NOT_OBSERVABLE`, the worker's rejection) disposes the observer, deletes the
record and fails the call; an observer still `pending` waits for its function's
worker, and the response note says so.

### Observable Compose mutations

`compose::add`, `compose::update`, and `compose::remove` accept work before the
mutation finishes. A caller that needs the final result creates a fresh
operation id, observes `compose::on-operation-progress` (in Compose's namespace)
with a one-shot wake filtered on

```jsonc
{ "input.operation_id": { "$eq": "<operation-id>" }, "input.terminal": { "$eq": true } }
```

before it calls the mutation with the same `operation_id`. The terminal filter
lets only the success or failure event reach the wake. After the mutation, one
`compose::operation` read closes the activation race; a terminal snapshot is
delivered through the ordinary hop by recovery (below), so the caller removes
the still-armed wake only when it reads the result itself.

## Delivery

`harness::observer::deliver` runs this order, and the order is load-bearing:

1. **Resolve** the binding by `__binding`. Unknown id → a dropped delivery (the
   binding pass disposes that observer).
2. **Guards** — an owner being deleted stops delivery; a record whose target is
   `harness::spawn` is retired loudly.
3. **Spent** — a binding whose budget is spent is a silent duplicate (the same
   event by a second path); one past its deadline is retired and, for an
   unfired wake, its owner is told.
4. **Declared conditions**, in order, short-circuiting on the first `skip`.
5. **Claim** — compare-and-set the fire count *before* dispatching: a
   redelivered event finds the budget spent, and two simultaneous calls cannot
   take the same ordinal.
6. **Project** the payload: the target's template with the event injected at
   `event_into`.
7. **Dispatch**, checked against the frozen `capability` and the approval
   gate's verdict for the payload that will actually run. A call target is
   awaited, so a failing target is recorded as `delivery_failed`.
8. **Retire** (once / max_fires reached): dispose the observer, then
   compare-and-delete the record.
9. **Record** the `observer_fired` entry.

## The condition contract

A condition is an ordinary iii function. It receives

```jsonc
{ "event": { }, "condition_config": { }, "binding": { "id": "…", "fires": 0 }, "context": { } }
```

and returns

```jsonc
{ "decision": "allow" | "skip", "payload": { }, "reason": "…" }
```

- `allow` — continue; a returned `payload` replaces the event for later steps.
- `skip` — do not deliver; a standing binding stays armed.
- An **error** or an unparseable result → `skip`, with the reason recorded on
  the delivery and one notice to the owner per binding.

Conditions run at the hop, so they inherit the activation's observe depth, and
the engine's loop guard bounds a call-observer cycle that runs through them.

## Observability

Every delivery attempt appends a model-invisible `observer_fired` entry to the
owner session's transcript — including the ones that did **not** deliver, with
the deciding gate or condition and its reason. Each record names the observer
id, the observed function, its args and filters, the target, the label and
action, the outcome, the fire count and any retirement. Transcripts written
before 0.26 carry the same record as `trigger_fired`, which readers accept.
`harness::on-observers-changed` rings whenever a session's observers or their
fire counts change; consumers then read `harness::observers::list`.

Entry ids are distinct by construction: `e_fire_<id>_<n>` (the wake message),
`e_obsfired_<id>_<n>` (its record), `e_obsskip_*`, `e_obsexpired_<id>`,
`e_expire_<id>`, `e_condfail_<id>`, `e_claimfail_<id>`. session-manager is
idempotent on entry ids, so a shared id would swallow one of two appends.

### Wake expiry — a wake's death wakes its owner

A session that arms a one-shot wake parks (`terminal: false`) until the wake
delivers. Every binding with an `expires_at` has its own one-shot deadline timer
(no periodic sweep): armed when the binding is reserved, re-armed from the
store by the binding pass, and cancelled whenever the record is deleted. When it
fires it retires the binding and, for a **never-delivered wake**, injects a
final `[notification]` into its destination session plus an `observer_fired`
record (`note: "expired unfired …"`). Retirement deletes the record FIRST: the
delete is the atomic claim against a concurrent delivery (`claim_fire`'s CAS
resolves to `Gone` once the record is missing). `harness::status` exposes the
armed set as `armed_wakes`.

## Restarts, the binding pass, and stored records

Observers belong to the harness connection; the record is the authority. At
boot (`bindings/gc.rs`):

1. **Migrate** — records written before 0.26 name a trigger type and config.
   Each is rewritten through the spec's table (`W::X` becomes `W::on-X`; config
   becomes args for a function that takes args, `$eq` filters on
   `input.<field>` otherwise). One that does not translate — a harness event,
   an unknown shape — is retired and its owner told what to observe instead.
2. **Sweep** — owner-gone bindings are deleted, stale spawn-target records are
   retired loudly, and delivery observers without a record are disposed.
3. **Replay** — every surviving binding's observer is re-created with the same
   id. When the previous connection still holds the id
   (`OBSERVER_ID_CONFLICT`, within the engine's suspension grace), the harness
   disposes the old one through `engine::observers::dispose` and observes
   again.
4. **Catch up** — a one-shot `state::on-changed` wake that never delivered reads
   its watched key and delivers the value it missed.

Within one process the SDK replays observers across reconnects. Every
`engine::on-workers-changed` call reruns the binding pass: orphan disposal,
Compose recovery and re-arming every deadline. The harness also keeps one
standing observer of `compose::on-operation-progress` (terminal events, every
operation), so an operation that finishes inside a wake's activation window
still recovers its wake from Compose's persisted terminal event.

## What stays out of scope

- **Reach.** Observers that other workers create with `iii.observe` bypass all
  of this. Only agent observers route through the harness.
- **Engine-side drops.** An activation the engine drops (`LOOP_GUARD`,
  `OVERFLOW`, `TOO_LARGE`) never reaches the hop and leaves no record.
- **Exactly-once.** iii's delivery is at-most-once. A harness crash between an
  activation and the claim loses the event; what this design achieves is
  at-most-once with durable retirement and no duplicate work.
- **Wake cycles.** The loop guard stops call-observer cycles; a wake starts a
  new turn whose calls begin at depth 0.

## File map

| File | Role |
|---|---|
| `functions/observe.rs` | the `harness::observe` / `harness::unobserve` intercept and the dispatch chokepoint |
| `functions/observer_deliver.rs` | `harness::observer::deliver`, the one delivery hop |
| `functions/observers_list.rs` | `harness::observers::list` / `harness::observers::dispose` |
| `bindings/mod.rs` | the record, observer creation and disposal, armed wakes |
| `bindings/store.rs` | CAS storage, the per-owner capacity index |
| `bindings/gc.rs` | the boot sweep, replay and orphan disposal |
| `bindings/migrate.rs` | stored records from before 0.26 |
| `bindings/expiry.rs` | deadline timers, retirement, wake-lost notices |
| `bindings/compose.rs` | Compose wake recovery and the standing terminal watch |
| `conditions.rs` | the condition contract |
| `subscriptions/fired.rs` | `observer_fired` records |
