---
name: harness
description: >-
  The durable agent turn loop — kick off a turn with `harness::send`, render
  it from session-manager transcript events, react to
  `harness::turn-completed`, with deny-by-default tool dispatch and synchronous
  hook extension points for policy siblings.
---

# harness

The harness is the durable turn loop that wires `session-manager`, `llm-router`,
and `context-manager` into an agent. A consumer stays thin: it kicks off a turn,
renders the conversation from the session transcript, and reacts to turn
boundaries and human-gated calls. The harness streams the assistant message into
the session as it generates, so you watch the session, not the harness — there
is no `agent::events` stream, and `harness::status` is a point-in-time recovery
read, not a render feed.

Every invocation is a trigger (`iii.trigger({ function_id, payload })`); there is
no separate "call" verb. Tool dispatch is deny-by-default: a send with no
`options.functions.allow` is a plain chat loop and every model-requested call is
refused until you allow globs per send. Sessions are minted by the harness
(`s_<uuid>`) or supplied by you; a send into a running turn folds in as steering
(`merged: true`) instead of erroring, and a repeated `idempotency_key` returns
the original turn without appending.

Prerequisites: `session-manager` (required — transcript store and change feed)
and `llm-router` (required — generation and the model catalog) must be present.
`context-manager` (token budgeting and compaction) is a soft dependency — absent
it, the harness sends raw history. `approval-gate` (the human-in-the-loop gate)
is optional; without it no call is held and every allowed call runs un-gated.

## When to Use

- Start or steer an agent turn and return immediately (`harness::send`).
- Cancel an in-flight turn (`harness::stop`) or read coarse turn state for
  recovery and guards (`harness::status`).
- Chain turns or react to outcomes by binding `harness::turn-completed`.
- Drive a turn from an arbitrary inbound event (cron tick, webhook, sensor) by
  translating it into a `harness::send`.

## Boundaries

- Not a transcript feed. Render from `session-manager`'s `session::message-added`
  / `message-updated` / `status-changed` (reconcile by `revision`); do not poll
  `harness::status` for content.
- Not the approvals engine. The harness ships only the gate mechanics; the
  policy, decision RPCs (`approval::resolve`), inbox (`approval::list-pending`),
  and prompt triggers live in `approval-gate`.
- Not a chain guard. `options.max_turns` bounds a single turn, not a
  send-completed-send loop; carry your own stop condition.
- Do not trigger the internal functions (below) — they forge call ids and turn
  progress, so calling them out of band corrupts the turn record.
- An in-run agent cannot start turns: `send` / `run` / `turn` / `stop` are denied
  to the model by policy. `harness::spawn` is the only turn-starter an agent calls
  directly, and it self-enforces depth, fan-out, and policy subsetting. The other
  path is event-driven: an agent binds `engine::register_trigger` →
  `harness::react` (see Reactive triggers) and the engine spawns the sub-agent
  when the event fires.

## Functions

Consumer-facing:

- `harness::send` — ensure the session, persist the incoming message, and kick
  off a turn; returns fast or merges into a running turn (steering).
- `harness::stop` — request cancellation of an in-flight turn; cascades to
  spawned children.
- `harness::status` — read the current turn state for a session; `null` when no
  turn ever ran. For recovery and guards, not rendering.
- `harness::spawn` — spawn a sub-agent in a child session. Model-facing (invoked
  through `agent_trigger`), not a consumer entry point.

Internal — the harness drives these; never trigger them directly:
`harness::turn` (the durable loop step), `harness::function::trigger` /
`harness::function::resolve` (dispatch and parked-call settle),
`harness::sweep-pending` (cron expiry), `harness::react` (the trigger-bridge
target — bound via `engine::register_trigger`, never triggered directly), and
`harness::on-config-change` (hot-reload).

## Reactive triggers

The harness emits two async turn-boundary trigger types so consumers and siblings
react without polling `harness::status`:

- `harness::turn-started` — a turn began executing (first loop step).
- `harness::turn-completed` — a turn reached a terminal status
  (`completed` / `cancelled` / `failed`), carrying the result or error for
  chaining, failure toasts, auto-titling, and result delivery.

Bind `turn-completed` for outcomes and to chain the next hop; bind `turn-started`
only for observability. Delivery is fire-and-forget, at-least-once, and unordered
— treat each event as an edge. Nothing replays on reconnect: re-seed with
`harness::status` and `approval::list-pending`, then rebind. Do not bind these for
live transcript rendering — that is `session-manager`'s job.

Binding `config` filters delivery by `session_id`, or by `parent_session_id` to
watch the children a turn `spawn`s (in-turn spawns only — a direct
`harness::spawn` call creates no parent link, so filter those by `session_id`).

An event can also START a sub-agent, not just notify a handler: bind the event to
`harness::react` with the sub-agent spec in the registration `metadata`
(`{ model, task, session_id?, parent_session_id?, provider?, options?, join? }`) —
when the event fires, the engine spawns it. `model` must be a live id from
`router::models::list` (validated at registration and again at fire time).
Omit `parent_session_id` and the child nests under the registering session's
root automatically; pin it only to choose a different REAL session (an invented
id shows the children as disconnected roots). A trigger-fired spawn has no
parent policy to inherit — it gets the harness's read-only `default_functions`
baseline unless `options.functions` grants more. Predecessor subscriptions
carrying the same `join.id`/`expect` and a distinct `key` each form a fan-in
barrier: the downstream spawns exactly once, fed every predecessor's result, and
the join's subscriptions are auto-unregistered once it fires (set
`join.rearm: true` to keep them registered so the join fires again on each next
complete set). The downstream delivers into the registering session by default
when `session_id` is omitted — the fan-in result lands back in that chat.
Filter turn-completed predecessors only by session ids the upstream specs
actually pin (or join on state keys instead); registration returns a warning
`note` when the filtered session doesn't exist. Runaway chains are stopped by three loop breakers — self-edge
drop, a reactive depth cap of 8, and a ~10-spawns/minute per-subscription rate
limit — but still design filters so a reaction is not matched by its own
subscription. Filter join predecessors by `session_id` (pre-pick the
child session ids, unique per run — a readable slug plus a few random
characters, e.g. `critic-a-b4k9`, never the originating session id as a
prefix; spawn's `session_id` creates the session if missing but silently
reuses an existing one, transcript and console nesting included), never
`parent_session_id`. Set the last stage's `session_id` to the originating session
to deliver the pipeline's result back into that conversation. This is the in-run
agent's chaining path; the `registerFunction` recipe below is for workers.

### How to bind

1. Register a handler: `registerFunction('myapp::on-turn-done', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'harness::turn-completed',
  function_id: 'myapp::on-turn-done',
  config: { session_id: sessionId },
})
```

For the event payload shape, call `get function info` on the trigger type.

### Hooks (policy siblings only)

The harness also registers five synchronous, in-path hook trigger types:
`harness::hook::pre-turn`, `harness::hook::pre-generate`,
`harness::hook::post-generate`, `harness::hook::pre-trigger`, and
`harness::hook::post-trigger`. A bound hook runs in the turn's critical path and
the harness acts on its return value (veto / hold / mutate) under a per-binding
`timeout_ms` and `on_error` policy; `pre-trigger` / `post-trigger` bindings take a
`functions` glob list to scope which calls they gate. These are for
operator-trusted policy siblings (`approval-gate` binds `pre-trigger`); ordinary
consumers do not bind hooks.
