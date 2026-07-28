# Trigger bindings — design

How an agent-registered trigger becomes a delivered invocation, and why the
harness owns that path instead of the engine.

Audience: anyone changing `functions/subscribe.rs`, `functions/trigger_deliver.rs`,
`bindings/*`, or the spawn/send seam. The
[integration contract](README.md) is the consumer-facing view; this is the
internal one.

## Why the harness owns this

Two facts about the engine decide the design.

**1. The engine drops the caller's identity on internal invocations.** Its
internal call path — the one hooks, middleware and trigger fires use — invokes
with no session and no principal (`engine/src/engine/mod.rs`,
`handle_invocation(None, None, …)`). A fired trigger therefore arrives at its
target with no idea who registered it, under whose authority it runs, or what
it is allowed to do.

**2. The engine's `Trigger` record holds `{id, trigger_type, function_id,
config, worker_id, metadata}`** — no owner, no lifecycle, no capability. `once`,
ownership and idempotency are not concepts it has.

Everything the harness does around subscriptions exists to compensate: it
stamps owner, policy and lifecycle into `metadata` at registration and rebuilds
them at fire time. That is why the subscription surface is thousands of lines.

The compensation can be *centralised* without changing the engine, because of a
third fact:

**3. The harness is already the only fire hop for agent bindings.** It
intercepts every `engine::register_trigger` an agent issues and never lets one
point anywhere except its own functions. Every agent-registered event in the
system lands on harness code before it reaches a target.

So the harness can own dispatch, conditions, lifecycle and context at that hop —
today, with no engine release. If the engine later grows a trusted
`InvocationContext` and a central `triggers::fire`, the harness deletes this
module and nothing agent-visible changes. The registration shape below is
chosen to make that a no-op.

## The model

```
provider event
      │
      ▼
harness::trigger::deliver          ← the ONE engine-registered target
      │
      ├── resolve binding record (durable)
      ├── built-in gates      (not-self-caused, causal depth, upstream failure)
      ├── declared conditions (typed decision)
      ├── payload projection  (target payload + event_into)
      ├── lifecycle claim     (once / max_fires, atomic)
      └── dispatch target ─── any iii function, including harness::send
                                         │
                                    delivery record
```

One hop, one durable record, one turn-seeding path.

- **A wake** is `target: harness::send` into the owner session.
- **A mechanical reaction** is `target: <any allowed non-harness function>`.

There is no separate notify handler and no separate call-shaping hop — and no
spawn shape at all: a binding never starts an agent. `harness::spawn` is a
direct call the owner makes on its own turn; a stored binding that still
targets it (pre-removal) is retired loudly at startup or on its first fire,
with the owner notified.

## The binding record

Durable in the state worker under `harness_binding/<binding_id>` — the same
pattern as `harness_turn` / `harness_idem` / `harness_queue` (see `src/state.rs`).
Durability is not new scope: the registry it replaces is an in-memory
`Mutex<HashMap>` wiped on every restart, which is why a startup reconciler has
to GC the wreckage and why notify bindings silently stop delivering after a
harness restart.

```jsonc
{
  "id": "sub_<32hex>",              // stable; what unregister takes
  "trigger_id": "<engine binding id>",
  "owner_scope": {
    "session_id": "<registrant>",
    "root_session_id": "<console root>"
  },
  "target": {
    "function_id": "harness::send",
    "payload": { },                 // template; default {}
    "event_into": "/event",         // JSON pointer; default "/event"
    "action": "void"
  },
  "conditions": [
    { "function_id": "state::barrier", "config": { } }
  ],
  "lifecycle": {
    "once": true,
    "max_fires": null,
    "expires_at": null
  },
  "capability": { "allow": [ ], "deny": [ ], "expose": "…" },
  "causation": { "depth": 0, "registered_by_turn": "<turn id>" },
  "created_at": 0,
  "fires": 0
}
```

Durability has a boundary worth naming: the record is as durable as the state
worker's configured adapter. The default kv store is in-memory unless a store
method is configured, so a STATE worker restart drops binding records even
though a HARNESS restart does not. The startup sweep then finds engine bindings
with no record and retires them, which is correct but silent.

`capability` is the registrant's dispatch policy **frozen at registration**. It
is what a fired call is checked against — not the owner's current policy, which
may have widened since. Freezing matches today's behaviour and is the safer of
the two readings.

The engine-side trigger metadata carries **only** `{"__binding": "<id>"}`. No
owner, no policy, no spec. Anything an agent smuggles into metadata is
irrelevant because nothing reads it.

## Registration

`engine::register_trigger` stays the agent-facing call; the harness keeps
intercepting it. The accepted shape:

```jsonc
{
  "trigger_type": "state",
  "config": { "scope": "run-x", "key": "result-a" },
  "target": {
    "function_id": "harness::send",
    "payload": { "message": "…" },
    "event_into": "/event"
  },
  "conditions": [ { "function_id": "conditions::turn-succeeded" } ],
  "lifecycle": { "once": true }
}
```

Backward-compatible shorthands, all still accepted:

| Shorthand | Means |
|---|---|
| `function_id` omitted | `target.function_id = "harness::send"` into the owner session (a wake) |
| `function_id: "<non-harness fn>"` + `metadata: {payload, event_into}` | `target` built from the metadata template |
| `once: <bool>` at top level | `lifecycle.once` |

The shorthands are what today's prompts and every live binding use, so they are
not deprecated — they are sugar over the same record.

`condition_function_id` inside `config` is **rejected** at registration, with a
pointer to `conditions`. The engine's own condition contract only vetoes on a
bare `false` and treats an erroring condition as "skip", so a typo'd id silently
starves the binding forever with no signal anywhere. Agents get the typed
contract instead.

## Delivery

`harness::trigger::deliver` runs this order, and the order is load-bearing:

1. **Resolve** the binding by `__binding`. Unknown id → record a dropped
   delivery and return (a stale engine binding from before a rewrite).
2. **Built-in gates**, applied unconditionally and *before* anything the agent
   declared:
   - `not-self-caused` — the binding never fires for an event its own delivery
     produced.
   - `causal-depth` — reactive chains stop at `MAX_REACTIVE_DEPTH`.
   - `upstream-failure` — a failed/cancelled upstream turn does not start a
     reaction unless the binding opted in.
   These are engine-of-record safety, not policy: an agent cannot disable them
   by omitting a condition.
3. **Declared conditions**, in order, short-circuiting on the first `skip`.
4. **Claim** — for a bounded lifecycle (`once`, `max_fires`), take the claim
   *before* dispatching, atomically. Order matters: claim → dispatch → retire.
   Claiming after dispatch double-fires on a crash; retiring before dispatch
   loses the fire.
5. **Project** the payload: the target's template with the event injected at
   `event_into`.
6. **Dispatch**, checked against the binding's frozen `capability`.
7. **Record** the delivery.

## The condition contract

A condition is an ordinary iii function. It receives:

```jsonc
{ "event": { }, "condition_config": { }, "binding": { "id": "…", "fires": 0 }, "context": { } }
```

and returns:

```jsonc
{ "decision": "allow" | "skip", "payload": { }, "reason": "…" }
```

- `allow` — continue; a returned `payload` replaces the event for later steps.
- `skip` — do not deliver; a recurring binding stays armed.
- An **error** or an unparseable result → `skip`, and the reason is recorded on
  the delivery. Unlike the engine's version, this is visible.

`defer` (reschedule, for throttling and coalescing) is deliberately **not** in
v1: it requires a durable scheduler, and nothing has demanded it yet. The
decision enum is open for it.

Joins, claims and rate limits are ordinary functions, not engine concepts — a
barrier is a condition that records an arrival idempotently, answers `skip`
until the set is complete, then answers `allow` with the accumulated results.

## Observability

Every delivery attempt appends a model-invisible `trigger_fired` entry to the
owner session's transcript — including the ones that did **not** deliver, with
the deciding gate or condition and its reason. Today only real fires are
recorded, which is why a mis-wired binding is indistinguishable from a quiet
one. "Why did this never fire?" must be answerable from the owner's timeline.

### Wake expiry — a wake's death wakes its owner

A session that arms a one-shot wake parks (`terminal: false`) until the wake
fires. Before the expiry sweep, that park had no exit but the fire itself:
`expires_at` was only consulted at claim time, so a wake nobody ever fired
ignored its own deadline, and `session_expects_wake` counted exhausted
bindings — the session stayed non-terminal forever (the reactive discovery
run's coordinator, parked beside a finished operation).

Now a periodic sweep (`III_HARNESS_EXPIRY_SWEEP_MS`, default 30s) retires
every lifecycle-spent binding, and for a **never-fired wake** delivers a
final `[notification]` into its destination session plus a `trigger_fired`
record (`note: "expired unfired …"`, entry ids `e_expire_<id>` /
`e_trigexpired_<id>` — distinct from every real fire's pair). The sweep
deletes the record FIRST: the delete is the atomic claim against a concurrent
real fire (`claim_fire`'s CAS resolves to `Gone` once the record is missing),
so the owner is never told "nothing is coming" by one hand while the other
delivers. The same notice covers a lineage teardown that unregisters someone
else's armed wake. `is_armed_wake` discounts exhausted bindings, so the woken
turn completes terminal; `harness::status` exposes the armed set as
`armed_wakes` (watch, config, created_at, expires_at). Pinned by E2E-012.

## What stays out of scope

Harness-only cannot deliver:

- **Reach.** Bindings that other workers register directly with the engine
  bypass all of this. Only agent bindings route through the harness.
- **Losing a delivery under burst — found and fixed, in two parts.** Measured
  with `database::row-changed`, one transaction committing three writes: 4
  claims produced 3 wakes, then (after the first fix) 3 claims produced 2
  records and 1 wake. Part one was the claim: a `get` then a `put`, so
  simultaneous fires collided on one ordinal — `claim_fire` now
  compare-and-sets the record (`state::compare-and-set`), and a losing racer
  recomputes against what is actually stored. Part two was subtler: the wake
  message and its delivery record shared one entry id, and session-manager is
  idempotent on entry ids, so exactly one of the two appends survived each
  fire — and WHICH one depended on timing. An idle session appended the wake
  first and lost the record; a running turn PARKED the wake while the record
  claimed the id, and the drained wake was then deduped as a replay. The
  record now derives its own id (`e_trigfired_*` vs `e_fire_*`). Re-run after
  both fixes: 3 claims, 3 records, 3 notifications.

- **Exactly-once.** The engine fires and forgets — `tokio::spawn`, result
  discarded, no retry, no outbox. A harness crash between the engine's fire and
  the claim loses the event. What is achievable here is at-most-once with
  durable retirement and no duplicate work.
- **Multi-hop context.** Authority is enforced at the dispatch decision. Once a
  target is invoked, the engine carries no principal onward.

Each needs the engine changes (trusted `InvocationContext`, central
`triggers::fire`, delivery outbox). This design is the forward migration for
them, not a detour: the record and the condition contract are the shapes the
engine would adopt.

## File map

| Before | Lines | After | Lines |
|---|---|---|---|
| `functions/trigger_call.rs` | 453 | deleted — projection lives in deliver | 0 |
| `subscriptions/notify_agent.rs` | 307 | deleted — a wake is `harness::send` | 0 |
| `subscriptions/reconcile.rs` | 345 | deleted — `bindings/gc.rs` | 148 |
| `subscriptions/registry.rs` | 554 | lineage edge only | 129 |
| `subscriptions/fired.rs` | 171 | kept; records non-deliveries too | 171 |
| `subagent.rs` | 651 | child preset; seeding moved to `send` | 545 |
| `functions/spawn.rs` | 1034 | request types + direct call | 331 |
| `functions/subscribe.rs` | 1477 | one interception path | 1039 |
| — | — | `bindings/{mod,store,gc}.rs` | 620 |
| — | — | `functions/trigger_deliver.rs` | 456 |
| — | — | `conditions.rs` | 306 |
| **total** | **4992** | | **3745** |

Prompts, over the same changes: the fallback and the eight provider identity
prompts first lost the `harness::react` and join cookbook, then the whole
wire/spawn/stop orchestration doctrine — what remains is tool guidance, with
the opt-in process guidance moved to `harness/skills/orchestration.md`
(`harness/tests/prompts.rs` enforces both halves repo-wide).

## Migration

Engine bindings are durable and survive restarts, so bindings registered
before these changes still point at `harness::notify_agent`,
`harness::trigger-call`, or `harness::spawn` — and the durable store may hold
records whose target is `harness::spawn`. None are adapted: `bindings/gc.rs`
unregisters the engine-side leftovers at startup, and a stored spawn-target
record is retired loudly (engine unregister + record delete + an owner
notification naming the register-a-wake-and-spawn-directly migration) at
startup or on its first fire, whichever comes first.
