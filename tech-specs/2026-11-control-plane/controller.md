# Project controller

Status: phases 1 and 2 (the controller chat) built 2026-11-24; phase 3 (autonomous orchestration) blocked on the open problems below. Revision 4.

It was split from revision 3 of `control-plane-worker.md` and builds on `README.md`
and `control-plane-worker.md`. It is implemented after `control-plane-worker.md`, and gets
one more review before that.

## Roles

- **The person (CEO)** sets direction from the control plane. They answer what is
  escalated to them, and audit what the controller decided.
- **The controller** is one agent per project. It plans and coordinates the
  project over the board.
- **Work sessions** are one root session per card, started by board
  dispatch. They are never children of the controller.
  - A crashed controller leaves running work running.
  - `max_depth` 3 stays enough: controller → card → phase.

## Why

- **A person cannot steer many sessions one by one.** With one controller
  per project:
  - the person directs and decides what only they should decide;
  - the controller coordinates the rest;
  - the control plane shows the exceptions.
- **Today any agent can start or stop other work.** It needs only
  `harness::stop` or a kanban write. That includes deepwork and workflow,
  which run with `allow ["*"]` from dispatch (`kanban/src/dispatch.rs:430`).
  This spec enforces the controller's role on the board and in the harness,
  not in prompts.

## Decisions

These are defaults, open to review.

- **C1. The board is the controller's only lever.**
  - To start a card, it moves the card to To do.
  - To stop one, it moves the card out of In progress.
  - To redirect one, it updates the card and moves it back to To do. The
    new session continues from what is already committed (deepwork resume).

  The controller gets neither `harness::send` nor `harness::stop`, so the
  repository rules `'!harness::send'` and `'!harness::stop'`
  (`iii-permissions.yaml:211`) stay.
- **C2. Questions stay harness asks.**
  - Work sessions keep `harness::ask`. deepwork and workflow need it for
    Design and plan approval (`kits/deepwork/agents/deepwork.md:16,81`,
    `kits/workflow/agents/workflow.md:19,214`).
  - The control plane's `escalate` list decides what is Now for the person
    (`control-plane-worker.md` §7).
  - The controller never answers a question: `harness::ask::answer` stays
    closed to agents.
  - The controller records its own routine decisions with
    `kanban::question::record`, and asks the person with `harness::ask`.
- **C3. The writing session decides, never the `actor` text.**
  - `actor` is caller-supplied text (`kanban/src/functions.rs:147`).
  - Every check reads the session that made the write: the activity
    entry's `session_id`, from baggage (`kanban/src/store.rs:80-86`,
    `kanban/src/board.rs:715`).
  - A write with no session comes from the console, a worker, or a CLI
    call.
- **C4. Stopping is graceful first.** A stop asks the card's session to
  finish at a safe point, and forces it only after a grace period.
  - `harness::stop` cascades and kills phase jobs.
  - deepwork itself forbids stopping a running batch lead. A suite that
    faults a shared system would then skip its restore (`deepwork.md:180`).

## Design

### 1. The controller session

**Profile.**
- `Project.lead` names the controller profile: an agent profile id, or
  `user` for none (`kanban/src/store.rs:155-157`).
- `kits/workflow` fits, or a new `controller` profile.
- deepwork does not fit: it needs `shell::exec`, `state::*` and
  `harness::link-card`, which this policy leaves out.

**Creation**, on the first direction the person sends, in this order:
1. The page calls `session::ensure` as the console. The id is fresh and
   unguessable, `lead-<project id>-<uuid>`, so no agent can occupy it
   first. The metadata is `{origin: "controller", project_id}`.
2. The page calls `kanban::project::update {controller_session}`. Kanban
   accepts the field only:
   - from a write with no session;
   - for a session that exists, has no `parent_session_id`, and has no
     turn yet.

   An agent can neither set it nor change it. The profile cannot be
   checked here, because `agent_profile` is stamped on the first send
   (`harness/src/functions/send.rs:727-737`).
3. The page sends the direction as a console `harness::send`, with:
   - `options.agent` = `Project.lead`;
   - `options.functions` = the policy below;
   - `max_total_tokens` and `max_cost_usd` (§5).

   Later directions omit the budget fields. They are frozen per root, and
   resending them with another value fails (`harness/src/budget.rs:171-185`).

**Helpers.** Sessions the controller spawns are its descendants. They count
as the controller everywhere below.

**Policy** (`options.functions`, allow list):
- kanban:
  - reads, plus `kanban::ticket::create|update|move`;
  - `kanban::comment::create`, `kanban::question::record`;
  - `kanban::project::get`, and the `kanban::release::*` reads;
- `control-plane::*` reads, and their sources (`control-plane-worker.md` §2):
  - `harness::ask::list`;
  - `session::list`, `session::messages-tail`;
  - `harness::status`;
  - the `github::pr::*` reads, named one by one (not the wildcard, which
    includes writes);
- harness: `harness::ask`, `harness::spawn`, `harness::observe`,
  `harness::unobserve`.

**Waking.** The controller arms one standing `kanban::on-change` observer:
- events `comment.created`, `ticket.moved`, `ticket.updated` and
  `ticket.deleted`;
- no ticket (`kanban/src/events.rs:76-98`), filtered on the payload's
  `ticket.project_id` (`kanban/src/board.rs:98,166-168`).

**No echo.** The change event's payload carries the writing session's
`session_id`; kanban adds it where it is missing.
- The filter drops events from the controller's own tree.
- It drops `comment.created` and `ticket.updated` from work sessions (their
  progress lines).
- It keeps everything from people and dispatch, and every `ticket.moved`.
- Deliveries are coalesced to at most one wake per
  `controller.wake_debounce_s`.

**What the controller learns from the board:**
- **A run's outcome:** from `ticket.moved`. The card's own tree moves it to
  review or done, or dispatch parks it (`dispatch.rs:449-465`). Dispatch
  never moves a card to done.
- **A session ended after its card was already moved:** this produces no
  event. The controller does not need it.

### 2. Kanban

#### 2.1 Dispatch's own writes

Dispatch's writes are made with no session, and carry `actor:
"kanban-dispatch"`.
- A write that carries a session and sets that actor is refused
  (`reserved_actor`).
- `reconcile` and `sweep` already run in `tokio::spawn`, without a session
  (`dispatch.rs:511,862`).
- The `on_final_turn` handler runs inside the finishing turn's baggage
  (`harness/src/functions/turn.rs:53-83`). It now wraps its writes in
  `iii_sdk::scope(None, …)`, so its park is not refused.

#### 2.2 No session through a binding

A binding's target is delivered with a plain `iii.call`, without the owner's
session (`harness/src/functions/observer_deliver.rs:552-560`).
`unstamped_target_refusal` (`harness/src/functions/observe.rs:453-465`) also
refuses, as binding targets:
- `kanban::ticket::create`, `update`, `move`, `delete` and `restore`;
- `kanban::project::*`.

No kit arms such a binding today.

#### 2.3 Starting a card

For a project with a `controller_session`, dispatch starts a card only when
the write that made it eligible came from one of:
- the controller's tree;
- a write with no session that is not dispatch's own (§2.1).

`trigger_actor` (`dispatch.rs:220-233`) returns that entry, not its actor
text. A project without a controller keeps today's rule.

#### 2.4 Stopping a card

A stop starts when a card dispatch claimed leaves In progress, or is
deleted, by the controller's tree or by a write with no session that is not
dispatch's own. A move by the card's own tree (§3.1) starts none: deepwork's
Step G.5 moves R and then still notifies and reports (`deepwork.md:353,362-363`).

1. **The target.** The stop targets the claimed session, `dispatch-<key>-<seq>`.
   When a chat session took the card over, it targets the
   `Orchestrator session:` that adopted it (`dispatch.rs:257-275`).
2. **A stop request.** Dispatch, as a worker, sends the target "stop
   requested: <reason>" with `harness::send`. It writes a `stop_pending`
   record `{session, requested_at}` and a `dispatch.stop_requested`
   activity entry.
3. **The grace period.** The session has `dispatch.stop_grace_min` (default
   30) to end by itself. deepwork honors the request:
   - it never stops a running batch;
   - once the batch has a result, it parks the remaining cards
     `unfinished` and ends with `stopped`.
4. **The forced stop.** After the grace period, the sweep (`dispatch.rs:883-889`)
   calls `harness::stop` again until `harness::status` shows the session is
   no longer running. Dispatch then deletes the record and writes
   `dispatch.stopped`. `plan` reads that entry, so it never emits the same
   stop twice.
5. **Capacity.** A card with `stop_pending` counts as running for
   `max_running` (`dispatch.rs:316-327`).
6. **The card's place.** It stays where the mover put it. `park_stopped` is
   not used.
7. **Moved back to To do.** Its `Start` waits until the stop is confirmed.
   The card stays queued meanwhile, so the deepwork Guard (`deepwork.md:179`)
   never sees two live roots.

#### 2.5 Who may change a claimed card

For a project with a controller, a card with a live claim accepts the writes
below only from the controller's tree, the card's own tree (§3.1), or a
write with no session:
- a status change;
- delete or restore;
- a change of `project_id` or `assignee`.

Any other writer gets `not_card_owner`. Subtasks have no claim, so the rule
does not touch them.

### 3. Harness

#### 3.1 Card tree

A card's tree is the claimed session (§2.4.1) and its descendants
(`session_tree`).
- Only dispatch decides who is in it, through the claim.
- A `harness::link-card` made before or after the claim adds no one.

`harness::link-card` on a ticket with a live claim is refused unless the
caller is in that card's tree.

#### 3.2 Reserved session ids

Ids that start with `dispatch-` are reserved for calls with no session.
- The harness refuses to create such a session from an agent caller,
  through `harness::spawn` or `harness::send`.
- Today `harness::spawn` creates any missing id as a child of the caller
  (`harness/src/functions/spawn.rs:141-144`). A dispatched run could then
  become an agent's child, and its budget would fail on an "existing
  unmetered session" (`budget.rs:161-166`).

#### 3.3 Stop and send scope for carded agents

The scope applies when an agent turn calls `harness::stop` or
`harness::send` and the caller is in a card's tree. The target must then be
one of:
- a descendant of the caller;
- a session whose root links the same ticket (`metadata.kanban_ticket`).
  This covers deepwork's resume, which stops `<old>` and `<old>-<key>`
  (`deepwork.md:180`).

Enforcement:
- `harness::stop` gets its own branch in the pre-call hook. It stamps the
  caller as `_caller_session_id`, because its `session_id` is the target
  (`harness/src/functions/observe.rs:362-379`, `stop.rs:14`).
- Any other target is refused with `outside_card_tree`.
- For `harness::send`, a new session id is outside the tree, so it is
  refused.
- Callers outside every card tree keep today's behavior, and so do workers
  and the console.

Kits:
- **deepwork:** it already links R when it creates it (`deepwork.md:153`).
  A lead stops its own phases.
- **workflow:** its nodes stay in the caller's tree through the stamped
  `caller_session_id`. The node default already denies `harness::send`
  (`workflow/src/functions/tick.rs:168`).

### 4. Control plane additions

- `control-plane::overview` and `control-plane::runs::list` gain `controller`:
  - its session and state;
  - the last direction sent, from its transcript;
  - the budget left (§5).
- Runs pins the controller first, with Resume.
- The Overview has a direction box (§1 Creation).

### 5. Budget and wakes

The harness's `max_total_tokens` and `max_cost_usd` bound a root's whole
tree, and the ledger is frozen per root (`harness/src/budget.rs:156-199`).
They are the controller's budget for its life.

| Key | Default | Meaning |
|---|---|---|
| `controller.max_total_tokens` | 20,000,000 | the controller's budget for its life |
| `controller.max_cost_usd` | 200 | the controller's budget for its life |
| `controller.wake_debounce_s` | 30 | at most one wake per window (§1) |
| `dispatch.stop_grace_min` | 30 | §2.4 |

- A spent budget does not end the session. Its next turn fails at
  reservation (`budget.rs:238-287`).
- On that failure, the control plane shows "controller budget spent" with Renew.
- Renew runs §1 Creation again, which records a new session. Running cards
  are not touched.
- The dispatch rule's `max_total_tokens` still bounds each card.

### 6. Limits

**Kept:**
- `max_depth` 3, and `max_children` 8 per turn;
- `dispatch_timeout_ms`;
- sub-agents cannot ask;
- `agent_call_forbidden`, and child ⊆ parent policy;
- approval holds on push, merge, deploy and outside sends;
- the side chat ceiling and the file jails.

**Known gap, out of scope.** A work session with `shell::exec` can call any
function through the `iii` CLI.
- That call carries no session, so it counts as a person in §2.
- Closing it needs a sandbox without engine access, which is a separate
  change.

## Migration (new commits on #1855, after `control-plane-worker.md`)

1. **Harness:**
   - the stop hook branch, `_caller_session_id` and the card-tree scope;
   - the reserved `dispatch-` ids;
   - kanban writes refused as binding targets;
   - `link-card` refused on another tree's claimed ticket.
2. **Kanban:**
   - `controller_session` and its check;
   - `session_id` on change events;
   - the reserved `kanban-dispatch` actor;
   - `on_final_turn` without a session;
   - the session-based start;
   - the graceful stop: `stop_pending`, grace, sweep, `max_running`, no
     repeat;
   - the claimed-card write check.
3. **deepwork:** honor "stop requested", finishing a running batch first.
4. **Control plane:** `controller` in the read model, Runs, the direction box and
   Renew.
5. **Docs:**
   - the kanban skill covers the controller and the dispatch rules;
   - the harness README covers the card-tree scope and the reserved ids.

## Validation

On an isolated stack:

1. **Start**
   - A work session moving a card to To do with `actor` set to the
     controller's name starts nothing.
   - A session's write with `actor: "kanban-dispatch"` is refused.
   - The controller's move starts the card, and so does a console move.
   - A helper the controller spawned starts it too.
   - A binding whose target is `kanban::ticket::move` is refused at
     creation.
2. **Recording**
   - An agent cannot set `controller_session`.
   - The console cannot record a session that has a parent or has already
     run.
   - The first direction runs after recording.
3. **Stop**
   - The controller moving a card out of In progress sends a stop request.
   - deepwork with a running batch ends after the batch, and its shared
     system is restored.
   - A session that ignores the request is stopped after the grace period.
   - A failed forced stop is retried.
   - `stop_pending` counts toward `max_running`.
   - The stop is not repeated.
   - The card stays where it was moved.
   - Moving it back to To do starts a new session only after the old one
     has stopped.
   - Deleting a claimed card from the console stops its session.
   - A deepwork run moving its own card to review is not stopped.
   - A dispatched turn that finishes in progress is parked at once, not at
     the next sweep.
4. **Ownership**
   - Another card's session cannot change a claimed card's status, delete
     it, or move it to another project.
   - The card's own tree can.
   - A session that ran `link-card` on the card while it was in To do is
     not in its tree.
5. **Scope**
   - A carded agent cannot stop or send to another card's session, or send
     to a new session.
   - An agent cannot spawn `dispatch-*`.
   - deepwork's resume stops the old root and its leads.
   - A lead stops its phases.
   - A chat session with no card keeps today's behavior.
6. **Waking**
   - The controller's own comments and updates do not wake it.
   - A burst of moves wakes it once per window.
7. **Questions**
   - Work sessions still ask through `harness::ask`, and the questions
     reach the control plane.
   - The `escalate` split holds.
   - The controller cannot answer.
8. **Resilience**
   - Stopping the controller does not stop running cards.
   - After a spent budget, Renew records a new session and leaves cards
     running.

## Built: the controller chat (phases 1 and 2, 2026-11-24)

The person talks to the controller; it does not run on its own. None of the
open problems below applies, because the controller claims no exclusive
authority: every turn is started by the person, and every write is confirmed
by them.

- **The session.** `control-plane::controller::open {scope}` returns one
  session per scope, created on first use (and again after the person deleted
  it) with a fresh `lead-<scope>-<uuid>` id, title `Controller · <name>`, kind
  `user`, and the controller model in its metadata so the console chat
  continues on it. The id is kept in the control plane state
  (`controller:<scope>`), not on the project; kanban enforcement (§2) waits
  for phase 3.
- **The first send** freezes the `control-plane-controller` profile (kit
  `iii/control-plane` 0.3.0) and the function policy, and asks for a short
  status. Later messages come from the console chat without a policy, so the
  harness keeps the first one.
- **The model:** `controller.model` and `controller.reasoning` in the
  settings; unset, the summary's.
- **The policy:** unrestricted for now, at the person's request
  (2026-11-24): `allow ["*"]` in every scope. The repository's permission
  rules still deny agents `harness::send`, `harness::stop`, `harness::run` and
  `harness::compact`. The board-only policy (C1, C2) returns with phase 3.
- **Confirmation:** before any write or side effect, the profile lists the
  exact actions in one `harness::ask` (Do it, Change, Cancel); a message from
  the person that names the exact action is its own confirmation. Moving a card to To do
  starts its run through dispatch; moving a running card out of In progress
  does not stop its session, and the controller says so.
- **The page:** a chat button in the header opens the scope's controller in
  the console chat; a failure (no model) shows why.

Phase 3 adds the wakes (§1 Waking), the budget (§5) and kanban's checks on
the writing session (§2), after the harness primitives below exist.

## Open problems (review 4, 2026-11-22)

This spec is not built in PR #1855. The fourth review found four blockers.
They all need harness primitives before the controller can be built:

1. **Forgeable lineage.**
   - `harness::send` accepts `session.metadata.parent_session_id` from its
     caller (`harness/src/functions/send.rs:695-719`), and the tree helpers
     trust it (`session_tree.rs:46-88`).
   - The stop target can come from the editable `Orchestrator session:`
     text (`kanban/src/dispatch.rs:257-275`).
   - Needed: lineage the harness records itself, and a claim owned by
     dispatch, with an explicit adoption operation.
2. **Missing baggage does not prove a person.**
   - Approved calls run outside the turn with no session baggage
     (`harness/src/deferred.rs:374-427`).
   - `fp::batch` spawns without the invocation context
     (`fp/src/batch.rs:247-257,367-378`).
   - Needed: positive provenance on privileged writes, carried across
     forwarding, and checked at the mutation.
3. **The stop request is not durable.**
   - A worker `harness::send` merges into an active turn, or starts a turn
     on a parked session (`send.rs:1080-1140`).
   - deepwork would read it as steering or as an answer.
   - Needed: an authenticated, durable stop request that takes precedence
     over ask handling. It must block new waves and be acknowledged after
     the batch is cleaned up. A forced stop must also reach quiescence across
     the whole tree and retire its wakes.
4. **Reserved `dispatch-` ids break deepwork.**
   - deepwork spawns `<own>-kan-12`, which starts with `dispatch-` under
     dispatch (`kits/deepwork/agents/deepwork.md:233`).
   - Needed: reserve only dispatcher root ids.

Majors from the same review:
- **Claims:** claim generation and capacity buckets are not persisted
  together with the mutation, and project changes through reparenting or
  `label::convert` are missed.
- **Wakes:** the wake filter would drop dispatch parks sent as
  `ticket.updated`.
- **Profile:** `kits/workflow` needs `shell`, `state` and `workflow::start`,
  so it does not fit the controller policy.
- **Workflow nodes:** they can be launched or stopped through worker calls.

## Open decisions

- **Default profile:** a new `controller` profile. `kits/workflow` needs
  `shell`, `state` and `workflow::start`, which this policy leaves out.
- **Answering for the person:** the controller could one day answer
  questions of non-escalated classes. That would need a delegated answer
  path, and is not in this spec.
- **Live redirection:** the controller could get `harness::send` to its
  project's work sessions, instead of restarting them from the board.
