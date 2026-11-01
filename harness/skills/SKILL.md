---
name: harness
description: >-
  The durable agent turn loop — kick off a turn with `harness::send`, render
  it from session-manager transcript events, observe
  `harness::on-turn-completed`, with deny-by-default tool dispatch and
  synchronous hook points for policy siblings.
---

# harness

The harness is the durable turn loop that wires `session-manager`, `llm-router`,
and `context-manager` into an agent. A consumer stays thin: it kicks off a turn,
renders the conversation from the session transcript, and reacts to turn
boundaries and human-gated calls. The harness streams the assistant message into
the session as it generates, so you watch the session, not the harness — there
is no `agent::events` stream, and `harness::status` is a point-in-time recovery
read, not a render feed.

Every invocation is a call (`iii.call({ function_id, payload })`), and every
reaction is an observer (`iii.observe(source, target)`). Tool dispatch is
deny-by-default: a send with no
`options.functions.allow` is a plain chat loop and every model-requested call is
refused until you allow globs per send. Sessions are minted by the harness
(`s_<uuid>`) or supplied by you; a send into a running turn folds in as steering
(`merged: true`) instead of erroring, and a repeated `idempotency_key` returns
the original turn without appending.

A session-creating send names a `model`, or only a `provider`: the harness
then starts on that provider's declared default model (`default_model` in
`router::provider::list`, checked against its live catalog; a provider that
reports none fails the send as before). A new session that names neither
`thinking_level` nor `provider_options` also starts on the provider's
`default_thinking_level`; an explicit level wins, and later sends into the
session inherit the prior turn's as usual. `thinking_level` is `off`,
`minimal`, `low`, `medium`, `high` or `xhigh`; `off` is honoured only on
models whose catalog entry says `supports_thinking_off: true`; elsewhere the
fallback is the provider's own (most send their lowest effort and warn, Kimi
ignores the level).

Prerequisites: `session-manager` (required — transcript store and change feed)
and `llm-router` (required — generation and the model catalog) must be present.
`context-manager` (token budgeting and compaction) is a soft dependency — absent
it, the harness sends raw history. `approval-gate` (the human-in-the-loop gate)
is optional; without it no call is held and every allowed call runs un-gated.

## When to Use

- Start or steer an agent turn and return immediately (`harness::send`).
- Cancel an in-flight turn (`harness::stop`) or read coarse turn state for
  recovery and guards (`harness::status`).
- Chain turns or react to outcomes by observing `harness::on-turn-completed`.
- Drive a turn from an arbitrary inbound event (cron tick, webhook, sensor) by
  translating it into a `harness::send`.

## Boundaries

- Not a transcript feed. Render from `session-manager`'s `session::on-message-added`
  / `on-message-updated` / `on-status-changed` (reconcile by `revision`); do not poll
  `harness::status` for content.
- Not the approvals engine. The harness ships only the gate mechanics; the
  policy, decision RPCs (`approval::resolve`), inbox (`approval::list-pending`),
  and prompt events live in `approval-gate`.
- Not a chain guard. `options.max_turns` bounds a single turn, not a
  send-completed-send loop; carry your own stop condition.
- Do not call the internal functions (below) — they forge call ids and turn
  progress, so calling them out of band corrupts the turn record.
- An in-run agent cannot start turns: `send` / `run` / `turn` / `stop` are denied
  to the model by policy. `harness::spawn` is the only turn-starter an agent calls,
  and it self-enforces depth, fan-out, and policy subsetting. An observer never
  starts an agent: an agent observes with `harness::observe` to be woken (or to
  have a plain function called), and spawns from the woken turn.

## Functions

Consumer-facing:

- `harness::send` — ensure the session, persist the incoming message, and kick
  off a turn; returns fast or merges into a running turn (steering).
- `harness::stop` — request cancellation of an in-flight turn; cascades to
  spawned children and kills the `shell::exec_bg` jobs the stopped sessions
  started through their own tool calls (the harness stamps those with the
  session; jobs started by observer deliveries or `fp::pipe` steps are unowned
  and survive).
- `harness::status` — read the current turn state for a session; `null` when no
  turn ever ran. For recovery and guards, not rendering.
- `harness::spawn` — spawn a sub-agent in a child session. Model-facing (invoked
  through `agent_call`), not a consumer entry point.
- `harness::observe` / `harness::unobserve` — the agent's observe tools,
  intercepted by the turn loop for the calling session (see Agent observers).
- `harness::observers::list` / `harness::observers::dispose` — read and remove a
  session's agent observers (in-turn calls default to the calling session).
- `harness::ask` — put a decision with discrete options to the user as a
  clickable card (1–4 questions, 2–4 options each). Model-facing (invoked
  through `agent_call`) and answered by the turn loop itself: the turn ends
  on the question and the answer arrives as the user's next message. Refused
  in sub-agents, structured-output turns, and for a second ask in one step.

Internal — the harness drives these; never call them directly:
`harness::turn` (the durable loop step), `harness::function::call` /
`harness::function::resolve` (dispatch and parked-call settle),
`harness::observer::deliver` (the target of every agent observer),
`harness::sweep-pending` (cron expiry), and `harness::on-config-change`
(hot-reload).

## Filesystem scope

`options.metadata.fs_scope.root` on a send is the session's working
directory; a later send that omits `fs_scope` keeps it. The harness stamps a
trusted `fs_scope { root, grants, boundary }` onto every `shell::*` /
`coder::*` call and strips any the model supplies. `boundary` decides what
`root` means to the `ide` worker:

- `workspace` — `coder::*`, `shell::fs::*` and an exec `cwd` stay inside
  `root` plus the session's grants (`harness::filesystem::grant`).
- `configured_roots` — `root` only anchors relative paths; the worker's own
  roots apply. The model's prompt says so ("default directory, not an access
  boundary").

The `filesystem_boundary` config picks it: `auto` (the default) is
`workspace` only while approval-gate's access watch is bound; `workspace` or
`configured_roots` pins it. `harness::filesystem::info` reports the boundary
in effect. A sub-agent spawned in a turn into a new session starts with a
copy of its parent's grants; later grants to the parent do not reach it.
Under `workspace`, an in-turn spawn's `options.filesystem_root` must lie
inside the parent's root or grants. What an exec'd process itself writes is
the `ide` worker's `fs.exec_confinement` switch.

## Turn events

The harness hosts private functions that consumers and siblings observe instead
of polling `harness::status`. Each returns the event it is called with, so an
observer with `deliver: 'output'` receives the event itself:

- `harness::on-turn-started` — a turn began executing (first loop step).
- `harness::on-turn-completed` — a turn reached a terminal status
  (`completed` / `cancelled` / `failed`), carrying the result or error for
  chaining, failure toasts, auto-titling, and result delivery; `terminal` is
  `false` while the session still owns an armed wake.
- `harness::on-message-queued`, `harness::on-observers-changed`,
  `harness::on-compaction`, `harness::on-session-tree-deletion` and
  `harness::on-ready` — see the worker README for their events.

None takes args: filter on `input.session_id`, or on
`input.parent_session_id` / `input.parent.session_id` to watch the children a
turn `spawn`s (in-turn spawns only — a direct `harness::spawn` call creates no
parent link, so filter those by `input.session_id`). Observe
`on-turn-completed` for outcomes and to chain the next hop; observe
`on-turn-started` only for observability. Delivery is at-most-once and
unordered — treat each event as an edge. Re-seed with `harness::status` and
`approval::list-pending` after a reconnect. Do not observe these for live
transcript rendering — that is `session-manager`'s job.

```typescript
iii.registerFunction('myapp::on-turn-done', handler)
iii.observe(
  {
    function_id: 'harness::on-turn-completed',
    filters: { 'input.session_id': { $eq: sessionId } },
    deliver: 'output',
  },
  'myapp::on-turn-done',
)
```

`engine::functions::info { function_id }` returns each event's shape as its
`response_format`.

## Agent observers

An in-run agent observes with `harness::observe`; the harness intercepts the
call, owns the binding for the calling session, and creates the observer:

```json
harness::observe {
  "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "run-x" }, "input.key": { "$eq": "report" } },
  "label": "report-ready",
  "lifecycle": { "expires_in_ms": 600000 }
}
```

Omit `target` to be woken in this session; name `target: { function_id,
payload?, event_into? }` to have a plain function called with the event
(never `harness::*`). `harness::on-timer { at | in_ms }` is the one-shot
deadline. Agents cannot observe the turn events: watch what the work writes. A
wake is once by default, a call observer is standing; bound standing observers
with `lifecycle` and remove them with `harness::unobserve { id }`. Nothing
throttles an observer, and a cycle routed through a state write re-enters at
depth 0 and every lap is a paid turn: design reactions acyclically. See
`architecture/reactive-observers.md` for the full payload.

## Hooks (policy siblings only)

The harness hosts six synchronous, in-path hook functions:
`harness::hook::on-pre-turn`, `harness::hook::on-pre-generate`,
`harness::hook::on-post-generate`, `harness::hook::on-pre-call`,
`harness::hook::on-post-call`, and `harness::hook::on-post-turn`. A sibling
observes one with args `{ functions?, sessions?, priority?, timeout_ms?,
on_error?, inject_prompt?, ... }`; at each point the harness calls the function
and its observers' targets run in `priority` order in the turn's critical path,
where the harness acts on their answers (veto / hold / mutate).
`on-pre-call` / `on-post-call` observers take a `functions` glob list to scope
which calls they gate. Each hook function's `request_format` and
`response_format` (`engine::functions::info`) are the input its observers'
targets receive; the harness gets back the chain's merged outcome. These are for operator-trusted policy siblings
(`approval-gate` observes `on-pre-call`); ordinary consumers do not observe
hooks. An agent may observe `on-post-turn` for its own session, with a
validator as the target.
