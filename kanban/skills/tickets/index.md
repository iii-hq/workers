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
  cheaply, file subtasks and set a project or release, finish by moving to
  `in_review` or `done`, never delete.
- `feature-planning` — the Product Manager turns a conversation into tickets
  whose descriptions carry observable acceptance criteria, groups outcomes
  under a parent ticket, and edits the ticket in the same turn the plan changes.
- `acceptance-review` — the gate: one verdict per criterion backed by
  something observed; any caveat sends the ticket back to `in_progress`, and a
  parent is accepted only when its subtasks are done or explicitly cut.
- `ticket-orchestration` — the Tech Lead splits a feature on its seam, makes
  the feature ticket the parent of every dispatched ticket, dispatches each
  with `harness::spawn` and a full allow list, and stays reachable on one
  parent-level wake.
- `ticket-worker` — the dispatched engineer's loop: check the parent, claim,
  arm the wake and its done-watch before reporting, report on your own ticket,
  reap on `done`.

Fetch one with `directory::skills::get { "id": "kanban/tickets/<name>" }`.
