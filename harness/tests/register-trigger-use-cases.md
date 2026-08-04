# `engine::register_trigger` — live use-case prompts

Hand-run probes for the harness's subscription surface: the three binding
modes (**notify**, **call**, **spawn**), the engine's **trigger condition**
(`condition_function_id`), and the guardrails around them. Each case is a
prompt you paste into a live harness session plus the exact thing to check
afterwards.

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
$III trigger harness::send --json '{
  "message": "<the prompt>",
  "model": "<a live id from router::models::list>",
  "options": { "functions": { "allow": ["engine::register_trigger", "engine::unregister_trigger", "state::set", "state::get"] } }
}'
```

`harness::send` returns `{session_id, turn_id}`. Keep the `session_id` — every
verification step needs it.

### Verification commands

```bash
# transcript (registration results, notes, fired-event notices)
$III trigger session::messages --json '{"session_id":"<sid>","limit":80}'

# what a reaction wrote
$III trigger state::get --json '{"scope":"rtuc-01","key":"result"}'

# live engine bindings, by the harness-internal target they route through
$III trigger engine::registered-triggers::list --json '{"function_id":"harness::notify_agent"}'
$III trigger engine::registered-triggers::list --json '{"function_id":"harness::trigger-call"}'
$III trigger engine::registered-triggers::list --json '{"function_id":"harness::spawn"}'
```

Those three ids are the only targets the harness ever binds engine-side — the
agent's `function_id` never reaches the engine directly. A binding still
listed after a run is a **leak**; see [§6 Cleanup](#6-cleanup).

### The surface in one table

| Mode | `function_id` | `metadata` | Fires |
|---|---|---|---|
| notify | *(omitted)* | — | a message into the registering session (wakes it) |
| call | any ordinary non-harness `worker::fn` | `{ payload?, event_into? }` | one deterministic, zero-token call |

Facts the cases below lean on (all from `harness/src/functions/subscribe.rs`):

- `once` defaults by SHAPE: a wake is **true**, a call binding **false**;
  `cron` is false, `timer` true. Explicit `once` wins; trust the echoed value.
- Every `harness::*` trigger type is refused for agents, in every shape —
  turn events included. A binding never starts an agent: `harness::spawn` is
  refused as a target, with the migration named in the error.
- A call target must be callable by the **registrant** and must not need
  approval — a trigger-fired call runs outside any turn and can never prompt.
- Re-registering identical arguments in the same session returns the
  **existing** `subscription_id` instead of a twin binding.
- Cap: 64 active bindings per session. Nothing throttles a standing binding —
  bound it with `lifecycle` or unregister it in your teardown.

---

## 1. Notify mode — the session wakes itself

### UC-01 — one-shot state wake

*Proves:* the default `once: true` on a `state` binding, the armed-wake
advisory, and that a wake actually re-enters the parked session.

Grant: `engine::register_trigger`, `state::get`

```text
Register exactly one subscription and then stop your turn — do not poll, do
not call anything else.

engine::register_trigger {
  "trigger_type": "state",
  "config": { "scope": "rtuc-01", "key": "go" },
  "label": "uc01-wake"
}

Report the subscription_id and the `note` field of the result verbatim, then
end the turn. When you are woken by the event, call
state::get {"scope":"rtuc-01","key":"go"}, say "woke with <value>", and stop.
```

Fire it from the shell:

```bash
$III trigger state::set --json '{"scope":"rtuc-01","key":"go","value":{"n":1}}'
```

*Expect:* result has `once: true` and a `note` warning that the session stays
parked until `rtuc-01/go` is written. One new turn after the write, ending
with `woke with {"n":1}`. A second `state::set` wakes nothing — the binding
retired itself.

---

### UC-02 — catch-all warning

*Proves:* `state_catchall_advisory` — a keyless `state` binding fires on every
write in the scope.

```text
Register:
engine::register_trigger { "trigger_type": "state", "config": { "scope": "rtuc-02" } }
Then report the `note` field verbatim and stop. Do not register anything else.
```

*Expect:* registration SUCCEEDS and the note warns the binding has no `key`
filter and fires for every key written in scope `rtuc-02`. Drop the `scope`
too and the warning escalates to "EVERY state write in EVERY scope".

---

### UC-03 — recurring cron notify

*Proves:* `once` defaults to **false** for `cron` — the only recurring default.

```text
Register a cron subscription that ticks every 20 seconds:
engine::register_trigger {
  "trigger_type": "cron",
  "config": { "expression": "*/20 * * * * *" },
  "label": "uc03-tick"
}
Report the returned `once` value, then stop. On the FIRST wake, immediately
call engine::unregister_trigger {"id": "<the subscription_id>"} , say "tick 1,
unregistered", and stop. Do not wait for a second tick.
```

*Expect:* `once: false` in the registration result. One wake, then the
unregister returns `{"removed": true}`. Verify no `harness::notify_agent`
binding survives.

> A cron binding with no termination path is the classic runaway: it wakes a
> paid turn forever. Always pair one with an explicit unregister in the same
> prompt.

---

### UC-04 — idempotent re-registration

*Proves:* `registration_dedup_key` — identical arguments return the standing
subscription instead of double-delivering.

```text
Call engine::register_trigger TWICE with byte-identical arguments:
{ "trigger_type": "state", "config": { "scope": "rtuc-04", "key": "go" }, "label": "uc04" }
Report both subscription_ids side by side, then unregister once and stop.
```

*Expect:* both ids are the **same** string; the single unregister returns
`{"removed": true}` and `engine::registered-triggers::list` shows no leftover.

---

### UC-05 — harness-internal trigger type is refused

*Proves:* the self-notification guard on the notify path.

```text
Try to register: { "trigger_type": "harness::turn-completed", "config": {} }
with NO function_id. Report the exact error text, then stop.
```

*Expect:* an error — `cannot bind harness-internal trigger type
'harness::turn-completed' (self-notification guard)`. Turn events are
bindable, but only through `harness::spawn` (UC-13) or a call target.

---

## 2. Call mode — deterministic, zero-token reactions

A call reaction dispatches one plain function call. No session, no model, no
tokens. Its result **goes nowhere** — nothing wakes you, so anything you need
to see must be written to state you separately watch.

### UC-06 — mirror an event into another key

*Proves:* the `payload` / `event_into` template and the default `/event`
injection point.

Grant: `engine::register_trigger`, `engine::unregister_trigger`, `state::set`,
`state::get`

```text
Register a call reaction that mirrors a state write into a second key:

engine::register_trigger {
  "trigger_type": "state",
  "config": { "scope": "rtuc-06", "key": "in" },
  "function_id": "state::set",
  "once": false,
  "metadata": {
    "payload": { "scope": "rtuc-06", "key": "mirror" },
    "event_into": "/value"
  }
}

Report the subscription_id and the note, then STOP. Do not wait for anything.
```

```bash
$III trigger state::set  --json '{"scope":"rtuc-06","key":"in","value":{"n":7}}'
sleep 2
$III trigger state::get  --json '{"scope":"rtuc-06","key":"mirror"}'
```

*Expect:* `rtuc-06/mirror` holds the full state event
(`{type, event_type, scope, key, old_value, new_value}`) — that whole object is
what landed at `/value`. The note reminds you a call reaction cannot wake the
session. No new turn is created; this costs zero tokens.

---

### UC-07 — cron heartbeat into a database table

*Proves:* call mode against a non-state worker, and that `once: false` on cron
keeps firing.

Grant: add `database::execute` to the allow list.

```text
First create the table:
database::execute { "db": "primary", "sql": "CREATE TABLE IF NOT EXISTS rtuc07_beats (at TEXT)" }

Then register a call reaction that inserts one row every 20 seconds:
engine::register_trigger {
  "trigger_type": "cron",
  "config": { "expression": "*/20 * * * * *" },
  "function_id": "database::execute",
  "metadata": {
    "payload": { "db": "primary", "sql": "INSERT INTO rtuc07_beats (at) VALUES (datetime('now'))" },
    "event_into": "/_tick"
  }
}

Report the subscription_id, then STOP. I will unregister it myself.
```

```bash
sleep 45
$III trigger database::query --json '{"db":"primary","sql":"SELECT COUNT(*) c FROM rtuc07_beats"}'
$III trigger engine::unregister_trigger --json '{"id":"<sub id>"}'   # see note
```

*Expect:* ~2 rows after 45 s, zero new turns. `event_into: "/_tick"` parks the
cron payload (`{job_id, scheduled_time, actual_time}`) somewhere the target
ignores — without it the event would overwrite `/event` inside a payload
`database::execute` never asked for.

> `engine::unregister_trigger` is **owner-scoped**: only the registering
> session (or a session in its reaction lineage) can tear a subscription down.
> From the shell you are nobody, so send the unregister as a steer into the
> same session instead: `harness::send {"session_id":"<sid>","message":"call
> engine::unregister_trigger {\"id\":\"<sub id>\"} and stop"}`.

---

### UC-08 — a bad `event_into` pointer fails at registration

*Proves:* `validate_template` rejects a pointer that would be a silent
per-event no-op at fire time.

```text
Try to register a call reaction with a malformed pointer:
{ "trigger_type": "state", "config": { "scope": "rtuc-08", "key": "in" },
  "function_id": "state::set",
  "metadata": { "payload": { "scope": "rtuc-08", "key": "out" }, "event_into": "value" } }
Report the exact error, then stop.
```

*Expect:* a registration error (a JSON pointer must start with `/`). Nothing is
bound — check `engine::registered-triggers::list` shows no
`harness::trigger-call` row.

---

### UC-09 — call targets are policy-gated and `harness::*` is refused

*Proves:* a reaction can only call what the registrant could call itself, and
that the harness control plane is never a call target.

Grant: `engine::register_trigger`, `state::get` **only** (no `state::set`).

```text
Run these two registrations and report each error verbatim, then stop:

1) { "trigger_type": "state", "config": { "scope": "rtuc-09", "key": "a" },
     "function_id": "state::set", "metadata": { "payload": { "scope": "rtuc-09", "key": "b" } } }

2) { "trigger_type": "state", "config": { "scope": "rtuc-09", "key": "a" },
     "function_id": "harness::send", "metadata": {} }
```

*Expect:*
1. `state::set is not permitted by this session's dispatch policy — a reaction
   can only call functions you can call yourself`.
2. `harness::send is harness-internal and cannot be a call target; bind
   harness::spawn for a sub-agent reaction, or omit function_id to be notified
   here`.

If an approval-gate worker is running, also try a target it marks
`needs_approval` — the binding is refused because a trigger-fired call runs
unattended and can never prompt.

---

## 3. Direct spawn + wake — the parent-owned pipeline

A binding never starts an agent. The fan-out shape is: the OWNER registers a
wake on the medium, spawns leaf workers itself with `harness::spawn`, and the
workers' writes wake it. These cases probe the seams of that shape.

### UC-10 — leaf children write, a barrier-gated wake fires once

*Proves:* direct spawn, medium-only child output, and `state::barrier` fan-in.

```text
1. Register a wake gated on two arrivals:
engine::register_trigger {
  "trigger_type": "state", "config": { "scope": "rtuc-10" },
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
recorded as a barrier skip, and the second wakes the owner ONCE with both
arrivals in the notification. The spawn results carry `fire_and_forget: true`
— the children's outcomes reach the owner only through the state they write,
and no `[child-failure]` or child result is ever injected.

---

### UC-11 — a leaf cannot orchestrate

*Proves:* the capability wall is policy, not prompt.

```text
harness::spawn { "task": "Try to call engine::register_trigger on any state key, then try harness::spawn. Report what happened, then stop.",
                 "session_id": "rtuc-11-probe" }
```

*Expect:* both attempts come back `is_error: true` with "not permitted by
this agent's dispatch policy" — the child inherited the parent's allow set
and gained the control-plane deny globs on top. Repeat the spawn with
`"options": { "orchestrator": true }` and the child's registration succeeds
(tear it down afterwards).

---

### UC-12 — spawn as a binding target is refused

*Proves:* the removal is loud, with the migration in the error.

```text
engine::register_trigger { "trigger_type": "state",
                           "config": { "scope": "rtuc-12", "key": "go" },
                           "function_id": "harness::spawn",
                           "metadata": { "task": "never" } }
```

*Expect:* an error naming the shape — "`harness::spawn` is not a binding
target … Spawn children directly from a turn and register a wake on what they
write." Nothing is registered.

---

### UC-13 — turn events are not agent-bindable

*Proves:* child outcomes flow through the medium, never through a binding on
their turns.

```text
engine::register_trigger { "trigger_type": "harness::turn-completed",
                           "config": { "session_id": "rtuc-13-x" } }
```

*Expect:* refused — "turn events are not agent-bindable. Watch what the work
WRITES instead — register a wake (omit `function_id`) on the state keys or
database rows the tasks update." The same refusal hits every shape.

---

## 4. Trigger conditions (`condition_function_id`)

The engine's **builtin** trigger providers accept a `condition_function_id`
inside `config`. Before dispatching, the engine calls that function with the
event as its whole payload and:

| Condition result | Outcome |
|---|---|
| bare `false` | handler **skipped** |
| any other value (`true`, object, string, number, null) | handler **fires** |
| no result at all | handler **fires** (logged as a warning) |
| the call **errors** (unknown id, bad payload) | handler **skipped**, silently |

Supported on every builtin type except `log` and `trace`: `state`, `cron`,
`durable:subscriber` (the queue), `subscribe` (pubsub), `stream`,
`stream:join` / `stream:leave`, `http`, `configuration`. Note the queue trigger
type string is `durable:subscriber` — `queue` is the *provider package* name,
not a trigger type. **Silently ignored** on worker-defined types
(`harness::turn-completed`, `storage::object-created`, …) — those are
dispatched by their own worker, which never consults the condition.

The condition sees **only the event payload** — it cannot read external state
unless the function it names goes and fetches it. Note also that no stock
worker function returns a bare `false`, so a real veto needs either a
purpose-built function or the `fp::get` trick in UC-16.

### UC-16 — a condition that actually vetoes

*Proves:* end-to-end veto and pass with stock workers only. A queue event is
the raw published message, so the message itself can carry the arguments
`fp::get` needs.

Grant: `engine::register_trigger`, `iii::durable::publish`, `state::set`

```text
Register a call reaction on a queue topic, gated by a condition:

engine::register_trigger {
  "trigger_type": "durable:subscriber",
  "config": { "topic": "rtuc-16", "condition_function_id": "fp::get" },
  "function_id": "state::set",
  "once": false,
  "metadata": { "payload": { "scope": "rtuc-16", "key": "seen" }, "event_into": "/value" }
}

Report the subscription_id, then STOP.
```

```bash
# gate CLOSED — fp::get returns false, the handler is skipped
$III trigger iii::durable::publish --json '{"topic":"rtuc-16","data":{"value":{"gate":false},"path":"/gate"}}'
sleep 2
$III trigger state::get --json '{"scope":"rtuc-16","key":"seen"}'   # -> not found

# gate OPEN — fp::get returns true, the handler fires
$III trigger iii::durable::publish --json '{"topic":"rtuc-16","data":{"value":{"gate":true},"path":"/gate"}}'
sleep 2
$III trigger state::get --json '{"scope":"rtuc-16","key":"seen"}'   # -> the message
```

*Expect:* nothing after the first publish, the message after the second. The
condition returned the bare boolean at `/gate` out of the message itself.

---

### UC-17 — a condition that errors starves the binding

*Proves:* the trap — an erroring condition skips the handler **silently**, so
the binding looks alive and never fires.

```text
Register a state notify subscription with a condition that cannot succeed:
{ "trigger_type": "state",
  "config": { "scope": "rtuc-17", "key": "go", "condition_function_id": "fp::get" } }
Report the subscription_id and the note, then stop and wait to be woken.
```

```bash
$III trigger state::set --json '{"scope":"rtuc-17","key":"go","value":{"n":1}}'
sleep 5
$III trigger session::messages --json '{"session_id":"<sid>","limit":20}'
```

*Expect:* **no wake, ever.** A state event is
`{type, event_type, scope, key, old_value, new_value}` — it has no `value` or
`path`, so `fp::get` errors, and an erroring condition is treated as "skip".
The registration succeeded, the binding is listed, the session sleeps forever.
Same outcome for a typo'd `condition_function_id`. This is the single most
expensive way to mis-wire a trigger, and nothing in the response warns you.

---

### UC-18 — harness-internal types are refused before conditions matter

*Proves:* the agent path refuses `harness::*` trigger types outright, so no
condition semantics ever apply to them.

```text
engine::register_trigger {
  "trigger_type": "harness::turn-completed",
  "config": { "session_id": "<this session id>", "condition_function_id": "fp::get" }
}
```

*Expect:* refused with "turn events are not agent-bindable" — the
`condition_function_id` question never arises. (Workers registering directly
with the engine can still bind turn events; the harness dispatches its own
turn events and never calls `check_condition`, so the key would ride along
inert there.)

---

### UC-19 — condition on cron

*Proves:* the cron condition payload, and that it cannot see the world.

```text
Register:
{ "trigger_type": "cron",
  "config": { "expression": "*/20 * * * * *", "condition_function_id": "fp::get" },
  "function_id": "state::set",
  "metadata": { "payload": { "scope": "rtuc-19", "key": "tick" } } }
Report the subscription_id and stop. I will unregister it.
```

*Expect:* the target never fires — a cron event is
`{job_id, scheduled_time, actual_time}`, so `fp::get` errors and every tick is
skipped. Point `condition_function_id` at a function that tolerates that
payload (or drop it) and the ticks land. Gating cron on external state
requires a function that fetches that state itself.

---

## 5. Limits and guardrails

### UC-20 — subscription cap

*Proves:* `MAX_SUBSCRIPTIONS_PER_SESSION = 64`.

```text
Register state subscriptions in a loop: scope "rtuc-20", keys k1 … k70, each
one a separate engine::register_trigger call with "once": false. Stop at the
first error and report which key number failed plus the error text.
```

*Expect:* the 65th registration fails with `subscription cap reached (64
active for this session); unsubscribe first`. Then tear them down (§6) —
64 standing bindings on a shared dev engine is not a state to leave behind.

---

### UC-21 — unregister is owner-scoped

*Proves:* the ownership check and its lineage exception.

```text
Session A: register any state subscription and report the subscription_id, then stop.
```

Then, from a **different** session:

```bash
$III trigger harness::send --json '{
  "message": "call engine::unregister_trigger {\"id\":\"<sub id from session A>\"} and report the exact result",
  "model": "<model>",
  "options": { "functions": { "allow": ["engine::unregister_trigger"] } } }'
```

*Expect:* `subscription belongs to a different session`. A sub-agent **spawned
by** session A can do it — reaction lineage is the deliberate teardown path for
a registrant parked on a wake its children must satisfy.

---

### UC-22 — unregistering something already gone

*Proves:* `{removed: false}` is the honest answer, not an error.

```text
Register a one-shot state subscription, let it fire, and after the wake call
engine::unregister_trigger with the SAME subscription_id. Report the result
verbatim and stop.
```

*Expect:* `{"removed": false}` — the `once` fire already retired it. Useful as
a probe: `removed: false` on a binding you never fired means something else
tore it down.

---

## 6. Cleanup

After a session of probing:

```bash
# what is still bound? Agent bindings all point at the delivery hop and are
# invisible to a per-target engine list — read the harness's own store:
$III trigger harness::triggers::list --json '{"session_id":"<owner sid>"}'
# engine-side leftovers from the pre-hop era (startup also sweeps these):
$III trigger engine::registered-triggers::list --json '{"function_id":"harness::trigger::deliver","include_internal":true}'
```

Anything listed is a live binding that will keep firing (and, for spawn
bindings, keep paying for turns). Tear each one down by steering its **owner
session**:

```bash
$III trigger harness::send --json '{
  "session_id": "<owner sid>",
  "message": "call engine::unregister_trigger {\"id\":\"<sub id>\"} and stop",
  "options": { "functions": { "allow": ["engine::unregister_trigger"] } } }'
```

Then drop the scratch state. `state::delete` takes a scope **and** a key —
there is no delete-the-whole-scope call, so enumerate with `state::list` first
if you improvised beyond the keys below:

```bash
while read -r scope key; do
  $III trigger state::delete --json "{\"scope\":\"$scope\",\"key\":\"$key\"}"
done <<'KEYS'
rtuc-01 go
rtuc-06 in
rtuc-06 mirror
rtuc-10 job
rtuc-10 result
rtuc-11 job
rtuc-11 denied
rtuc-13 raw
rtuc-13 chained
rtuc-15 ping
rtuc-16 seen
rtuc-17 go
rtuc-18 result
rtuc-19 tick
KEYS

$III trigger state::list    --json '{"scope":"rtuc-20"}'   # anything left from the cap probe
$III trigger database::execute --json '{"db":"primary","sql":"DROP TABLE IF EXISTS rtuc07_beats"}'
```

## Writing prompts that do not run away

Every case above follows the same rules, learned the expensive way:

1. **Pin the exact JSON.** Name the trigger type, config keys, `function_id`,
   and every metadata field. A model left to infer them invents plausible keys
   the engine rejects — or worse, accepts as inert.
2. **Say "then stop".** Without it the model keeps working, re-registering, or
   polling `harness::status` in a loop.
3. **Ban polling.** A watcher that polls is not testing the trigger.
4. **Give every binding a termination path** in the same prompt: `once: true`,
   or an explicit unregister the woken turn performs.
5. **Verify from outside.** State keys and `engine::registered-triggers::list`
   are ground truth; the transcript is what the model *believes* happened.
