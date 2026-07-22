---
name: harness/orchestration
description: >-
  One-way orchestration playbook: binding triggers, harness::spawn (never waits), self-contained single-shot child tasks, reactions via harness::react, call-edges, joins, child naming and permissions. Mandatory before any spawn, trigger registration, or 'when X happens do Y'.
---

# Binding a trigger

Copy the config keys from the schema. A binding can succeed and still never fire if the type's
provider is down or the keys are wrong. The bound function receives what the trigger type
delivers and returns what the type expects:
the handler contract is the trigger type's, not a generic one.

## One-way orchestration: wire, spawn, stop

`engine::register_trigger` is THE callback primitive. Any "when X happens, do Y" is a
registered trigger — never a poll, never a turn kept alive to wait. Subscriptions live in
the engine: they fire with no live turn, keep firing after your turn ends, and are replayed
after an engine restart. Registering a callback IS a deliverable: register it, say what you
registered, end the turn.

`harness::spawn` never waits: it seeds the child and returns
`{ child_session_id, child_turn_id }` immediately. The child's result is NEVER delivered
back to you — a child talks to the world only through the state it writes and the
`harness::turn-completed` event its finish fires. Every fan-out follows the same three
steps, in this order:

Step 1. Wire the consumers FIRST. Register a trigger for every result you intend to
consume — a `state` trigger on each key a child will write, or `harness::turn-completed`
edges and joins (see Reacting to events) — BEFORE any producer starts. A result nothing
listens for is lost.

Step 2. Spawn the children, each with a fully SELF-CONTAINED task: the exact inputs
inline, the exact state scope + key to write, and the envelope to write there —
`{ "ok": true, "value": <result> }` on success, `{ "ok": false, "error": "<why>" }` on
failure — and ZERO context about the overall goal, sibling agents, or you. A child knows
only its task, by design; a task that says "report back to me" or "as part of the larger
plan" is malformed. Pin the shared destination ONCE: whatever the children write to — a
state scope+key, or a database table — fix its EXACT name up front and use that IDENTICAL
string in every child task AND in the gate that later counts it. A gate counting
`results_v2` while the children write `results` never reaches its target and the run never
ends; never invent a second name or paraphrase the one you chose. Each child task is
SINGLE-SHOT: it does the thing once, writes its
envelope, and stops — NEVER write a task that tells the child to loop, retry over time,
sleep, or wait for a deadline (a child stuck in its own loop ignores your deadline and
can stall the whole run). Retrying a failed item over rounds is YOUR job, via the
failure-key respawn below; a flaky call inside the child uses the worker's own bounded
`timeout`/`retries` arguments, not a hand-rolled loop. Independent spawns go in ONE
message; each returns its child ids instantly and the children run in parallel.

Step 3. End your turn: reply with what you wired and started, then stop. Notifications
wake you when the watched state changes; the engine owns the sequencing. Never poll,
never wait, and never redo work a registered reaction owns.

Name every child you spawn: always pass `session_id` — a short readable slug for the child's
job plus a few random characters for uniqueness, e.g. `fetch-headlines-b4k9`. Never prefix it
with your own session id. Omitted, the engine mints an opaque UUID row in the console; a slug
without the random suffix can collide with an earlier run and silently resume that session,
old transcript and all. A react trigger's `metadata.session_id` has its own rule (Reacting
section below): pin a fresh unique id for each stage that must run as its own child; omit it
only when the reaction should deliver back into the registering chat — and never pin a fixed
id on a recurring trigger, which funnels every firing into one session.

By default an in-turn child INHERITS your full policy — the same permissions you hold. You
can never grant a child MORE than you have (spawns never escalate), and you may narrow one
to least privilege with `options: { functions: { allow: [...] } }` — but if you narrow, give
it everything its task must CALL: a child told to write state without `state::set` in its
allow list finishes politely with its work stranded in its transcript, and every reaction
armed on that write waits forever. A child inherits the permission to `harness::spawn` and
register triggers too, and is a LEAF by DEFAULT: it does one task and writes state, and does
not orchestrate UNLESS you give it an explicit coordinator task. Flat is usually better —
if a task just needs more hands, split it into more children here; but for genuinely
hierarchical work you MAY hand a child a coordinator task that spawns its own sub-tree (it
inherited the permission), and depth is bounded by the spawn-depth limit. (A direct/CLI/trigger-fired spawn has no parent
to inherit from — it starts from the read-only baseline, so grant it explicitly.)

## Reacting to events

An event can START a sub-agent, not just notify a handler — but a `harness::turn-completed` or
`state` event carries no `task`/`model`, so it cannot bind straight to `harness::spawn`. Bind it
to `harness::react` and put the sub-agent you want in the trigger's `metadata`:

    engine::register_trigger {
      trigger_type: "harness::turn-completed",           # or "state", … per engine::triggers::list
      function_id:  "harness::react",
      config:   { parent_session_id: "<this session>" }, # the type's config schema (filters)
      metadata: { task: "<what the reacting sub-agent should do>",
                  model: "<optional; omit to run the reaction on your own model>",
                  session_id: "<pin a child id here, exactly like harness::spawn's session_id>",
                  parent_session_id: "<your session id — the console root for the child>" }
    }

`metadata.session_id` picks WHICH session the reacting sub-agent runs in — omitting it does
NOT create a fresh distinct child. It falls back to the session that REGISTERED this trigger,
so the reaction fires back into THAT chat every time (fine for a pipeline's last stage,
deliberately delivering the final result "back here" — wrong for any EARLIER stage meant to
run as its own child). Any stage you want spawned as a distinct sub-agent — including each
branch of a fan-out like "two parallel analysts" — needs its OWN explicit `session_id`, picked
by you, unique to this run (same discipline as a direct `harness::spawn` call and as the Join
section's predecessor ids below): a readable slug plus a few random characters, e.g.
`summarizer-b4k9`.
Reusing an id from an earlier run silently RESUMES that old session instead of starting fresh.

The delivery rule, explicitly: the FINAL stage of a join or pipeline — the one composing the
result the user is waiting for — must OMIT `metadata.session_id`, so it spawns back into the
chat that registered it and the answer arrives here. Pin a `session_id` on the final stage
ONLY when the result must land in a different session; a pinned final stage never reports
back, and the user waits on a reply that already went somewhere else.

`metadata.parent_session_id` pins where the reacting sub-agent nests in the console tree
(unrelated to which session it runs in). It MUST be a REAL session id — normally your own. An
invented group id has no session behind it, so the console cannot attach the children anywhere
and shows them as disconnected top-level rows. Omit it and the reaction nests under the firing
session's root (session events) or the registering session's root (`state`/`cron`/`stream`
events carry no session in the event). `metadata.model` is OPTIONAL — omit it and the reaction
runs on your own model; set it only to switch models, and then only to a live id from
`router::models::list`, never one from memory (an unknown model is rejected at registration and
never spawns). A trigger-fired sub-agent starts with only a read-only baseline (discovery and
reads — no writes, no spawning, no trigger registration) — grant anything more via
`metadata.options` (same shape as `harness::spawn` `options`), e.g.
`options: { functions: { allow: ["state::get", "shell::fs::*"] } }`.

`harness::react` is documented here on purpose: it never runs as a direct call (agents are
denied), only as a trigger target — do not look it up or probe it first; use the id exactly as
written, and keep the id `register_trigger` returns as your handle to unregister.

A reaction can also be a FUNCTION CALL instead of a sub-agent: put `call` in the metadata
instead of `task` — `metadata: { call: { function_id: "<id>", payload: { ... } } }` (no
`model`, no `session_id`, no `options`). On fire, the event is injected into the payload at
`/event` (override with `call.event_into`) and the function is dispatched directly:
deterministic, token-free, milliseconds. `call.function_id` must be a function YOUR policy
allows — it is checked at registration, and harness-internal targets are refused. A
completed join's downstream call receives all predecessor results as
`{ results: { "<key>": <event>, ... } }` at the same pointer. Prefer a call for anything
MECHANICAL — counting, thresholds, moving values (pair it with `fp::pipe`, whose `fp::when`
guard stops the pipe when a condition fails); use `task` only when the reaction needs
judgment. A call can NEVER reach you: its result is discarded, nothing lands in any
session. Anything that must WAKE you — your turn-complete watcher, your deadline — is
ALWAYS a plain notify (`engine::register_trigger` with NO `function_id`); binding your own
wake to a react/call edge reads into the void and strands the run.

`harness::react` simple edges are one-shot by default (except cron, which is recurring).
Set `once: false` only for a deliberate standing watcher. Join predecessors ignore `once`;
the join owns their lifecycle.

`harness::react` spawns a sub-agent (`harness::spawn`) with your `task` (the event JSON appended
so it sees what fired — a `turn-completed` event carries the turn's `status` and, when it
completed, its `result`). Failed/cancelled turns and completed turns carrying `result_error`
do NOT start the success-path reaction. Set `metadata.continue_on_error: true` only for an
explicit error handler that needs the failure event and any preserved partial result. Two
common shapes:

- Notify when a sub-agent finishes: `harness::turn-completed` with
  `config { parent_session_id: "<this session>" }` — fires when any child you spawned completes.
- Start work on a state change: `state` with `config { key, scope }` — fires on
  create / update / delete of that key.

Join (wait for several): to spawn only after MULTIPLE predecessors finish:

Step 1. Pick a session id for each predecessor yourself, unique to THIS run: a readable slug
plus this run's random suffix, e.g. `critic-a-b4k9` (never your own session id as a prefix).
`harness::spawn`'s `session_id` creates the session if it does not exist — but an id from an
earlier run silently REUSES that session: its old transcript carries over and the console
keeps it nested under the old run.
Step 2. Register ONE `harness::turn-completed` subscription per predecessor, filtered on that
predecessor's own id: `config { session_id: "<that child>" }`. Do NOT filter a join on
`parent_session_id` — it matches EVERY child, so the first completion would fill every key.
Each subscription's `metadata` is the SAME full downstream spec — the combiner's `task` on
all of them (`model` optional, as above), the SAME `join.id` and `expect` list, and only its OWN `key` differing:

    metadata: { task: "<combine the inputs>",
                join: { id: "J", expect: ["a","b"], key: "a" } }   # the "b" predecessor uses key: "b"

A metadata without `task` is silently ignored and the join never fires; differing
tasks make the downstream nondeterministic (the last arrival's spec spawns it).

Step 3. Spawn the predecessors into the ids you picked.

`harness::react` accumulates each predecessor's result durably and spawns the downstream
sub-agent EXACTLY ONCE — when the last successful predecessor arrives, fed all their results —
and unregisters the join's predecessor subscriptions automatically. A failed predecessor counts
as arrived but stops the normal downstream spawn once the barrier settles; set
`metadata.continue_on_error: true` on its join edge only when the downstream is intentionally an
error handler. Set `join.rearm: true` on every predecessor to keep them registered so the join
can fire again on each next complete set.
That is how you build a graph edge-by-edge (fan-in / dependencies) without a workflow spec.

The pipeline's final output arrives back in THIS chat by default: a completed join's
downstream spawns into the session that registered it, as a new turn here. Set the LAST
stage's metadata `session_id` only to deliver into a different session instead. For a
top-level run answering a user, prefer the turn-complete key (next section) as the stop
signal — it wakes YOU to compose the answer instead of handing your chat to a reaction
agent. Build join
predecessors on `state` keys each stage writes (no session identity involved). If one instead
filters `harness::turn-completed` by `session_id`, you MUST pin that SAME id on the upstream
reaction's `session_id`: an id no spawn pins names a session that never exists, and the join
starves at 0/N forever (registration returns a warning `note` when the filtered session does
not exist).

Unsubscribe with `engine::unregister_trigger { id }` (the id `register_trigger` returned). Aim
the reaction at a session NOT covered by the same filter (or unsubscribe when done) so it cannot
retrigger itself. Three loop breakers are built in — a subscription never fires for the completion of the sub-agent it itself spawned, reactive chains hard-cap at depth 8, and a single subscription is rate-limited to ~10 spawns per minute — but still design filters so a reaction is not matched by its own subscription.

# End-of-turn checklist

If work continues after your reply ("when X finishes, do Y"), check: did I register it with
`engine::register_trigger` instead of waiting or polling?

If you spawned children, check: was every consumer trigger registered BEFORE its producer
started, and does every child task name its exact state destination + envelope and carry
everything the child needs inline? Children know nothing else.
