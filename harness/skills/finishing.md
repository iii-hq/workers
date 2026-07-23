---
name: harness/finishing
description: >-
  How a fan-out run ends: the turn_complete state key, mechanical (fp::pipe) vs judgment (sub-agent) validators, the deadline cron, wave dispatch, the armed-wake invariant. Mandatory before ending the first turn of any fan-out run.
---

## Finishing: the turn-complete key

A fan-out request is NOT answered in one turn. You wire, spawn, and stop; the answer is
composed later, when the work is actually done. The stop signal is a state key you watch:

Step 1. Pick a run scope unique to this run — a slug plus random characters, e.g.
`run-headlines-b4k9`. Every child writes its own key inside this scope.

Step 2. Register a one-shot notify trigger (NO `function_id`) on
`{ trigger_type: "state", config: { scope: "<run scope>", key: "turn_complete" } }`. Its
firing is your wake-up call to finish. NEVER bind this (or the deadline in Step 4) to
`harness::react` or a `call` — a call's result is discarded in the void; only a plain
notify injects into YOUR session and wakes you.

Step 3. Register the validator that decides when the run is done — always with
`continue_on_error: true` on every edge so failed children still reach it (a validator is
an outcome checker, not a success handler). For a couple of children, one edge on
`harness::turn-completed` with `config { parent_session_id: "<this session>" }` is enough;
for larger fan-ins use a join with `expect` covering every child (a single subscription's
fires are rate-limited to ~10 per minute — a join accumulates instead of firing per event).
`parent_session_id` only matches children that CARRY the link: ones you `harness::spawn`
from this session, or reaction-spawned children whose `metadata.parent_session_id` names
this session. Children dispatched by reactions or later waves without that link never
match the filter — for those, key the validator on the run-scope state keys the children
write (a `state` trigger on the scope) or a join with explicit per-child `expect`, or
their completions never reach the validator and `turn_complete` is never written.
Pick the validator KIND by the rule:

- MECHANICAL rule (keys present, a count reaching N, a value matching) → a `call` reaction
  running `fp::pipe`: zero tokens, milliseconds, no sessions. The `fp::when` guard stops
  the pipe when the condition fails, so `turn_complete` is only written on pass:

      engine::register_trigger {
        trigger_type: "harness::turn-completed",
        function_id:  "harness::react",
        config: { parent_session_id: "<this session>" },
        once: false,
        metadata: { continue_on_error: true, call: { function_id: "fp::pipe", payload: {
          through: [
            { function: "database::query", payload: { db: "primary",
              sql: "SELECT COUNT(*) AS n FROM <run table>" } },
            { function: "fp::get",  payload: { path: "/rows/0/n" } },
            { function: "fp::when", payload: { op: ">=", to: <N> } },
            { function: "state::set", payload: { scope: "<run scope>",
              key: "turn_complete", value: { done: true } }, into: "/value/count" }
          ] } } }
      }

  The same shape counts state keys (start from `state::list`) or checks any readable
  store. A standing (`once: false`) call edge re-checks on every child completion and
  passes exactly once the threshold holds; unregister it when you finish.

  A pipe GATES and moves ONE value — it does NOT compute a new value from several. `fp::*`
  has no arithmetic (no sum/average/reduce), so a pipe that `state::get`s two keys just
  threads the LAST one, never their total. To AGGREGATE (sum children into a subtotal,
  combine results), gate on all inputs being PRESENT — a join over their keys, or a count
  reaching N — and let that gate WAKE YOU with a plain notify (no `function_id`) so the sum
  runs in YOUR OWN next turn, where you already hold your permissions. Do NOT put the
  compute in a react/join DOWNSTREAM TASK unless you also grant it write access:
  a trigger-fired downstream is PARENTLESS — it starts from the read-only baseline and its
  `state::set` is denied, so the subtotal silently never writes and the whole tree above it
  stalls. Either wake yourself and compute (simplest), or pass
  `metadata: { options: { functions: { allow: ["state::get","state::set"] } } }` on the
  downstream. And never gate a MULTI-input condition with a one-shot watcher on ONE key: the
  fire can beat the sibling writes, short-circuit, and strand the check forever — use a join
  (it accumulates every predecessor) instead.

  A pipe READS
  (`state::get`, `state::list`, read-only `database::query`, `fp::*` transforms) and
  writes ONLY `state::set`: steps run with WORKER authority, so `harness::spawn`, every
  DATABASE WRITE (`database::execute`/transactions/prepared statements), and all
  turn-control, session, trigger-control, credential, router, and shell/coder steps are
  REFUSED in a pipe. The gate writes `turn_complete` to STATE; anything else — a db
  marker row, a respawn — happens on the woken turn or a react edge, with agent
  authority. For a mechanical retry, write failures to their OWN key (e.g.
  `<key>-failed`) and register a react edge on it whose REACTION is the respawn
  (`metadata.task` + `model` + `session_id` + `options`) — that spawn runs with agent
  authority under your policy. The threaded value lands at each
  step's `into` (default `/value`) and REPLACES any literal there — on a write step aim
  it at a scratch subfield (`into: "/value/_seen"`) or the literal `value` you meant to
  write is silently overwritten.
- JUDGMENT rule (quality, coherence, "is this actually an answer") → a sub-agent
  validator. Pin it into its OWN session — `metadata.session_id:
  "validator-<run>-<rand>"`, with `metadata.parent_session_id: "<this session>"` for
  console nesting; NEVER leave its `session_id` out, or the reaction delivers into THIS
  chat. Grant it `metadata.options: { functions: { allow: ["state::get", "state::set"] } }`.
  Its task is self-contained, e.g.: "For each of <scope>/<key1>, <scope>/<key2>:
  `state::get` it, expecting `{ ok, value | error }`. All present with ok true →
  `state::set` <scope>/turn_complete = { done: true, keys: [...], summary: '<one line>' }.
  Anything missing or ok false → `state::set` <scope>/turn_complete = { done: false,
  missing: [...], errors: [...] }. ALWAYS write turn_complete." (`state::list` returns
  values without keys — verify named keys with `state::get`.)

Step 4. Register a one-shot `cron` notify at this run's deadline — and pass
`once: true` EXPLICITLY: cron is the one trigger type that defaults to RECURRING, so a
deadline registered without it fires every period forever (and is not tracked as your
armed wake). If it fires before `turn_complete` is set, a child never terminated:
compose what state holds, report the gap, unregister everything, stop. A run must never
depend on every child behaving.

Step 5. Spawn the children (up to your per-turn spawn cap), then end the turn: "running —
I'll report when the results land." If there are MORE items than one turn can spawn, this
is a WAVE: record the full work-list and a cursor in the run scope, spawn the first wave,
then arm ONE RELIABLE dispatch-wake to bring you back for the next wave — a single
recurring short `cron` (e.g. every minute) registered ONCE for the whole run, or a
self-notify you RE-ARM as the first act of each woken turn. Pick ONE mechanism, ONCE:
never arm a fresh cron per wave — stacked crons all keep firing for the rest of the run,
burning a wake-turn each, every minute. Do NOT rely on child-`turn-completed` events to
pace dispatch: they stop firing once the current wave's children finish (and are
rate-capped ~10/min), so a run that waits on them deadlocks with items still
un-dispatched. On each dispatch-wake, spawn the next wave from the cursor; once the
cursor is exhausted, drop the dispatch-wake (unregister the cron; a self-notify you
simply stop re-arming) — but ONLY if the `turn_complete` gate notify or the deadline
still stands to wake you. INVARIANT for every turn you end: while children are in flight
or the gate has not passed, at least ONE armed wake must survive the turn — gate notify,
dispatch-wake, or deadline. End a turn with zero armed wakes and the run strands at
whatever the children last wrote, done work with no one left to collect it. A fan-out that
stops after one wave silently covers only the first slice and the gate never reaches its
count. And do NOT begin a large downstream deliverable (a UI, a compiled report) until the
producer loop is finished and the gate is satisfied: interleaving a secondary build with an
unfinished fan-out is how a run wanders off and stalls half-done. Finish the loop, THEN
build the extras.

When the `[notification]` for `turn_complete` arrives, read the scope and finish:
`done: true` → compose the final answer FROM STATE (what was actually written, never your
memory of what you asked for), unregister the deadline and any standing watchers with
`engine::unregister_trigger`, and stop. `done: false` → report the failure with what
`missing`/`errors` name — or re-arm a fresh one-shot notify on `turn_complete` FIRST, then
respawn only the named work.

# End-of-run checklist

If this is a fan-out run, check the finish path: validator pinned to its OWN session?
One-shot state notify armed on the `turn_complete` key itself — the deadline cron is the
FALLBACK, never the loop's clock (cron-only wakes stall every round until the next
boundary)? Will `turn_complete` be written on EVERY path — success, failure, and timeout?

If you end with reactions armed, check each one: can its producer actually produce the watched
key or event — is the write inside the producer's allowed functions, and will the filtered
session exist? A reaction armed on something nothing can produce waits forever, silently.
