---
name: kanban
description: >-
  File-backed kanban board: create, read, move and comment on tickets by key
  or uuid, assign agent profiles, and wake on comments through the
  kanban:comment trigger instead of polling. Reach for it whenever work is
  planned, dispatched, reported or reviewed as tickets.
---

# kanban

The `kanban` worker owns one board file (`<data_path>/board.json`) holding
every ticket with its comments and activity timeline inline. Every ticket
carries an internal uuid and a human key (`KAN-7`, case-insensitive); both
work everywhere a ticket is addressed. Columns, priorities and the key prefix
are configuration (`kanban::config::info` prints the live values), and the
default board is `backlog`, `todo`, `in_progress`, `in_review`, `done`.

Tickets are never removed by an agent: a finished ticket is moved to
`in_review` or `done`, and a human soft-deletes from the console's ticket
screen (the row stays on disk with `deleted_at` set). Writes take an `actor`
and comments an `author`; pass your own agent-profile id in both, because the
comment filters below key on exactly that string. The worker also injects a
board page and a ticket page into the console.

## When to Use

- A feature is being planned, dispatched to other agent profiles, or reviewed,
  and the plan must outlive the chat session.
- You were dispatched onto a ticket and must claim it, report on it and hand
  it off where the reviewer will see it.
- You need to react to a human's or another agent's comment on one ticket
  without polling.
- The console should show a board or a ticket beside the conversation.

## Boundaries

- Not a general task queue or a durable job store; use `queue` for work that
  must be executed, and the board for work that must be planned and reviewed.
- No author filter on `kanban:change`; use `kanban:comment` with
  `exclude_author` for a feedback loop that must not wake on your own posts.
- Deleting and restoring tickets is the human's action; agents move tickets.
- Agent profiles are resolved through `iii-directory` (with the configured
  agents folder as the fallback); this worker does not create profiles.

## Functions

- `kanban::config::info` — resolved settings: board file path, columns, priorities, key prefix, agents folder.
- `kanban::board::get` — every column with its ticket summaries; `statuses`, `updated_since`, `limit` and `compact` keep the read small.
- `kanban::ticket::create` — create a ticket (title, description, status, priority, assignee, labels, actor).
- `kanban::ticket::get` — one ticket with comments and activity, by uuid or key.
- `kanban::ticket::list` — ticket summaries narrowed by column, assignee or `updated_since`; `compact` rows for catch-up reads.
- `kanban::ticket::update` — change title, description, status, priority, assignee or labels.
- `kanban::ticket::move` — move a ticket to a column and position (the board drag and drop).
- `kanban::ticket::delete` / `kanban::ticket::restore` — soft delete and undo; human actions.
- `kanban::comment::create` — comment or reply (`parent_id`); replies stay one level deep.
- `kanban::comment::list` — comments filtered by author, excluded author, root only, or since a timestamp.
- `kanban::activity::list` — the ticket's timeline, oldest first, optionally narrowed by type or time.
- `kanban::agent::list` — assignable agent profiles and their ids.

## Reactive triggers

Register a `kanban:comment` trigger when a session or function should run
every time someone comments on one ticket — the reviewer's answer after a
hand-off, a human's reply, another agent's note — without re-reading the
board.

Reach for it when:

- You reported on a ticket and moved it to `in_review`, and the rejection or
  acceptance must reach you after your turn ended.
- You dispatched a ticket to another profile and its report arrives as a
  comment.

If you only need the comment you just wrote, use the return value of
`kanban::comment::create`; register a trigger only when a *different* author
should wake you. `kanban:change` is the board-wide sibling: it fires on every
event (`ticket.created`, `ticket.updated`, `ticket.moved`, `ticket.deleted`,
`ticket.restored`, `comment.created`) with the whole ticket, filtered by
ticket and event name only, and is the right primitive for keeping a view in
sync or for a done-watch on `ticket.moved`.

### How to bind

1. Register a handler, or omit `function_id` from an agent session to be woken.
2. Register the trigger:

```json
engine::register_trigger {
  "trigger_type": "kanban:comment",
  "config": { "ticket_id": "KAN-7", "exclude_author": "<your profile id>", "ticket": "summary" },
  "once": false,
  "lifecycle": { "max_fires": 20, "expires_in_ms": 3600000 }
}
```

`ticket_id` is required and matches the uuid or the key; `author`,
`exclude_author` and `root_only` narrow further. `"ticket": "summary"` on
either trigger type delivers the board-card fields instead of the whole
ticket with its thread — the right choice for an agent that re-reads the
ticket on a wake. Mutations fire triggers; reads do not. The worker delivers only to bindings in its own in-process map,
so pair a standing wake with a `cron` check-in over `kanban::comment::list`.
For the payload shape call `get function info` on the trigger type.
