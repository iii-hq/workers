---
name: tickets
description: >-
  How work moves through the kanban board as tickets: the shared working loop
  every profile follows, and the planning, dispatching, executing and
  reviewing halves the Product Manager, Tech Lead and engineers each own.
---

# tickets

Five documents, one loop. `kanban-tickets` is the mechanics everyone shares;
the other four are the roles' halves of the same hand-off, written so that a
decision made in one profile's session reaches the next profile through the
ticket and nothing else.

- `kanban-tickets` — address tickets by key or uuid, pass `actor` and
  `author`, wake on `kanban:comment` with `exclude_author`, read the board
  cheaply, finish by moving to `in_review` or `done`, never delete.
- `feature-planning` — the Product Manager turns a conversation into tickets
  whose descriptions carry observable acceptance criteria, and edits the
  ticket in the same turn the plan changes.
- `acceptance-review` — the gate: one verdict per criterion backed by
  something observed; any caveat sends the ticket back to `in_progress`.
- `ticket-orchestration` — the Tech Lead splits a feature on its seam,
  dispatches each ticket with `harness::spawn` and a full allow list, and
  stays reachable on comment wakes.
- `ticket-worker` — the dispatched engineer's loop: claim, arm the wake and
  its done-watch before reporting, report on the ticket, reap on `done`.

Fetch one with `directory::skills::get { "id": "kanban/tickets/<name>" }`.
