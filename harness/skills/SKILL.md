---
name: harness
description: >-
  The durable agent turn loop — kick off a turn with `harness::send`, render
  it from session-manager transcript events, observe
  `harness::on-turn-completed` from a worker (agents watch state instead),
  with deny-by-default tool dispatch and synchronous hook points for policy
  siblings.
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
- Chain turns or react to outcomes from a worker by observing
  `harness::on-turn-completed` with the SDK's `iii.observe`. An in-run agent
  cannot observe turn events; see [Agent observers](#agent-observers).
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
  and survive). It does not end a session parked on a wake, which ends on its next
  wake, deadline included. A stop that means no more turns passes
  `quiet_deadlines: true`: that wake's deadline then starts no turn, and the session
  ends as cancelled.
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
  on the question and the answer arrives as the user's next message, one
  `Header: answer` line per question (`(skipped)` for a question the user
  passed on; a dismissed card sends a notice instead). Refused in
  sub-agents, structured-output turns, and for a second ask in one step.
- `harness::set-description` — keep the conversation's one-line description
  (and, once, its name) up to date. Offered as a tool of its own in every turn
  while the Chat setting `descriptions.agent_writes` is on; answered by the
  turn loop for the turn's own session, so a direct call fails. Call it with
  other calls or after the reply text: a step whose only call it is, beside
  reply text, asks the model once more (end with no text when the reply was
  the answer; that reply is then the turn's result). A description the
  user wrote is never replaced (`The user wrote this description; leave it.`).
- `harness::projects::list { version: 2 }` — the chat projects: named,
  coloured groups of folders; a conversation's `metadata.project_id` names
  its project (`"none"` = kept out on purpose). Always send `version: 2`;
  without it the call answers the saved folders for apps released before
  projects. Internal like every harness control-plane function (off the
  default `engine::functions::list`), but agents may call it by id.

Console and app control-plane (internal, list them with
`engine::functions::list { include_internal: true }`): `harness::folders::list`
/ `upsert` / `delete` (the saved-folder catalogue, formerly
`harness::projects::*`), `harness::projects::create` / `update` / `delete` /
`assign` (refusals are one JSON object in the error: `{code, message}` with
`name_taken`, `folder_taken`, `bad_name`, `bad_color`, `no_folders`,
`too_many_folders`, `folder_not_absolute`, `not_found`), `harness::context-items`
(the context pane: subagents, pull requests, links, attachments, images and side
chats of a conversation, newest first), `harness::chat-settings::get` / `set`,
and the side chats: `harness::side-chat::open { session_id, entry_id?, quote? }`
(fork a conversation at an entry into a side chat that runs on its model,
effort and account, locked by `harness::send`: a send naming other options
fails `side_chat_locked`), `harness::side-chat::close { session_id }`
(`deleted: true` when nothing was sent in it), `harness::side-chat::keep
{ session_id }` (an ordinary conversation in the conversation's project) and
`harness::side-chat::summarize { session_id }` (a fixed summary prompt; the
reply's entry id joins `side_chat_summaries`). Their refusals:
`nested_side_chat`, `nothing_to_fork`, `side_chat_locked`, `side_chat_busy`,
`not_side_chat`, `not_found`. Agents are denied the writes and the side-chat
functions.

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

A `harness::hook::on-post-call` target may move the working directory by
answering `mutations.fs_scope.root` on a `shell::*` / `coder::*` call; the
`ide` worker does so after a `git worktree add`, unless the call passes
`follow_worktree: false` (and back, after removing the worktree the session is
rooted in). Only a `shell::*` / `coder::*` target is heard: a filter, not
authentication. The step's later calls and later turns
run in the new root, and session metadata gets `fs_scope { root, moved_from }`.
Under `workspace` an approved `git worktree add` moves the boundary too, for
sub-agents as well. A send that names a root still wins, so a client must
adopt the mirrored root. A re-tasked sub-agent keeps its own root unless the
spawn names one.

## Attachments

A file attached to a message is stored by `session-manager` and rides the
message as a `file` block (the text mentions it as `@attachment(id="a_…")`).
The model never gets the bytes or the raw block: each `file` block reaches it,
in place, as one text block whose attribute values are escaped as XML:

```text
<attachment id="a_…" name="report.pdf" mime="application/pdf" size="1234567">
Read it with session::read-attachment {"session_id":"<sid>","attachment_id":"a_…"}
</attachment>
```

Read a file by making that call when the task needs it; nothing is inlined.
`session::read-attachment` answers the characters of a text file, or of an
office document converted to markdown, in windows (`offset`, `next_offset`),
and for a PDF, an image or any other file `how_to_read`: the exact `fp::pipe`
calls that convert it to markdown (`pdf::to-markdown`) or save a copy in a
temporary folder outside the working folder for your own tools (unzip an
archive, run a script over data), or the worker to install when one is
missing. When the message also carries the file's picture, the block says so
before the call. The persisted transcript keeps the `file` block.

## Turn events

The harness hosts private functions that consumers and sibling workers observe
with the SDK's `iii.observe` instead of polling `harness::status`. Each returns
the event it is called with, so an observer with `capture: ['output']` receives
the `Observed` envelope with the event at `output`. They are not agent-observable: `harness::observe` refuses
them (see [Agent observers](#agent-observers)).

- `harness::on-turn-started` — a turn began executing (first loop step).
- `harness::on-turn-completed` — a turn reached a terminal status
  (`completed` / `cancelled` / `failed`), carrying the result or error for
  chaining, failure toasts, auto-titling, and result delivery; `terminal` is
  `false` while the session still owns an armed wake. A turn that ended with a
  wake armed and was then stopped completes twice: `terminal: false`, then
  `terminal: true` when its last wake is gone.
- `harness::on-message-queued`, `harness::on-observers-changed`,
  `harness::on-compaction`, `harness::on-session-tree-deletion` and
  `harness::on-ready` — see the worker README for their events.
- `harness::projects::on-changed` — a chat project was created, updated or
  deleted (`{ revision, project_id, change }`); refetch
  `harness::projects::list { version: 2 }`.
- `harness::chat-settings::on-changed` — the Chat settings changed
  (`{ settings }`).

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
// Worker code (iii-sdk), not an agent tool call.
iii.registerFunction('myapp::on-turn-done', handler)
iii.observe(
  {
    function_id: 'harness::on-turn-completed',
    filters: { 'input.session_id': { $eq: sessionId } },
    capture: ['output'],
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
deadline.

Agents cannot observe the turn events. `harness::observe` refuses
`harness::on-turn-completed`, `harness::on-turn-started` and every other
`harness::*` function with `harness/invalid_request: cannot observe harness
function ...: turn events are not agent-observable`. The two exceptions are
`harness::on-timer` and `harness::hook::on-post-turn` with a validator as the
target, scoped to the calling session or the children it names under the
`<session_id>-` prefix (see [Hooks](#hooks-policy-siblings-only)).

Watch what the work writes instead. To wait for a sub-agent, name a state key
in its brief, have it write its result there, and observe that key with a
deadline before you `harness::spawn` it, so a fast child cannot finish
unobserved and a failed one cannot park you forever:

```json
harness::observe {
  "function_id": "state::on-changed",
  "filters": { "input.scope": { "$eq": "run-x" }, "input.key": { "$eq": "child-a" } },
  "label": "child-a-done",
  "lifecycle": { "expires_in_ms": 600000 }
}
```

Set `lifecycle.expires_in_ms` on every wake except a timer, to how long the work
should take: `once` bounds deliveries, not time, so a one-shot wake without it
expires after 10 minutes (`harness::on-timer` and `cron::on-tick` fire by
themselves and get no default). When the deadline passes first, the session is
woken with an expiry notice instead of the write. The child may still be running:
to keep waiting, re-arm first, then read the key with `state::get`, because a
write made while nothing is armed never reaches the new observer. Retry, report,
or continue without the child only once you know it failed. A `state::barrier`
never sees those writes, so its deadline must outlast the whole fan-in. A wake whose
session was stopped mid-turn, or while parked with `quiet_deadlines`, retires at its
deadline with no notice. For a fan-out, arm one wake per
child key, each with a deadline, and read each result with `state::get` once
woken. A wake is once by default, a call observer is standing; bound standing
observers with `lifecycle` and remove them with `harness::unobserve { id }`.
Nothing throttles an observer, and a cycle routed through a state write
re-enters at depth 0 and every lap is a paid turn: design reactions acyclically.
See `architecture/reactive-observers.md` for the full payload.

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
hooks. An agent may observe `on-post-turn` for its own session or for the
children it names under the `<session_id>-` prefix, with a validator as the
target.
