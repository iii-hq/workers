---
title: feature-planning
description: Turn a feature conversation into kanban tickets whose descriptions stay the source of truth — interview first, write acceptance criteria as observable checks, and edit the ticket in the same turn the plan changes.
type: how-to
---

# Planning a feature into tickets

## Open these first

- The `kanban-tickets` skill owns the write mechanics — actor, keys, columns, priorities, the comment loop. If it is not in your context: `directory::skills::get { "id": "kanban/tickets/kanban-tickets" }`.
- `kanban::config::info {}` — the live column ids, priorities and key prefix. Never assume a column id; read it.
- `kanban::board::get {}` — what is already planned. A ticket you cannot see is a ticket you will duplicate.

The board is the authoritative record. The chat is a scratchpad that is thrown away when the session ends.

## The first move

`kanban::board::get {}` **before you say anything about the feature**. Then, for every ticket named, `kanban::ticket::get { "id": "KAN-7" }` — its description and comments are the context you are about to edit, and rewriting them from memory is how a decision somebody already made gets silently dropped.

## Interview before writing

Ask until you can answer all five in one sentence each. Those sentences become the ticket.

1. Who is this for, and what do they do today instead?
2. What changes for them when this ships — the observable difference?
3. What is explicitly NOT in this slice?
4. How will we know it works, in a way a person can check without reading the diff?
5. What must exist before this can start — other tickets, an API, a decision?

If the user says "just write the tickets", answer 1–4 yourself, mark each answer `Assumed:` in the description, and say the assumption out loud. Never ask a question you can answer by reading the board or the code.

## The ticket description template

Every description is this, headings verbatim, in this order:

```markdown
## Problem
<one paragraph: who is hurt today, and how>

## Outcome
<what is true after this ships, from the user's side>

## Acceptance criteria
1. <subject> <action> → <observable expected result>. Verify: <URL, command, or screen>.
2. ...

## Out of scope
- <the thing a reasonable reader would assume is included, and is not>

## Notes
- Depends on: KAN-4
- Assumed: <anything you decided without confirmation>
```

The rules that make criteria usable:

- **One criterion per verifiable statement.** If it contains "and", it is probably two.
- **Name the actor and the observation.** "An operator sees the row in In review" beats "status updates correctly".
- **Every criterion carries a `Verify:`** — a URL, a command, a screen. A criterion nobody can check is a criterion that will pass on vibes.
- **Criteria describe behaviour, not implementation.** File paths, table names and module choices belong in `Notes` as constraints, never as criteria.
- **3–7 criteria.** More than that is two tickets.

## Split by outcome, not by layer

A ticket is a slice that can ship and be checked on its own.

- One ticket per observable outcome. "Add the search API and build the filter UI" is two outcomes, therefore two tickets.
- If two tickets must land together before either is checkable, they are one ticket.
- Ordering is a `Depends on:` line in Notes, not a tenth criterion.
- Infrastructure with no observable outcome is a `Notes` line inside the ticket that needs it — not a ticket.

## The edit loop — the part that must not slip

The plan changes every few turns. Each change lands in the ticket **in the same turn it is agreed**:

```json
kanban::ticket::update {
  "id": "KAN-7",
  "description": "<the FULL new body>",
  "actor": "product-manager"
}
```

`description` **replaces** the body — read the ticket, edit it, write the whole thing back. Never a fragment.

Then report the change in prose — "criterion 3 now reads X, and I moved Y to out of scope" — and stop for confirmation. A plan the user has not seen written down is not agreed.

Creating a new one:

```json
kanban::ticket::create {
  "title": "Search filters persist across sessions",
  "description": "<the template above>",
  "status": "backlog",
  "priority": "medium",
  "labels": ["feature"],
  "actor": "product-manager"
}
```

`status` takes a configured column id (`backlog`, `todo`, `in_progress`, `in_review`, `done` by default — read them from `kanban::config::info`); `priority` is one of `low`, `medium`, `high`, `urgent`.

Assigning is a **separate decision**. Leave `assignee` null unless the user names who does the work; then use the profile id `kanban::agent::list {}` returned — `senior-software-engineer`, never the display name "Senior Software Engineer".

## Traps

- **The plan lives in chat.** Symptom: the user comes back next session and the ticket still says what it said yesterday. Cause: you batched the update "for later". Fix: write it in the turn it is agreed, mid-conversation if need be.
- **You overwrote a description an engineer had started annotating.** Cause: `update` replaced a body that had grown comments. Fix: `kanban::ticket::get` first; if work is underway (a comment, a move to `in_progress`), post the change as a comment and get an answer before rewriting the body.
- **A ticket nobody can start.** Symptom: the engineer replies with three questions. Cause: criteria written as intentions, with no `Verify:` line and no dependency list.
- **Two tickets that are really one.** Symptom: neither can be moved to `in_review` on its own.
- **Duplicate plan.** Symptom: two tickets with overlapping criteria. Cause: planning from memory instead of `kanban::board::get`.
- **You invented scope.** Symptom: the user reads the description and finds a feature they never asked for. Cause: filling a gap with a plausible product decision instead of asking. Fix: ask, or mark it `Assumed:`.

## Checklist

- [ ] `kanban::board::get {}` read this session before any planning.
- [ ] Every created or edited ticket read back with `kanban::ticket::get` and matches what you told the user.
- [ ] Every ticket has ≥3 acceptance criteria, each with an observable result and a `Verify:` line.
- [ ] `Out of scope` is non-empty on every ticket.
- [ ] Every `Depends on:` key exists on the board.
- [ ] `actor: "product-manager"` on every write; `assignee` only set to an id `kanban::agent::list` actually returned.
