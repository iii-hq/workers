# `harness::observe` — live use-case prompts

Hand-run probes for agent observers: the two shapes (**wake** and **call**),
the observed function's `args` and `filters`, the harness's `conditions`, and
the guardrails around them. Each case is a prompt you paste into a live
harness session plus the exact thing to check afterwards.

These are *live* probes against a running stack — the deterministic
regression tests live in [`e2e/`](e2e/README.md). Use these when you want to
see how a real model drives the surface, or to reproduce a wiring bug before
turning it into an E2E scenario.

---

## 0. Setup

```bash
III=${III:-iii}                       # or .../iii/target/release/iii
ENGINE=ws://127.0.0.1:49134           # default dev engine
```

Every prompt below is sent the same way. The **dispatch policy is
fail-closed**: a new session with no `options.functions` may call nothing, so
each case lists the grant it needs.

```bash
$III call harness::send --json '{
  "message": "<the prompt>",
  "model": "<a live id from router::models::list>",
  "options": { "functions": { "allow": ["harness::observe", "harness::unobserve", "state::set", "state::get"] } }
}'
```

`harness::send` returns `{session_id, turn_id}`. Keep the `session_id` — every
verification step needs it.

### Verification commands

```bash
# transcript (observe results, notes, observer_fired records)
$III call session::messages --json '{"session_id":"<sid>","limit":80}'

# what a reaction wrote
$III call state::get --json '{"scope":"rtuc-01","key":"result"}'

# the session's observers, as the harness records them
$III call harness::observers::list --json '{"session_id":"<sid>"}'

# the engine's observers of one function; an agent observer's id is its
# binding id, and its target is harness::observer::deliver
$III call engine::observers::list --json '{"function_id":"state::on-changed"}'
```

`harness::observer::deliver` is the only target the harness gives an agent
observer engine-side — the agent's `target.function_id` never reaches the
engine. An observer still listed after a run is a **leak**; see
[§6 Cleanup](#6-cleanup).

### The surface in one table

| Shape | `target` | Each delivered call |
|---|---|---|
| wake | *(omitted)* | a `[notification]` message into the observing session (wakes it) |
| call | `{ function_id, payload?, event_into? }` naming a plain non-harness function | one deterministic, zero-token call |

Facts the cases below lean on (all from `harness/src/functions/observe.rs`):

- `once` defaults by SHAPE: a wake is **true**, a call observer **false**;
  `cron::on-tick` is false, `harness::on-timer` true. Explicit `once` wins;
  trust the echoed value.
- `args` belong to the observed function's worker (a cron expression, a queue
  topic, a timer instant); `filters` belong to iii (`input.<field>` operators
  on each call). A function with no `args_format` refuses args.
- Harness functions are not agent-observable, apart from `harness::on-timer`
  and the session's own post-turn validator. A `harness::*` function is never
  a call target, and an observer never starts an agent.
- A call target must be callable by the **observing session** and must not
  need approval — a delivered call runs outside any turn and can never prompt.
- An identical observe in the same session returns the **standing**
  `observer_id` instead of a twin observer.
- Cap: 64 live observers per session. Nothing throttles a standing observer —
  bound it with `lifecycle` or unobserve it in your teardown.

---

## 1. Wake — the session wakes itself

### UC-01 — one-shot state wake

*Proves:* the default `once: true` on a wake, the armed-wake advisory, and that
a wake actually re-enters the parked session.

Grant: `harness::observe`, `state::get`

```text
Observe exactly once and then stop your turn — do not poll, do not call
anything else.

harness::observe {
  "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "rtuc-01" }, "input.key": { "$eq": "go" } },
  "label": "uc01-wake"
}

Report the observer_id and the `note` field of the result verbatim, then end
the turn. When you are woken by the event, call
state::get {"scope":"rtuc-01","key":"go"}, say "woke with <value>", and stop.
```

Deliver it from the shell:

```bash
$III call state::set --json '{"scope":"rtuc-01","key":"go","value":{"n":1}}'
```

*Expect:* the result has `once: true` and a `note` saying the session stays
parked until a state write on `rtuc-01/go`. One new turn after the write,
ending with `woke with {"n":1}`. A second `state::set` wakes nothing — the
observer was removed on its first delivery.

---

### UC-02 — catch-all note

*Proves:* the state advisory — a state observer with no `input.key` filter
delivers on every write in the scope.

```text
Observe:
harness::observe { "function_id": "state::on-changed",
                   "filters": { "input.scope": { "$eq": "rtuc-02" } }, "once": false }
Then report the `note` field verbatim and stop. Do not observe anything else.
```

*Expect:* the observer is CREATED and the note says it has no `input.key`
filter, so it delivers for EVERY key written in scope `rtuc-02` — the right
shape when producers write one unique key per event. Drop the scope filter too
and the note becomes a warning that it filters on neither `input.scope` nor
`input.key`. Drop `"once": false` (a wake is one-shot by default) and the note
says the FIRST write of ANY key in the scope delivers and removes it.

---

### UC-03 — recurring cron wake

*Proves:* `once` defaults to **false** for `cron::on-tick` — the only
recurring default — and that the cron worker reads the args.

Grant: `harness::observe`, `harness::unobserve`

```text
Observe a cron that ticks every 20 seconds:
harness::observe {
  "function_id": "cron::on-tick",
  "args": { "expression": "*/20 * * * * *" },
  "label": "uc03-tick"
}
Report the returned `once` value, then stop. On the FIRST wake, immediately
call harness::unobserve {"id": "<the observer_id>"}, say "tick 1,
unobserved", and stop. Do not wait for a second tick.
```

*Expect:* `once: false` and an UNBOUNDED-cron warning in the result. One wake,
then the unobserve returns `{"removed": true}`. `harness::observers::list` for
the session is empty afterwards.

> A cron observer with no termination path is the classic runaway: it wakes a
> paid turn forever. Always pair one with an explicit unobserve or a
> `lifecycle` in the same prompt.

---

### UC-04 — idempotent observe

*Proves:* the dedup key — identical arguments return the standing observer
instead of delivering twice.

```text
Call harness::observe TWICE with byte-identical arguments:
{ "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "rtuc-04" }, "input.key": { "$eq": "go" } },
  "label": "uc04" }
Report both observer_ids side by side, then unobserve once and stop.
```

*Expect:* both ids are the **same** string; the single unobserve returns
`{"removed": true}` and `engine::observers::list` shows no leftover.

---

### UC-05 — harness functions are refused

*Proves:* the self-notification guard.

```text
Try: harness::observe { "function_id": "harness::on-turn-completed" }
with NO target. Report the exact error text, then stop.
```

*Expect:* an error — "cannot observe harness function
`harness::on-turn-completed`: turn events are not agent-observable. Watch what
the work WRITES instead …". The same refusal covers every `harness::*`
function except `harness::on-timer`.

---

## 2. Call — deterministic, zero-token reactions

A call observer dispatches one plain function call per delivery. No session,
no model, no tokens. Its result **goes nowhere** — nothing wakes you, so
anything you need to see must be written to state you separately watch.

### UC-06 — mirror an event into another key

*Proves:* the `payload` / `event_into` template.

Grant: `harness::observe`, `harness::unobserve`, `state::set`, `state::get`

```text
Observe with a call target that mirrors a state write into a second key:

harness::observe {
  "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "rtuc-06" }, "input.key": { "$eq": "in" } },
  "target": {
    "function_id": "state::set",
    "payload": { "scope": "rtuc-06", "key": "mirror" },
    "event_into": "/value"
  }
}

Report the observer_id and the note, then STOP. Do not wait for anything.
```

```bash
$III call state::set --json '{"scope":"rtuc-06","key":"in","value":{"n":7}}'
sleep 2
$III call state::get --json '{"scope":"rtuc-06","key":"mirror"}'
```

*Expect:* `rtuc-06/mirror` holds the full state event
(`{type, event_type, scope, key, old_value, new_value}`) — that whole object is
what landed at `/value`. The note says the call observer is STANDING — it
re-runs its call on every matching event until unobserved. No new turn is
created; this costs zero tokens.

---

### UC-07 — cron heartbeat into a database table

*Proves:* a call target on a non-state worker, and that a cron call observer
keeps delivering.

Grant: add `database::execute` to the allow list.

```text
First create the table:
database::execute { "db": "primary", "sql": "CREATE TABLE IF NOT EXISTS rtuc07_beats (at TEXT)" }

Then observe with a call target that inserts one row every 20 seconds:
harness::observe {
  "function_id": "cron::on-tick",
  "args": { "expression": "*/20 * * * * *" },
  "target": {
    "function_id": "database::execute",
    "payload": { "db": "primary", "sql": "INSERT INTO rtuc07_beats (at) VALUES (datetime('now'))" },
    "event_into": "/_tick"
  }
}

Report the observer_id, then STOP. I will remove it myself.
```

```bash
sleep 45
$III call database::query --json '{"db":"primary","sql":"SELECT COUNT(*) c FROM rtuc07_beats"}'
$III call harness::observers::dispose --json '{"observer_id":"<observer id>"}'
```

*Expect:* ~2 rows after 45 s, zero new turns. `event_into: "/_tick"` parks the
tick (`{expression, scheduled_time, actual_time}`) somewhere the target
ignores — without it the event would land at `/event` inside a payload
`database::execute` never asked for.

> `harness::unobserve` is **owner-scoped**: only the observing session can
> remove its observer. From the shell, `harness::observers::dispose` is the
> operator's path; it removes the observer and tells its owner session.

---

### UC-08 — a bad `event_into` pointer fails at observe time

*Proves:* the template is validated before the observer exists, so a pointer
that would be a silent per-event no-op is refused up front.

```text
Try to observe with a malformed pointer:
{ "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "rtuc-08" }, "input.key": { "$eq": "in" } },
  "target": { "function_id": "state::set",
              "payload": { "scope": "rtuc-08", "key": "out" }, "event_into": "value" } }
Report the exact error, then stop.
```

*Expect:* "`event_into` must be a JSON pointer starting with `/`, got
`value`". Nothing is created — `harness::observers::list` is empty.

---

### UC-09 — call targets are policy-gated and `harness::*` is refused

*Proves:* a reaction can only call what the observing session could call
itself, and the harness control plane is never a call target.

Grant: `harness::observe`, `state::get` **only** (no `state::set`).

```text
Run these two observes and report each error verbatim, then stop:

1) { "function_id": "state::on-changed",
     "filters": { "input.scope": { "$eq": "rtuc-09" }, "input.key": { "$eq": "a" } },
     "target": { "function_id": "state::set", "payload": { "scope": "rtuc-09", "key": "b" } } }

2) { "function_id": "state::on-changed",
     "filters": { "input.scope": { "$eq": "rtuc-09" }, "input.key": { "$eq": "a" } },
     "target": { "function_id": "harness::send" } }
```

*Expect:*
1. "`state::set` is not permitted by this session's dispatch policy — a
   reaction can only call functions you can call yourself".
2. "`harness::send` is not a binding target: a binding can wake you (omit
   `target`) or call a plain non-harness function — it never starts an agent.
   Spawn children directly from a turn and observe what they write."

If an approval-gate worker is running, also try a target it marks
`needs_approval` — the observe is refused because a delivered call runs
unattended and can never prompt.

---

## 3. Direct spawn + wake — the parent-owned pipeline

An observer never starts an agent. The fan-out shape is: the OWNER arms a wake
on the medium, spawns leaf workers itself with `harness::spawn`, and the
workers' writes wake it. These cases probe the seams of that shape.

### UC-10 — leaf children write, a barrier-gated wake delivers once

*Proves:* direct spawn, medium-only child output, and `state::barrier` fan-in.

```text
1. Arm a wake gated on two arrivals:
harness::observe {
  "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "rtuc-10" } },
  "conditions": [{ "function_id": "state::barrier",
                   "config": { "id": "rtuc-10-gate", "expect": ["a", "b"] } }]
}
2. Spawn two leaves (one per key):
harness::spawn { "task": "Write {\"ok\":true} to state scope rtuc-10 key a via state::set, then stop.",
                 "session_id": "rtuc-10-leaf-a",
                 "options": { "functions": { "allow": ["state::set"] } } }
   …and the same for key b into rtuc-10-leaf-b.
3. End your turn.
```

*Expect:* the session parks (a wake is once by default), the first write is
recorded as a barrier skip (an `observer_fired` record with
`outcome: "skipped"`), and the second wakes the owner ONCE with both arrivals in
the notification. The spawn results carry `fire_and_forget: true` — the
children's outcomes reach the owner only through the state they write, and no
`[child-failure]` or child result is ever injected.

---

### UC-11 — a leaf cannot orchestrate

*Proves:* the capability wall is policy, not prompt.

```text
harness::spawn { "task": "Try harness::observe on state::on-changed (filters input.scope rtuc-11) with target {\"function_id\": \"state::set\", \"payload\": {\"scope\": \"rtuc-11\", \"key\": \"x\"}}, then try harness::spawn. Report what happened, then stop.",
                 "session_id": "rtuc-11-probe" }
```

*Expect:* both attempts come back `is_error: true`. The observe says "a
sub-agent cannot bind a mechanical call to `state::set` … Omit `target` to
observe as a wake for your own session instead"; the spawn is "not permitted
by this agent's dispatch policy" — the child inherited the parent's allow set
and gained the control-plane deny globs on top. A leaf's own wake (no
`target`) is accepted and carries a deadline. Repeat the spawn with
`"options": { "orchestrator": true }` and the child's call observer is
accepted (unobserve it afterwards).

---

### UC-12 — spawn as a call target is refused

*Proves:* the refusal is loud, with the working shape in the error.

```text
harness::observe { "function_id": "state::on-changed",
                   "filters": { "input.scope": { "$eq": "rtuc-12" }, "input.key": { "$eq": "go" } },
                   "target": { "function_id": "harness::spawn", "payload": { "task": "never" } } }
```

*Expect:* "`harness::spawn` is not a binding target … Spawn children directly
from a turn and observe what they write." Nothing is created.

---

### UC-13 — turn events are not agent-observable, in any shape

*Proves:* child outcomes flow through the medium, never through an observer on
their turns — and the source check runs before the target is looked at.

```text
harness::observe { "function_id": "harness::on-turn-completed",
                   "filters": { "input.session_id": { "$eq": "rtuc-13-x" } },
                   "target": { "function_id": "state::set",
                               "payload": { "scope": "rtuc-13", "key": "raw" } } }
```

*Expect:* refused — "turn events are not agent-observable. Watch what the work
WRITES instead — observe the state keys or database rows the tasks update
(omit `target` for a wake)."

---

## 4. Args, filters and conditions

Three tools select what reaches a target, and they live in three places:

| Tool | Who evaluates it | When it fails |
|---|---|---|
| `args` | the observed function's worker, when the observer is added | the observe call's error (`INVALID_ARGS`, `INVALID_CRON_EXPRESSION`, `UNKNOWN_DB`, …) |
| `filters` | iii, on every call of the observed function | a call that does not match never reaches the delivery hop |
| `conditions` | the harness, at the delivery hop, per delivery | an erroring condition skips the delivery, records why, and tells the owner once |

### UC-14 — a filter that vetoes

*Proves:* filters on the observed call's input decide delivery before the
harness sees anything. A queue message is the call's input, so the message
itself carries the field the filter reads.

Grant: `harness::observe`, `harness::unobserve`, `state::set`

```text
Observe a queue topic with a call target, filtered on the message:
harness::observe {
  "function_id": "queue::on-message",
  "args": { "topic": "rtuc-14" },
  "filters": { "input.gate": { "$eq": true } },
  "target": { "function_id": "state::set",
              "payload": { "scope": "rtuc-14", "key": "seen" }, "event_into": "/value" }
}

Report the observer_id, then STOP.
```

```bash
# gate CLOSED — the filter rejects the message
$III call iii::durable::publish --json '{"topic":"rtuc-14","data":{"gate":false}}'
sleep 2
$III call state::get --json '{"scope":"rtuc-14","key":"seen"}'   # -> not found

# gate OPEN — the message reaches the target
$III call iii::durable::publish --json '{"topic":"rtuc-14","data":{"gate":true}}'
sleep 2
$III call state::get --json '{"scope":"rtuc-14","key":"seen"}'   # -> {"gate":true}
```

*Expect:* nothing after the first publish — and no `observer_fired` record for
it, because the filtered message never reached the delivery hop. The second
message lands in `rtuc-14/seen`.

---

### UC-15 — args the worker refuses fail the observe

*Proves:* args are validated by the worker that hosts the function, and a
refusal leaves nothing behind.

```text
Run these two observes and report each error verbatim, then stop:

1) { "function_id": "cron::on-tick", "args": { "expression": "every 20 seconds" } }
2) { "function_id": "state::on-changed", "args": { "scope": "rtuc-15" } }
```

*Expect:*
1. the cron worker's `INVALID_CRON_EXPRESSION`, as the observe call's error.
2. `INVALID_ARGS`: `state::on-changed` takes no args — select it with
   `filters` on `input.scope` / `input.key`.

`harness::observers::list` is empty afterwards: a refused observer is disposed
and its record deleted.

---

### UC-16 — a condition that errors skips, records, and tells the owner

*Proves:* the condition contract's failure path: a broken condition records
why it skipped and tells the owner.

```text
Arm a wake on rtuc-16/go with a condition that cannot succeed:
harness::observe {
  "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "rtuc-16" }, "input.key": { "$eq": "go" } },
  "conditions": [ { "function_id": "fp::get", "config": { "path": "/missing" } } ]
}
Report the observer_id, then stop and wait to be woken.
```

```bash
$III call state::set --json '{"scope":"rtuc-16","key":"go","value":{"n":1}}'
sleep 5
$III call session::messages --json '{"session_id":"<sid>","limit":20}'
```

*Expect:* no wake from the event. The transcript holds an `observer_fired`
record with `outcome: "skipped"`, the failing condition and its reason, plus ONE
notice to the owner naming the condition — a second write records another skip
but no second notice. The wake stays armed (a skipped delivery does not spend
`once`).

---

### UC-17 — a condition on cron sees only the tick

*Proves:* a condition receives `{ event, condition_config, binding, context }`
and cannot see the world unless the function it names fetches it.

```text
Observe:
harness::observe {
  "function_id": "cron::on-tick",
  "args": { "expression": "*/20 * * * * *" },
  "conditions": [ { "function_id": "fp::get", "config": { "path": "/gate" } } ],
  "target": { "function_id": "state::set", "payload": { "scope": "rtuc-17", "key": "tick" } }
}
Report the observer_id and stop. I will remove it.
```

*Expect:* the target never runs — the condition receives the envelope around
the tick (`{expression, scheduled_time, actual_time}`), finds no gate in it,
and every tick is a recorded skip. Point the condition at a function
that fetches the state it gates on (or drop it) and the ticks land. Remove the
observer with `harness::observers::dispose`.

---

## 5. Limits and guardrails

### UC-18 — observer cap

*Proves:* the 64-per-session cap.

```text
Observe state in a loop: function state::on-changed, filters input.scope
"rtuc-18" and input.key k1 … k70, each one a separate harness::observe call
with "once": false. Stop at the first error and report which key number
failed plus the error text.
```

*Expect:* the 65th observe fails with "observer cap reached (64 active for
this session); unobserve first". Then tear them down (§6) — 64 standing
observers on a shared dev engine is not a state to leave behind.

---

### UC-19 — unobserve is owner-scoped

*Proves:* the ownership check.

```text
Session A: arm any state wake and report the observer_id, then stop.
```

Then, from a **different** session:

```bash
$III call harness::send --json '{
  "message": "call harness::unobserve {\"id\":\"<observer id from session A>\"} and report the exact result",
  "model": "<model>",
  "options": { "functions": { "allow": ["harness::unobserve"] } } }'
```

*Expect:* "observer belongs to a different session". A session removes its own
observers; the console and operators use `harness::observers::dispose`, which
tells the owner.

---

### UC-20 — unobserving something already gone

*Proves:* `{removed: false}` is the honest answer, not an error.

```text
Arm a one-shot state wake, let it deliver, and after the wake call
harness::unobserve with the SAME observer_id. Report the result verbatim and
stop.
```

*Expect:* `{"removed": false}` — the `once` delivery already removed it. Useful
as a probe: `removed: false` on an observer that never delivered means
something else removed it.

---

## 6. Cleanup

After a session of probing:

```bash
# what is still observing? The harness's own records, per owner session:
$III call harness::observers::list --json '{"session_id":"<owner sid>"}'
# the engine's view: every agent observer targets the delivery hop
$III call engine::observers::list --json '{"worker":"harness"}'
```

Anything listed is a live observer that will keep delivering (and, for wakes,
keep paying for turns). Remove each one:

```bash
$III call harness::observers::dispose --json '{"observer_id":"<observer id>"}'
```

Then drop the scratch state. `state::delete` takes a scope **and** a key —
there is no delete-the-whole-scope call, so enumerate with `state::list` first
if you improvised beyond the keys below:

```bash
while read -r scope key; do
  $III call state::delete --json "{\"scope\":\"$scope\",\"key\":\"$key\"}"
done <<'KEYS'
rtuc-01 go
rtuc-06 in
rtuc-06 mirror
rtuc-10 a
rtuc-10 b
rtuc-14 seen
rtuc-16 go
rtuc-17 tick
KEYS

$III call state::list    --json '{"scope":"rtuc-18"}'   # anything left from the cap probe
$III call database::execute --json '{"db":"primary","sql":"DROP TABLE IF EXISTS rtuc07_beats"}'
```

## Writing prompts that do not run away

Every case above follows the same rules, learned the expensive way:

1. **Pin the exact JSON.** Name the observed function, its args, its filters,
   the target and every template field. A model left to infer them invents
   plausible keys the worker rejects — or worse, a filter that never matches.
2. **Say "then stop".** Without it the model keeps working, re-observing, or
   polling `harness::status` in a loop.
3. **Ban polling.** A watcher that polls is not testing the observer.
4. **Give every observer a termination path** in the same prompt:
   `once: true`, a `lifecycle`, or an explicit unobserve the woken turn
   performs.
5. **Verify from outside.** State keys, `harness::observers::list` and
   `engine::observers::list` are ground truth; the transcript is what the
   model *believes* happened.
