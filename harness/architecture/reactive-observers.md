# Reactive observers: how `harness::observe` works

An agent reacts to an event by observing the function that emits it. It calls
`harness::observe`; the harness intercepts the call in the turn loop, records
a durable binding for the calling session, and creates one iii observer for
it. Each call of the observed function then either wakes the session or calls
one plain function.

- **How to use it** — [The payload](#the-payload) is the reference.
- **How the harness delivers** — [observer-bindings.md](observer-bindings.md).
- **Live probes for every branch** — [../tests/observe-use-cases.md](../tests/observe-use-cases.md).

## The payload

```jsonc
{
  "function_id": "state::on-changed",      // required: the observed function
  "namespace": "default",                   // optional; see below
  "args": { },                              // what the function's worker reads, when it takes args
  "filters": { "input.scope": { "$eq": "run-x" } },   // which calls reach the observer
  "label": "explorer-messages",             // optional identity, echoed in the notification
  "action": "new Explorer message received",// optional user-facing text for each delivery
  "once": true,                             // shorthand for lifecycle.once

  //   omit `target`                         → wake me
  //   target: { function_id, payload?, event_into? }  → call a function

  "conditions": [ ],
  "lifecycle":  { }
}
```

Response: `{ observer_id, once, note? }`. `once` is the **effective** value
after the per-function default — trust the echo. `note` is advisory: the
observer was created but the wiring looks suspicious (a key someone already
wrote, an unbounded cron, a function no connected worker hosts yet).

`label` and `action` differ on purpose. The label is a short stable name that
identifies the observer. The action describes what a future delivery means;
`harness::observers::list` returns it before the first delivery and every
`observer_fired` record carries it. It must be non-empty text and never changes
delivery.

### Args and filters

`args` belong to the worker that hosts the function: they set something up for
this observer (a cron expression, a database table, a timer instant). A
function's `engine::functions::info` carries its `args_format`; a function
without one takes no args and refuses them (`INVALID_ARGS`). `filters` belong to
iii: explicit operators on the call's fields (`input.<field>`, `output.<field>`,
`caller.<field>`), evaluated for every call.

| Function | Args | Filters |
|---|---|---|
| `state::on-changed` | none | `input.scope`, `input.key` — omit the key filter and every key in the scope reaches it |
| `cron::on-tick` | `{ expression }` (6-field: sec min hour day month weekday) | — |
| `harness::on-timer` | `{ in_ms }` or `{ at }` (epoch ms) | — |
| `database::on-row-changed` | `{ db, table?, ops? }` | `input.*` row fields |
| `queue::on-message` | `{ topic, max_retries?, backoff_ms?, queue_config?, group? }` | `input.*` |
| `publish` | none | `input.topic` |
| `compose::on-operation-progress` | none | `input.operation_id`, `input.terminal` |
| `observability::on-log` | none | `input.severity_number` |
| `configuration::on-updated` | none | `input.id`, `input.event_type` |
| any worker's `W::on-<event>` | its `args_format` | its event fields |

The namespace is resolved home-first when the payload names none: the harness's
own namespace, then `default`, then Compose's (`III_COMPOSE_NAMESPACE`) — the
first one that hosts the function. When none does yet, the observer waits as
`pending` and activates when the function's worker registers it; the response
`note` says so.

The harness waits briefly for the observer to become active, so a refusal —
`INVALID_ARGS`, `INVALID_FILTER`, `FORBIDDEN`, `NOT_OBSERVABLE`, or the
function's worker rejecting the args (`INVALID_CRON_EXPRESSION`,
`UNKNOWN_DB`) — is the observe call's error and leaves nothing behind.

Two refusals of its own. **Harness functions are not agent-observable**, apart
from `harness::on-timer` and the session's own post-turn validator: a session
notified of its own turn ending would wake itself forever, and child outcomes
belong in the medium the children write, so observe what the work *writes*.
And **a call target is never `harness::*`**: an observer wakes its owner or
calls a plain function, and never starts an agent.

### The two shapes

**Wake me** — omit `target`. Each delivered call arrives as a `[notification]`
message in the OBSERVING session and starts a turn. This is the only shape that
can reach you. The notification carries the observed call's output (for an
event source, the event).

**Call a function** — `target: { function_id, payload?, event_into? }`. The
event is injected into the template at `event_into` (default `/event`).
Deterministic, token-free, no session — and its **result is discarded**; it
cannot reach you. Gated twice: your dispatch policy, and the approval gate,
because a delivered call runs outside any turn and can never prompt a human.

### `conditions`

```jsonc
"conditions": [ { "function_id": "state::barrier", "config": { "id": "run-x", "expect": ["a","b"] } } ]
```

Ordinary iii functions, evaluated in order before each delivery,
short-circuiting on the first skip. Each receives
`{ event, condition_config, binding, context }` and returns
`{ decision: "allow" | "skip", payload?, reason? }`; a bare boolean also works.
A returned `payload` **replaces the event** downstream — that is how a barrier
hands its aggregate to the wake. A condition that errors **skips and records
why**, and the owner gets one actionable notice per binding.

Conditions and filters are different tools. Filters select calls by their
fields and cost nothing; conditions run code, carry per-observer configuration,
and can aggregate across calls.

### `lifecycle`

```jsonc
"lifecycle": { "max_fires": 5, "expires_in_ms": 3600000 }
```

`once` (top level) removes the observer after its first *delivered* call — a
skipped call does not consume it. `max_fires` is a lifetime delivery budget.
`expires_in_ms` is relative to the observe, resolved to an absolute instant
server-side (an absolute `expires_at` is refused by name). Each removes the
observer and records `retired: true`.

The default `once` is **by shape**: a WAKE (no target) is once — it parks the
session; a CALL observer is **standing** — per-event work until removed or its
lifecycle ends; `cron::on-tick` is recurring; `harness::on-timer` is once.
Explicit `once` always wins, and a standing call observer is told so in the
response note.

### `harness::on-timer` — the one-shot deadline

```jsonc
{ "function_id": "harness::on-timer", "args": { "in_ms": 600000 } }   // or { "at": <epoch ms> }
```

Delivers exactly once, on time, then the observer is removed. `in_ms` resolves
to an absolute `at` when you observe, because observer args replay unchanged
after a restart and a replayed countdown must not restart from zero. An `at`
in the past or more than 7 days out is refused. `once: false` is refused;
recurrence is `cron::on-tick`'s job. "Wake me when X happens, or tell me at T
that it did not" is a wake on X plus one timer observer.

A deadline on a **wake** is a real deadline, not only a stop: the binding's own
timer removes it at that instant, and when the wake never delivered it
**injects a `[notification]` into the parked session** naming the watch, the
deadline, and that nothing else will deliver it — so the session un-parks and
runs its own fallback. The same notice is delivered when the console removes
someone's armed wake. `harness::status` lists each armed wake behind
`expects_wake` (function, args, filters, `created_at`, `expires_at`).

### What you do not control

The harness stamps the owner, freezes your dispatch policy onto the binding (a
delivered call is checked against what you could call *when you observed*, not
against a policy that widened later), and creates the observer itself. An
identical observe in the same session returns the standing observer instead of
a twin. Cap: 64 live observers per session. Removal is
`harness::unobserve { id }`, owner-scoped. A sub-agent (a leaf) may arm wakes
for itself but may not create call observers, and every leaf wake carries a
deadline.

### Post-turn validators

The one harness function a session observes with a target is its own post-turn
hook:

```jsonc
{
  "function_id": "harness::hook::on-post-turn",
  "target": { "function_id": "fp::pipe" },
  "args": { "payload": { … }, "result_into": "/value", "retry_prompt": "…", "timeout_ms": 30000 }
}
```

`args.sessions` defaults to the calling session and may name only the session
or children under its `<session>-` prefix. The response's `observer_id` is what
`harness::unobserve` takes.

## Why the call is intercepted

`harness::observe` is a harness tool, not a wrapper over `iii.observe`. The
interception is what gives `target`, `conditions` and `lifecycle` their meaning.

**A delivery needs an owner.** An observer holds what it observes, its target
and a metadata sidecar; it has no owner, no lifecycle the harness counts, and no
frozen capability. The harness keeps those in the binding record and puts only
`{ "__binding": "<id>" }` in the sidecar, so nothing an agent writes is ever
trusted at delivery.

**A direct observer would be an authority escape hatch.** A delivered call runs
with worker authority, outside any turn. Observing straight through the engine
would let an agent aim `shell::exec` — or anything the deployment gates behind
human approval — at a cron tick and have it run unattended forever. The
interception checks the target against the session's dispatch policy and the
approval gate's verdict (side-effect-free) when the agent observes.

**Shaping, ownership and cleanup live here.** An event's shape is never the
target's, so something projects it into the target's payload. Owner-scoped
removal, the per-session cap, the session-deleted sweep, delivered-fire
counting, wake deadlines with owner notices, and the durable record that
re-creates observers after a restart are harness concepts.

Workers observe directly, and should: a worker that wrote its own handler calls
`iii.observe` under its own identity, with no model choosing the target.

## Acting after N arrivals: `state::barrier`

Waiting for a set of arrivals is a condition, an ordinary function anyone can
write or replace. The shipped one is `state::barrier`:

```jsonc
"conditions": [ { "function_id": "state::barrier",
                  "config": { "id": "run-x", "expect": ["a","b","c"], "carry": "/new_value" } } ]
```

It records each arrival idempotently, answers `skip` while incomplete, and
answers `allow` **exactly once** — on the arrival that completes the set —
carrying every arrival's payload. A coordinator watching N producers wakes once,
not N times. `expect` as a named list (rather than a count) tells you *which*
producer never arrived.

## What bounds a runaway

Nothing throttles an observer. A standing observer delivers per matching call;
a cycle routed through a state write re-enters, and every lap is a paid
delivery (measured live: a self-re-arming reaction ran **22 paid turns in 70
seconds** and would not have stopped on its own). The engine's loop guard bounds
call-observer cycles at its observe depth, but a wake starts a new turn whose
calls begin at depth 0. The tools are structural: keep observers acyclic, give
standing observers a `lifecycle`, put deadlines on wakes, and unobserve what you
are done with.

## Bindings that target `harness::spawn`

`harness::spawn` is never a target. A stored binding that still targets it is
retired **loudly** at startup or on its first delivery: the observer is
disposed, the record deleted, and the owner notified with the move — observe
the same function as a wake (omit `target`) and call `harness::spawn` from the
woken turn.
