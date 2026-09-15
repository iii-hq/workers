---
name: harness/reference
description: >-
  What the API reference cannot tell you about the harness: how a turn runs
  from send to terminal status, the trigger types it publishes and their
  delivery semantics, the three reaction forms a session can register, and
  the sub-agent lifecycle from fire-and-forget spawn to [child-failure].
  For the function surface read the API reference
  (https://workers.iii.dev/workers/harness.md). The conduct playbooks
  say what an agent SHOULD do; this says what the machine DOES.
---

# harness/reference

The harness is the durable turn loop. It wires three siblings into an agent:
`session-manager` (the transcript), `context-manager` (what fits in the
window), and `llm-router` (which model generates). A consumer stays thin: it
calls `harness::send`, renders the conversation from session events, and
reacts to turn boundaries. The assistant message streams into the session as
it generates — there is no separate render feed, and `harness::status` is a
point-in-time recovery read, not something to poll.

## The turn lifecycle

One turn, end to end:

1. **`harness::send`** ensures the session exists, persists the incoming user
   message, and kicks off a turn. It returns fast with
   `{ accepted, session_id, turn_id }`. If a turn is already running in that
   session, the message merges into it or queues behind it (a queued message
   fires the `harness::message-queued` event; the console's edit path uses the
   internal `harness::unqueue` / `harness::edit_queued` to modify a
   still-parked entry). An `idempotency_key` dedupes redelivered webhooks: the
   same key can only create one message (`e_idem_<key>`).
2. **Context assembly.** The system prompt is composed as: optional operating
   mode paragraph, then the identity — the router's operator override if set,
   else the provider-declared prompt, else the harness's embedded default.
   A caller `options.system_prompt` combines per
   `options.system_prompt_strategy`: `override` replaces the identity
   verbatim, `enrich` (default) appends to it. Then the `pre-generate` hooks
   run and may mutate the assembled prompt (this is how fp injects its pipe
   guidance).
3. **Generation.** The model streams; text lands in the session live. Each
   `agent_trigger` the model emits goes through the internal
   `harness::function::trigger`: the dispatch policy is checked
   (`options.functions` is FAIL-CLOSED — absent means every call is denied
   and the session is a plain chat loop), `pre-trigger` hooks may deny, hold,
   or rewrite arguments, the function runs, `post-trigger` hooks may rewrite
   the result, and the normalised result is appended to the transcript.
4. **Loop.** Steps repeat (each step is an internal `harness::turn` job on the
   harness-turn queue, which is what makes the loop durable across restarts)
   until the model produces a final reply, `options.max_turns` is reached, or
   `harness::stop` cancels it. Transient provider errors resume the turn
   (bounded by `max_transient_resumes`); output-contract violations retry
   generation (bounded by `max_validation_retries`).
5. **Terminal status.** A turn ends `completed`, `cancelled`, or `failed`
   (live states are `running` and `awaiting_functions`). The
   `harness::turn-completed` event fires with the outcome.

**Parking.** A pending call parks the turn without burning the loop: an
approval hold (approval-gate `pre-trigger` hook) or a hook hold. The internal
`harness::function::resolve` settles the pending call — delivering a result or
releasing the hold — and resumes the parked turn; a sweep expires stragglers
so an abandoned approval can never park a turn forever. Sub-agent spawns never
park: they are fire-and-forget (below).

## The function surface lives in the API reference

Read contracts at https://workers.iii.dev/workers/harness.md or live
with `engine::functions::info`. Two semantics the schemas cannot express:
`harness::react` is a trigger TARGET only (direct calls are denied - use
the id in `engine::register_trigger` without probing it first), and the
dispatch policy on `harness::send` `options.functions` is FAIL-CLOSED -
absent means every call is denied and the session is a plain chat loop.

## Trigger types the harness publishes

Session lifecycle events, bindable via `engine::register_trigger`:

- `harness::turn-started` — a turn began executing its first loop step.
- `harness::turn-completed` — a turn reached a terminal status. Payload:
  `{ session_id, turn_id, status, terminal, timestamp }` plus `result` (the
  final text) when completed, `result_error` when the completion carries an
  error, `reason`, and `parent` for sub-agent turns. **`terminal: false`
  means the session still owns an armed wake** (a one-shot notify is
  registered) — the run continues and a later turn carries the real outcome;
  finalize only on `terminal: true`.
- `harness::message-queued` — a message was queued behind a running turn.

Both turn-event types take the same binding config (unknown fields are
rejected): `{ session_id }` to follow ONE session, or `{ parent_session_id }`
to follow every child a session spawned. A join predecessor must filter on the
child's own `session_id` — a `parent_session_id` filter matches every sibling,
so the first completion would fill every join key.

Hook points (operator-trusted policy siblings only — agents cannot bind
these): `harness::hook::pre-turn` (veto before any model spend),
`pre-generate` (extend the system prompt, append messages, or veto),
`post-generate` (observe), `pre-trigger` (deny / hold / rewrite arguments,
scoped by `functions` globs), `post-trigger` (rewrite results). Hooks run
synchronously in the turn's critical path, chained by ascending `priority`,
each under `timeout_ms` (default 5000) with `on_error` defaulting to
fail-closed for `pre-*` and fail-open for `post-*`. A worker that injects
prompt guidance binds `pre-generate` with `on_error: fail_open` so its
failure can never block generation.

## How a session registers triggers, and what fires back

`engine::register_trigger { trigger_type, function_id?, config, metadata?,
once? }` returns the subscription id (`engine::unregister_trigger { id }`
removes it). `once` defaults to one-shot for every type EXCEPT `cron`, which
is recurring — a cron used as a deadline needs `once: true` explicitly. Three
reaction forms, by what `function_id` and `metadata` carry:

1. **Plain notify** — no `function_id`. The event is injected into the
   REGISTERING session as a `[notification]` and wakes that agent's next
   turn. This is the only way a session wakes ITSELF; anything that must wake
   you (a turn-complete gate, a deadline) is a plain notify, never a react.
2. **React-spawn** — `function_id: harness::react` with
   `metadata: { task, model?, session_id?, parent_session_id?, options?,
   continue_on_error? }`. On fire, the harness spawns a sub-agent with your
   `task` (the event JSON appended so it sees what fired). `session_id`
   picks WHICH session the reaction runs in: omitted, it falls back to the
   session that REGISTERED the trigger — the reaction delivers back into that
   chat (right for a pipeline's final stage, wrong for any earlier stage).
   Pin a fresh unique id for each stage that must run as its own child; a
   reused id silently RESUMES the old session; never pin a fixed id on a
   recurring trigger. `parent_session_id` only controls console nesting and
   must be a REAL session id. `model` omitted runs the reaction on the
   registering agent's model. Failed/cancelled turns and completions carrying
   `result_error` skip the reaction unless `continue_on_error: true` (set it
   on validator edges — an outcome checker must see failures too).
3. **React-call** — `metadata: { call: { function_id, payload, event_into? } }`
   instead of `task`. The event is injected into the payload at `/event`
   (or `event_into`) and the function dispatches directly: deterministic,
   token-free, no session. The result is DISCARDED — a call can never wake
   you. The target must be allowed by your policy, checked at registration.

**Joins** (fan-in): register one `turn-completed` subscription per
predecessor, each filtered on that predecessor's own `session_id`, each
carrying the SAME `metadata.task` and the same
`join: { id, expect: [keys], key: <own key> }`. The harness accumulates each
predecessor's result durably and spawns the downstream exactly once — when
the last successful predecessor arrives, fed all results — then unregisters
the join's subscriptions. `join.rearm: true` keeps them registered for the
next complete set. A failed predecessor counts as arrived but blocks the
downstream unless its edge sets `continue_on_error: true`.

Built-in loop breakers: a subscription never fires for the completion of the
sub-agent it itself spawned, reactive chains hard-cap at depth 8, and one
subscription is rate-limited to roughly 10 spawns per minute.

## Sub-agent lifecycle

`harness::spawn { task, session_id?, model?, options? }`:

- **Fire-and-forget.** Returns `{ child_session_id, child_turn_id }`
  immediately; the child's result is NEVER delivered back. It reaches the
  world only through the state it writes and the `turn-completed` event its
  finish fires — wire consumers BEFORE spawning. One automatic exception: a
  terminally FAILED child delivers a `[child-failure]` message into the
  parent session, so no failure listener is needed.
- **Naming.** Always pass `session_id`: a short readable slug plus a few
  random characters (`fetch-headlines-b4k9`). Spawn creates the session if
  absent — an id from an earlier run silently RESUMES that session, old
  transcript included.
- **Policy.** An in-turn child inherits the parent's full dispatch policy;
  `options.functions` may only NARROW it (spawns never escalate). If you
  narrow, include everything the task must call — a child without
  `state::set` finishes politely with its work stranded in its transcript. A
  parentless spawn (direct call, CLI, trigger-fired react) has no parent to
  mirror and starts from the read-only baseline; grant explicitly via
  `options` (or react `metadata.options`).
- **Identity.** Children get the embedded sub-agent prompt, never the
  router-served identity: one task, a state destination, nothing else.
  `options.system_prompt` (+ `system_prompt_strategy`) is the escape hatch —
  it is how a five-line special-purpose prompt from the prompt store becomes
  a child's whole identity.
- **Shape.** A child is a LEAF by default; it inherited spawn/register
  permission, so an explicit coordinator task MAY orchestrate its own
  sub-tree, bounded by the spawn-depth limit.
- **Cancellation.** `harness::stop` on the parent cascades to children.

## The mesh around a turn: dependency workers

The harness owns no storage and no model access; every capability is a
sibling worker reached over the bus. What it calls, per dependency:

- **session-manager** — the transcript of record: every message, function
  result, and streaming assistant update lands there, and the harness cleans
  up turn state when a session is deleted. The console renders chats from
  session events, not from the harness.
- **context-manager** — fits the transcript to the model's window before
  each generation (compaction lives here) and prices prompts and messages.
- **llm-router** — streams each generation over a channel, resolves models
  and capabilities, and serves the per-provider identity prompt (operator
  override, then provider-declared, then the harness falls back to its
  embedded default). Provider workers sit behind the router and are never
  called directly. See the llm-router skill for its own semantics.
- **queue** — the harness defines a dedicated `harness-turn` queue at boot
  (required) and every loop step is a job on it; this is what makes turns
  durable across harness restarts.
- **state** — holds the turn records the loop resumes from and the
  webhook-dedupe rows. Agents share the same state surface for their own
  run scopes.
- **engine** — the bus itself: function dispatch, the trigger registry the
  harness's event types and hook types register into, and the function
  catalog that backs discovery.

Optional policy siblings extend a turn without the harness knowing them by
name: **approval-gate** binds the `pre-trigger` hook to hold calls for human
approval; **fp**, **web**, and **memory** bind `pre-generate` to inject
usage guidance or recalled rules into the system prompt while they are
connected. Stop any of them and their contribution vanishes; the harness
keeps running.

## Reading this alongside the playbooks

This doc is mechanics. For conduct — wire-before-spawn ordering,
self-contained child tasks, the `turn_complete` gate, validators, deadlines,
waves, and the armed-wake invariant — read `harness/orchestration` and
`harness/finishing`. For install and worker-authoring flows, read
`harness/building`.
