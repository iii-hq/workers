# Reactive triggers: what changed, and how `engine::register_trigger` works now

Three consecutive changes replaced the harness's reactive surface. If you last
looked at it when `harness::react` existed — or when a binding could still
target `harness::spawn` — what you knew is stale, but the call you make has
barely moved.

- **What changed and why** — below.
- **How to use it** — [The payload](#the-payload) is the reference.
- **Why the harness owns dispatch at all** — [trigger-bindings.md](trigger-bindings.md).
- **Live probes for every branch of it** — [../tests/register-trigger-use-cases.md](../tests/register-trigger-use-cases.md).

## Change 1 — `harness::react` is gone

`harness::react` was a second control plane: a function agents could not call,
bound as a trigger target, carrying a spec in `metadata` that it interpreted at
fire time. It implemented payload mapping, conditions, loop breaking, a
fire-rate gate with coalescing, joins, `once`, cleanup, observability and
dispatch — about 2,450 lines.

It is deleted. An event now points at what it actually does (the spawn row
lasted one more change — see
[spawn bindings were removed](#spawn-bindings-were-removed)):

| Before | Now |
|---|---|
| `function_id: "harness::react"`, `metadata: { task, … }` | **removed** — register a wake, spawn directly from the woken turn |
| `metadata: { call: { function_id, payload } }` | `function_id: "<the function>"`, `metadata: { payload, event_into }` |
| `metadata: { join: { id, expect, key } }` | **removed** — see [`state::barrier`](#acting-after-n-arrivals-statebarrier) |
| fire-rate gate, coalescing, `__coalesced_fires` | **removed** — see [what bounds a runaway](#what-bounds-a-runaway) |

Anything still naming `harness::react` in a prompt or a config is dead wiring;
a repo-wide test (`harness/tests/prompts.rs`) now fails if a shipped prompt
mentions it. That test exists because the removal originally missed all eight
provider identity prompts — and the provider's prompt, not the harness's
fallback, is what an agent actually reads. Agents kept reaching for `react`
because they were still being told to.

## Change 2 — one durable binding, one delivery hop

Removing `react` left three fire handlers (`notify_agent`, `trigger-call`,
`harness::spawn`), three metadata shapes, and an in-memory registry that a
restart wiped. Those collapsed into one record and one hop (change 3 then
removed the spawn dispatch branch entirely — a binding wakes its owner or
calls a plain function, never starts an agent):

```
provider event
      │
      ▼
harness::trigger::deliver        ← the ONE target the engine ever sees
      │
      ├── resolve the durable binding record
      ├── stale-target check  (a pre-removal spawn record retires loudly)
      ├── declared conditions (typed decision)
      ├── claim               (once / max_fires, before dispatch)
      ├── project             (payload template + event_into)
      └── dispatch            (harness::send for a wake, or the bound function)
                                          │
                                    delivery record
```

The engine-side metadata is now exactly `{"__binding": "<id>"}`. Owner, target,
conditions, lifecycle, frozen capability and fire count live in a durable
record; nothing an agent writes into metadata is read back.

Three consequences worth knowing:

- **Bindings survive a harness restart.** They previously did not: spawn
  bindings kept firing with no bookkeeping, notify bindings stopped delivering
  entirely, and a startup reconciler existed only to clean up after that.
- **Non-deliveries are recorded.** Every skip — a gate, a condition, a spent
  lifecycle — appends a model-invisible `trigger_fired` entry to the owner's
  transcript with the reason. "Why did this never fire?" is answerable.
- **Durability is bounded by the state worker.** The record lives in the state
  worker, so it is as durable as that worker's adapter. The dev default is
  in-memory: a *state* restart drops binding records even though a *harness*
  restart does not.

## The payload

```jsonc
{
  "trigger_type": "state",       // required
  "config":  { },                // that type's own filters, passed verbatim
  "label":   "…",                // optional, echoed in the notification text
  "once":    true,               // shorthand for lifecycle.once

  //   omit the target entirely            → wake me
  //   function_id + metadata (call shape)  → shorthand
  //   target: { … }                        → long form

  "conditions": [ … ],
  "lifecycle":  { … }
}
```

Response: `{ subscription_id, once, note? }`. `once` is the **effective** value
after the per-type default — trust the echo. `note` is advisory: the
registration succeeded but the wiring looks suspicious.

### `trigger_type` → `config`

| Type | Config |
|---|---|
| `state` | `scope?`, `key?` — omit `key` and it fires for every key in the scope |
| `cron` | `expression` (6-field: sec min hour day month weekday) |
| `durable:subscriber` | `topic`, `queue_config?` — the queue. `queue` is the provider *package*, not a type |
| `subscribe` | `topic` — pubsub |
| `stream` | `stream_name?`, `group_id?`, `item_id?` |
| `stream:join` / `stream:leave` | `stream_name?` |
| `log` | `level?` |
| `trace` | `service_name?`, `status?` |
| `configuration` | `configuration_id?`, `event_types?` |
| `http` | `api_path`, `http_method?` |
| `harness::turn-started` / `harness::turn-completed` | worker-bindable only (direct engine registrations); the agent path refuses them |
| worker-defined | whatever that worker accepts — `engine::triggers::info { id }` |

Two refusals. **Turn-event types are not agent-bindable in any shape** — a
session notified of its own turn ending would wake itself forever, and child
outcomes belong in the medium the children write, so watch what the work
*writes* instead. And **`condition_function_id` inside `config`** is rejected,
for the reasons below.

### Why `condition_function_id` is refused

It is a real engine feature: the builtin providers (`state`, `cron`,
`durable:subscriber`, `subscribe`, `stream`, `http`, `configuration`) accept it
in `config` and call that function before dispatching. Four things compound
until it is unusable.

**Only a literal `false` vetoes.** From `engine/src/condition.rs`:

```rust
Ok(Some(result)) => Ok(result.as_bool() != Some(false)),
Ok(None)         => { warn!("Condition function returned no result"); Ok(true) }
```

An object passes. A string passes. A number passes. `null` passes. No result at
all passes. Unless your function returns the bare JSON literal `false`, the gate
is decorative — and essentially no real function returns that.

**The condition receives the raw event as its ENTIRE payload.** There is nowhere
to put per-binding configuration. `state::get` needs `{scope, key}`; `fp::when`
needs `{value, op, to}`; neither gets them, because the payload *is* the event.
One function cannot serve two bindings that want different thresholds.

**Which makes the error path the default outcome — and errors are silent.**
Since the event shape rarely matches a real function's request schema, the call
errors, and every provider call site treats that as skip-and-continue:

```rust
Ok(false) => { debug!("Condition check failed, skipping handler"); continue; }
Err(err)  => { has_error = true; error!(…); continue; }
```

The binding stays registered, `engine::registered-triggers::list` shows it, it
looks healthy — and it never fires, forever, with nothing surfaced to whoever
registered it. A typo'd function id is indistinguishable from a correctly wired
one that simply has not triggered yet.

**Worker-defined trigger types ignore it entirely.** Only builtin providers
consult it (seven duplicated call sites), so on `harness::turn-completed` or
`storage::object-created` it rides along as inert metadata. There is also no
authorization: the engine calls whatever id you name, with engine authority, no
policy check.

The replacement is [`conditions`](#conditions), evaluated by the harness at the
delivery hop:

| Engine's contract | `conditions` |
|---|---|
| Only bare `false` vetoes | Typed `{decision, payload?, reason?}`; a bare boolean still accepted as sugar |
| Payload is the raw event | Envelope `{event, condition_config, binding, context}` — parameterized and reusable |
| Error → silent permanent skip | Error → skip **with the reason recorded** on the delivery record |
| Non-bool return → pass | Unparseable answer → error-skip; a condition that cannot say what it decided has not decided anything |
| One condition | A list, in order, short-circuiting on the first skip |
| Builtin providers only | Every trigger type, because the harness evaluates it rather than the provider |

Two consequences. An `allow` may return a `payload` that **replaces the event**
downstream, which is what makes fan-in expressible as an ordinary function —
`state::barrier` hands its accumulated results to the wake that way. And the
built-in gates run *before* anything you declare, so omitting a condition cannot
switch them off.

### The two shapes

**Wake me** — omit the target. The event arrives as a message in the OWNING
session and starts a turn. This is the only shape that can reach you.

**A function call** — `function_id: "<any function your policy allows>"`,
`metadata: { payload?, event_into? }`. The event is injected into your template
at `event_into` (default `/event`). Deterministic, token-free, no session — and
its **result is discarded**; it cannot reach you. Gated twice: your dispatch
policy, and the approval gate, because a fired call runs outside any turn and
can never prompt a human. `harness::*` targets are refused — a binding never
starts an agent (`harness::spawn` is a direct call the owner makes on its own
turn) and never re-enters the harness control plane.

**Long form** — `target: { function_id, payload?, event_into? }`. The
shorthands are sugar over exactly this.

### `conditions`

```jsonc
"conditions": [ { "function_id": "state::barrier", "config": { "id": "run-x", "expect": ["a","b"] } } ]
```

Ordinary iii functions, evaluated in order, short-circuiting on the first skip.
Each returns `{ decision: "allow" | "skip", payload?, reason? }`; a bare boolean
also works. A returned `payload` **replaces the event** downstream — that is how
a barrier hands its aggregate to the wake. A condition that errors **skips and
records why**, rather than passing an ungated reaction through or stalling
silently.

### `lifecycle`

```jsonc
"lifecycle": { "max_fires": 5, "expires_at": 1785029761878 }
```

`once` (top level) retires the binding after its first *delivered* fire — a
skipped fire does not consume it. `max_fires` is a lifetime delivery budget.
`expires_at` is epoch-ms. Each retires the binding on both sides and records
`retired: true`. Default `once` is **by shape**: a WAKE (no target function)
is once — it parks the session; a CALL binding is **standing** — per-event
work until unregistered or its lifecycle ends (three of five discovery runs
registered intended-standing handlers, got the old always-once default, and
silently lost every event after the first); `cron` is recurring; `timer` is
once. Explicit `once` always wins, and a defaulted-standing call binding is
told so at registration.

### `timer` — the one-shot deadline

```jsonc
{ "trigger_type": "timer", "config": { "in_ms": 600000 } }   // or { "at": <epoch ms> }
```

Fires exactly once, exactly on time, then retires. Provided by the harness
itself (registered with the engine, so it appears in `engine::triggers::list`
and re-arms from the engine's registration replay after a restart). `in_ms`
resolves to an absolute `at` at registration — a replayed countdown must not
restart from zero. `once: false` is refused; recurrence is cron's job. This
is the deadline primitive every run used to encode as a cron boundary plus a
remembered `once: true`: "wake me when X happens, or tell me at T that it
did not" is a wake binding plus one timer registration.

`expires_at` on a **wake** is a real deadline, not just a stop: a periodic
sweep retires any binding whose lifecycle is spent, and when the retiree is a
never-fired wake it **injects a `[notification]` into the parked session** —
naming the watch, the deadline, and that nothing else will fire it — so the
session un-parks and runs its own fallback instead of sleeping forever. The
same notice is delivered when a lineage session unregisters someone else's
armed wake. "Wake me when X happens, or tell me at T that it didn't" is
therefore ONE registration; no cron-boundary deadline hack needed.
`harness::status` lists each armed wake behind `expects_wake` (watch, config,
`created_at`, `expires_at`) so a parked session is inspectable from outside.

### What you do not control

The harness stamps the owner, freezes the registrant's dispatch policy onto the
binding (a fired call is checked against what you could call *when you
registered*, not against a policy that widened later), and records causation.
Identical re-registration in the same session returns the standing binding
instead of a twin. Cap: 64 live bindings per session. Teardown is
`engine::unregister_trigger { id }`, owner-scoped — your session, or a reaction
in its lineage cleaning up the run.

## Why the call is intercepted

There is no wrapper function. You call `engine::register_trigger` — the
engine's own id, verbatim. The harness *intercepts* it at its dispatch
chokepoint rather than forwarding it, and that interception is the only reason
`target`, `conditions` and `lifecycle` mean anything.

Three things stop working without it, worst first.

**A fired trigger arrives with no identity.** The engine's internal invocation
path — shared by hooks, middleware and trigger fires — invokes with
`handle_invocation(None, None, …)`: no session, no principal. Its `Trigger`
record is `{id, trigger_type, function_id, config, worker_id, metadata}`: no
owner, no lifecycle, no capability. So at fire time nothing knows who
registered the binding, under whose authority it runs, or how deep in a chain
it is. Someone has to carry that, and if the *agent* carries it in `metadata`,
the agent can forge it.

**Direct registration would be an authority escape hatch.** A trigger-fired
call runs with worker authority, outside any turn. Registering straight to the
engine would let an agent bind `shell::exec` — or anything the deployment gates
behind human approval — to a cron and have it run unattended forever, with no
turn to prompt from. The interception is where both gates live: the target is
checked against the *registrant's* dispatch policy, and against the approval
gate (side-effect-free), so anything a human would normally be asked about is
refused at registration instead of running silently later. Neither check exists
on the raw engine path, because the engine has no notion of a session's policy.

**Shaping, ownership and cleanup have nowhere else to live.** An event's schema
is never the target's, so something must project it into the target's payload.
Owner-scoped unregister, the per-session cap, the session-deleted sweep, and
`once` / `max_fires` / `expires_at` are not engine concepts.

### Workers register directly, and should

This is not a prohibition on the engine path. A worker registering its own
handler (`iii.registerTrigger(...)`) goes straight to the engine, and that is
correct: it wrote the handler, it knows the event shape, it runs under its own
worker identity, and no model is choosing the target. `state`, `cron` and
`storage::object-created` all work that way. The interception is specifically
for **agent**-registered bindings, where the target is model-chosen and the
caller identity is exactly what the engine drops.

### What would let us delete it

Three engine changes, and the interception mostly evaporates:

1. A trusted `InvocationContext` — principal, owner scope, causation/depth, an
   opaque capability handle — generated by the engine and propagated through
   sync, void, queue and trigger paths, unforgeable from the payload.
2. Owner and lifecycle on the `Trigger` itself.
3. A central `engine::triggers::fire` that evaluates conditions and commits the
   delivery atomically, instead of each provider invoking its target directly.

That is why the payload is shaped `{target, conditions, lifecycle}` rather than
something harness-specific: it is the form the engine would plausibly adopt, so
if that work lands, the harness deletes its dispatch hop and its binding store
and **nothing agent-visible changes** — same call, same fields, same semantics.
The reasoning behind the split is in [trigger-bindings.md](trigger-bindings.md).

## Acting after N arrivals: `state::barrier`

`react`'s `join` accumulated predecessor results inside the harness. It is
gone; waiting for a set of arrivals is now a condition, which means it is an
ordinary function anyone can write or replace. The shipped one is
`state::barrier`:

```jsonc
"conditions": [ { "function_id": "state::barrier",
                  "config": { "id": "run-x", "expect": ["a","b","c"], "carry": "/new_value" } } ]
```

It records each arrival idempotently, answers `skip` while incomplete, and
answers `allow` **exactly once** — on the arrival that completes the set —
carrying every arrival's payload. A coordinator watching N producers wakes once,
not N times. `expect` as a named list (rather than a count) is what tells you
*which* producer never arrived.

## What bounds a runaway

Nothing throttles a binding, and the removed reaction machinery took its
built-in gates with it — they existed to bound trigger-spawned agent chains,
and a binding no longer starts an agent. A standing binding fires per matching
event; a cycle routed through a state write re-enters unguarded, and every lap
is a paid delivery (measured live in the react era: a self-re-arming reaction
ran **22 paid turns in 70 seconds** and would not have stopped on its own).
The tools are structural: keep bindings acyclic, give standing bindings a
`lifecycle`, put deadlines on wakes, and unregister what you no longer need.

## Spawn bindings were removed

A binding no longer starts an agent — `harness::spawn` is refused as a target
at registration, at the turn-event handler, and (for records that predate the
removal) at delivery resolve, where the stale binding is retired **loudly**:
the engine trigger is unregistered, the record deleted, and the owner session
notified with the migration. The migration is one move:

> Where you bound `function_id: "harness::spawn"`, register a **wake** on the
> same trigger (omit `function_id`) and call `harness::spawn` yourself from
> the woken turn — or spawn the worker up front and let the binding wake you
> when its output lands in the medium. The parent owns the control plane;
> children write the medium; the medium wakes the parent.

Pre-existing engine bindings that still point at `harness::react`,
`harness::notify_agent`, `harness::trigger-call`, or `harness::spawn` are
unregistered by the startup sweep rather than adapted — a shim would have to
trust the very metadata this model exists to stop reading.
