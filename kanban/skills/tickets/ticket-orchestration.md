---
title: ticket-orchestration
description: Dispatch one feature across several agent profiles as kanban tickets — split on the seam, spawn one child per ticket with a self-contained task, and stay reachable through comment wakes until the seam is verified.
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
  once per planning pass, not per wake — and `kanban::config::info {}` for the real column ids
  and key prefix. Add `"status": "done"` with `updated_since` to see what landed recently.
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
`labels`, `status`, and `actor`. Create into the *default* column and let the child move
itself, so its claim is visible on the activity timeline.

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

Arm the wake **before** the turn that dispatches, one binding per active ticket:

```json
engine::register_trigger {
  "trigger_type": "kanban:comment",
  "config": { "ticket_id": "KAN-8", "exclude_author": "tech-lead", "ticket": "summary" },
  "once": false,
  "label": "KAN-8-wake",
  "lifecycle": { "max_fires": 20, "expires_in_ms": 3600000 }
}
```

- `exclude_author` must equal the id you pass as `author` on your own comments, character for
  character. One typo and your own reply re-enters your handler.
- Leaving `exclude_author` off wakes you on every comment, your own included — only safe when
  your handler never comments back on that ticket.
- Bound the life. A bare `once: false` with no `lifecycle` is the binding that outlives its
  ticket and wakes a session with nothing to do.
- Label every binding `<KEY>-<purpose>` so the reap is mechanical, not a judgement call.
- Pass `"ticket": "summary"` on every binding you arm: a wake then costs a board card, not the
  whole thread, and you re-read the ticket before acting anyway.
- Pair it with a `cron` check-in reading
  `kanban::comment::list { ticket_id, exclude_author, since }`. A wake the kanban worker never
  emits reports **nothing at all** — silence looks exactly like "no news". The cron sweep goes
  through the board file and cannot fall silent with the worker's emit path.
- `kanban:comment` fires per ticket, so N tickets need N bindings.
  `harness::triggers::list` audits what is actually armed; `harness::triggers::unregister`
  retires a ticket's wiring once it lands.
- **Arm a done-watch with every wake** — `kanban:change` with
  `config: { "ticket_id": "<KEY>", "events": ["ticket.moved"], "ticket": "summary" }`, labelled
  `<KEY>-teardown`.
  On each fire read the ticket with `kanban::ticket::get`; when `status` is `done`, unregister
  every `<KEY>-*` binding on that ticket, the done-watch last so it stays alive while you reap.
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

```json
harness::metrics { "root_session_id": "<your session id>" }
```

`complete` is true only once every session in your durable tree reached a terminal turn.
That is the "all my children are done" signal — not a guess, not a timeout.

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

- [ ] Every ticket names one owner, one deliverable, its dependency by key, and criteria with
      literal `Verify:` targets.
- [ ] The schema-owning ticket is `done` before any consumer ticket is dispatched.
- [ ] Each child's task names the ticket key, the claim/report/hand-off calls, the arm-the-pair
      duty, and the stop condition — with no reference to anything only you can see.
- [ ] The spawn carries `options.functions.allow` with the ids the child actually needs, named
      explicitly.
- [ ] A `kanban:comment` binding with `exclude_author` set to your own id is armed per active
      ticket, bounded by `lifecycle`, labelled `<KEY>-wake`, with a `cron` backstop.
- [ ] Every wake has its `<KEY>-teardown` done-watch, and every binding is unregistered once
      its ticket is `done`.
- [ ] `harness::metrics { root_session_id }` reports `complete: true` before you claim the
      fan-out finished.
- [ ] The seam was exercised end to end — caller and callee in one run — and the evidence is
      in a ticket comment, not in your summary.
