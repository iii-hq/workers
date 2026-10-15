---
title: ticket-orchestration
description: Dispatch one feature across several agent profiles as kanban tickets — split on the seam, make the feature ticket the parent of one ticket per child, spawn each with a self-contained task, and stay reachable through one parent-level comment wake until the seam is verified.
type: how-to
---

# Orchestrating a feature across agent profiles

You are the one who splits a feature across other profiles. The kanban board is the
only wire between you and them. Everything below exists to keep that wire honest.

## Open these before you dispatch

- **The dispatch contract**: `engine::functions::info { function_ids: ["harness::spawn"] }`.
  Read `task`, `agent`, `options.functions`, and the return shape before spawning anything.
- **The roster you dispatch into**: `directory::agents::get { id: "<owner>", raw: true }`.
  The `skills:` and `functions:` lists are what that child actually starts with — and what
  it therefore does *not* know.
- **The board**: `kanban::board::get { "statuses": [<the active columns>], "compact": true }` —
  once per planning pass, not per wake — and `kanban::config::info {}` for the real column ids,
  the `done_status` column and the key prefix. Add `"status": "done"` with `updated_since` to
  see what landed recently.
- **The mechanics**: the `kanban-tickets` skill — keys vs uuids, `actor`/`author`, the comment
  filters, review-vs-done, never delete. The child's half of the same loop is the
  `ticket-worker` skill, which the engineer profiles preload.

## The ticket is the only channel

`engine::functions::list { prefix: "harness::" }` shows **no `harness::send`** on this
engine. Combined with the spawn contract — "the child's outcome reaches you only through
whatever destination its task names" — that leaves exactly one wire:

1. **The task text is the child's whole brief.** It cannot infer a path, a function id, a
   convention, or who else is working on the feature. Name them literally or it guesses.
2. **The destination must be a ticket.** A comment on the ticket is how the child reports;
   a lane move is what it believes it achieved. Neither is proof — see *Your gate*.
3. **Your answer goes back on the same ticket.** A decision you keep in your own session
   never reaches the engineer.
4. **The return leg has to exist.** The child only hears your answer if it armed a wake on
   the ticket with `author` and `exclude_author` set to its own profile id. Nothing you do in
   your session can arm that for it — name the duty in the task.

## Split on the seam, not on the layer

The failure this role exists to prevent is a ticket pair that both pass and still do not
work together. Prevent it structurally:

- **Contract first.** Exactly one ticket owns each new function id's *schema* — the id, the
  request fields, the response fields — and it must be `done` before the ticket that calls it
  starts. Give the consumer ticket `Depends on: <key>` and do not dispatch it early.
- **One owner per ticket.** A ticket with two assignees has no owner. Work needing both is
  two tickets.
- **Split by outcome.** Each ticket must be verifiable alone, by someone who did not write it.
- **Say what is out of scope.** The cheapest way to stop a child inventing work.

Symptom of skipping the contract ticket: the frontend registers `game::move` with `{ cell }`
while the backend registered `game::play` with `{ row, col }`. Two green tickets, one broken
feature.

## The feature ticket is the parent

Keep one ticket for the feature itself and make it the **parent** of every ticket you
dispatch: create each agent's ticket with `parent` set to the feature's key.

```json
kanban::ticket::create {
  "title": "Backend: register game::play",
  "description": "<the template below>",
  "parent": "KAN-20",
  "actor": "tech-lead"
}
```

- **One level.** The parent must be live and must not have a parent itself; a child cannot
  have children, and a ticket that already has live children cannot be made a child. A feature
  too big for one level is two feature tickets.
- **Nothing is inherited.** `project` and `release` are not copied from the parent. If the
  feature belongs to a project or release, pass `project` / `release` (id or name) on each
  child. You may set them; creating, shipping or converting projects and releases is a human's
  action in the console.
- **Children do not move the parent.** `kanban::ticket::get` on the parent returns `children`
  and `progress: { done, total }` over its live children, and `done` counts only the
  `done_status` column — a child in `in_review` is not done. You move the parent yourself, after
  the seam is verified.
- **Attach or detach** an existing ticket with `kanban::ticket::update { "id", "parent": "KAN-20" }`
  (`"parent": null` detaches). Both tickets get an activity line.
- A deleted parent leaves its children in place; the child's `parent` brief then carries
  `deleted_at`.

## The description template

Put this in every ticket you create — it is the child's contract as much as yours:

```
Deliverable: <the one thing this ticket produces>
Owner: <profile id>
Depends on: <ticket key + the function ids that must already resolve>
Acceptance:
1. <observable statement>
   Verify: <a command, a url, or a screen — something a stranger can run>
2. ...
Out of scope: <what a reader would reasonably assume is included and is not>
```

A criterion with no `Verify:` line is not a criterion, it is a wish — and it comes back to
you as a caveat you cannot adjudicate.

`kanban::ticket::create` takes `title` plus any of `description`, `assignee`, `priority`,
`labels`, `status`, `parent`, `project`, `release`, and `actor`. Create into the *default*
column and let the child move itself, so its claim is visible on the activity timeline.

## Dispatch

Spawn **after** the ticket exists — the task's first instruction is to read it.

```json
harness::spawn {
  "agent": "backend-engineer",
  "session_id": "kan8-backend",
  "display": { "name": "Backend · KAN-8", "icon": "terminal", "color": "blue" },
  "options": { "functions": { "allow": ["kanban::ticket::get", "kanban::ticket::update", "kanban::ticket::move", "kanban::comment::create", "kanban::comment::list", "kanban::config::info", "engine::register_trigger", "harness::triggers::list", "harness::triggers::unregister", "coder::read-file", "coder::create-file", "coder::update-file", "coder::search", "coder::tree", "shell::exec"] } },
  "task": "You own ticket KAN-8 on the kanban board; work it with the ticket-worker skill. Read it with kanban::ticket::get { \"id\": \"KAN-8\" }, then claim it: kanban::ticket::update { \"id\": \"KAN-8\", \"assignee\": \"backend-engineer\", \"status\": \"in_progress\", \"actor\": \"backend-engineer\" }. Before you report, arm the pair the skill describes: engine::register_trigger on kanban:comment with config { \"ticket_id\": \"KAN-8\", \"exclude_author\": \"backend-engineer\", \"ticket\": \"summary\" }, plus the kanban:change done-watch labelled KAN-8-teardown — and reap every KAN-8-* binding when the ticket reaches done. Do the work in <path>. The function ids to register are ... When finished, report with kanban::comment::create { \"ticket_id\": \"KAN-8\", \"author\": \"backend-engineer\", \"body\": \"<exactly what you ran and what it returned>\" } and then kanban::ticket::move { \"id\": \"KAN-8\", \"status\": \"in_review\", \"actor\": \"backend-engineer\" }. Do not delete the ticket."
}
```

- `agent` is a profile id from `directory::agents::list`, never a display name.
- One fresh `session_id` per ticket: the console tree stays legible, and the child cannot
  inherit another ticket's context.
- The task names the ticket key, the exact claim/report/hand-off calls, the arm-the-pair duty,
  and the stop condition. Assume the child arrives knowing nothing but its own profile.
- The engineer profiles preload the `ticket-worker` skill and the ticket loop's function ids,
  so the brief can name the ticket, the owner, and the stop condition and let the skill carry
  the call order. Name the ticket key and the owner literally anyway — the child cannot infer
  which ticket it owns.
- **Preloading is not permission.** `options.functions` is the fail-closed dispatch policy,
  intersected with your own and never escalating; absent, every call is denied. Pass
  `options.functions.allow` with the ids the child needs, spelled out one per entry — no
  wildcard: the six `kanban::` calls above, `engine::register_trigger`,
  `harness::triggers::list`, `harness::triggers::unregister`, and its own toolchain — or it
  answers "I cannot call any function." `engine::register_trigger` is the entry you must never
  drop: without it the child cannot arm its `kanban:comment` wake, and every answer you post on
  the ticket reaches nobody.
- `harness::spawn` returns `{ child_session_id, child_turn_id }` immediately. That means the
  child *started*. It never means the child finished.

## Stay reachable

Arm the wake **before** the turn that dispatches. With the feature ticket as parent, one
binding on the parent covers every child, the feature ticket included:

```json
engine::register_trigger {
  "trigger_type": "kanban:comment",
  "config": { "parent_id": "KAN-20", "exclude_author": "tech-lead", "ticket": "summary" },
  "once": false,
  "label": "KAN-20-wake",
  "lifecycle": { "max_fires": 50, "expires_in_ms": 3600000 }
}
```

`parent_id` fires for a comment on the parent or on any of its children (a child attached
later is covered too), so you do not arm one binding per child. Each payload's `ticket_key`
says which ticket was commented on.

- `exclude_author` must equal the id you pass as `author` on your own comments, character for
  character. One typo and your own reply re-enters your handler.
- Leaving `exclude_author` off wakes you on every comment, your own included — only safe when
  your handler never comments back on that ticket.
- Bound the life. A bare `once: false` with no `lifecycle` is the binding that outlives its
  ticket and wakes a session with nothing to do.
- Label every binding `<KEY>-<purpose>` (the parent's key for the parent-level ones) so the reap
  is mechanical, not a judgement call.
- Pass `"ticket": "summary"` on every binding you arm: a wake then costs a board card, not the
  whole thread, and you re-read the ticket before acting anyway.
- Pair it with a `cron` check-in reading
  `kanban::comment::list { ticket_id, exclude_author, since }` for the parent and each open
  child (the list takes `ticket_id`, not `parent_id`). A wake the kanban worker never
  emits reports **nothing at all** — silence looks exactly like "no news". The cron sweep goes
  through the board file and cannot fall silent with the worker's emit path.
- Without a parent, `kanban:comment` fires per ticket, so N tickets need N bindings. Parent the
  tickets and use `parent_id` instead: one wake binding and one done-watch for the whole
  feature. `harness::triggers::list` audits what is actually armed;
  `harness::triggers::unregister` retires the wiring once it lands.
- **Arm a done-watch with every wake** — `kanban:change` with
  `config: { "parent_id": "<PARENT>", "events": ["ticket.moved", "ticket.updated", "ticket.deleted"], "ticket": "summary" }`,
  labelled `<PARENT>-teardown` (for a lone ticket, `ticket_id: "<KEY>"` and `<KEY>-teardown`).
  `ticket.updated` and `ticket.deleted` matter as much as moves: detaching a cut child or deleting
  one also changes `progress`, and it can be the change that completes the parent.
  On each fire read the parent with `kanban::ticket::get`; when `progress.done == progress.total`
  and the parent's own `status` is `done`, unregister every `<PARENT>-*` binding, the done-watch
  last so it stays alive while you reap. The fire also arrives for children's moves and for the
  parent's own, so the first act is always that read.
  Nobody can reap for you: `engine::unregister_trigger` removes only bindings owned by the
  calling session, a foreign `session_id` is not honoured, and agent wakes do not appear in
  `engine::registered-triggers::list` at all. Your engineers reap their own on the same event;
  the task must tell them to.
- `kanban:change` is board-wide and filters by ticket and event only — no author filtering.
  Never use it where you need to avoid waking on your own comments.

When a wake arrives: read the ticket, decide, answer on the ticket. Re-read the ticket
first and no-op unless it is still open — a re-armed or late-firing binding can deliver a
comment for work that is already closed.

## Fan-in: knowing the children actually stopped

Two signals, and you need both. The ticket side:

```json
kanban::ticket::get { "id": "KAN-20" }
```

"All children done" means the parent's `progress.done == progress.total` — every live child sits
in the `done_status` column. A child in `in_review` is not done, and a deleted child is not
counted. If a child is cut from the plan, comment on it that it is cut and detach it
(`"parent": null`); do not leave it open and call the feature done. The session side:

```json
harness::metrics { "root_session_id": "<your session id>" }
```

`complete` is true only once every session in your durable tree reached a terminal turn.
That is the "all my children are done" signal for the sessions — not a guess, not a timeout.

- `harness::session-tree { root_session_id }` — the shape of the run, root and descendants.
- `harness::status { session_id }` — one child's current turn (lean; `verbose: true` for the
  full report and untruncated result).

## Traps

- **The child reports success and the feature does not work.** Each ticket was checked against
  its own contract, not against the seam. Someone must exercise caller and callee together;
  that check belongs on a ticket, not in a summary.
- **A spawned engineer answers "I cannot call any function."** The dispatch policy on the spawn
  was fail-closed. Pass `options.functions` explicitly, with the ids it needs in `allow`.
- **The child reports and your answer wakes nobody.** The child never armed a wake on the
  ticket, so your review comment lands on a session whose turn already ended. Cause: the task
  named the claim and the hand-off but not the arm-the-pair duty. Fix: say it in the task, and
  check the ticket's reply flow on a first small ticket before trusting the pattern.
- **`done` and the wakes keep firing.** The ticket landed and the bindings armed on it were
  never reaped, so sessions get woken with nothing to do. Cause: a wake armed without its
  done-watch. Fix: arm the pair together, and label every binding `<KEY>-<purpose>`.
- **The parent is `done` with open children.** A parent does not follow its children; the console
  flags it. Cause: you moved the feature ticket before `progress.done == progress.total`. Fix:
  move it back to `in_review` and finish or cut the open children first.
- **A child has no parent and the parent-level wake never fires for it.** Cause: the ticket was
  created without `parent`. Fix: `kanban::ticket::update { "id": "<child>", "parent": "<PARENT>" }`.
- **You dispatched against a stale dependency.** The upstream function id or schema changed
  while the consumer ticket sat in the queue. Re-read the upstream ticket immediately before
  dispatching the consumer, not when you wrote the plan.
- **Two tickets editing one file.** Parallel children have no merge protocol here. If the split
  has both touching `package.json`, the route tree, or one shared module, it is one ticket.
- **You rewrote another profile's report.** Comment as yourself; never edit what a child
  claimed.
- **A child spawned with a tiny `options.max_turns` strands mid-task** and reports nothing.
  Omit it unless you have a reason.

## Checklist

- [ ] The feature ticket is the parent: every dispatched ticket was created with `parent` set
      to it (and `project` / `release` set explicitly where the feature has them).
- [ ] Every ticket names one owner, one deliverable, its dependency by key, and criteria with
      literal `Verify:` targets.
- [ ] The schema-owning ticket is `done` before any consumer ticket is dispatched.
- [ ] Each child's task names the ticket key, the claim/report/hand-off calls, the arm-the-pair
      duty, and the stop condition — with no reference to anything only you can see.
- [ ] The spawn carries `options.functions.allow` with the ids the child actually needs, named
      explicitly.
- [ ] One `kanban:comment` binding with `parent_id` and `exclude_author` set to your own id is
      armed, bounded by `lifecycle`, labelled `<PARENT>-wake`, with a `cron` backstop.
- [ ] The wake has its `kanban:change` `parent_id` done-watch labelled `<PARENT>-teardown`, and
      every binding is unregistered once the parent's `progress.done == progress.total` and the
      parent is `done`.
- [ ] The parent's `progress.done == progress.total` and `harness::metrics { root_session_id }`
      reports `complete: true` before you claim the fan-out finished.
- [ ] The seam was exercised end to end — caller and callee in one run — and the evidence is
      in a ticket comment, not in your summary.
