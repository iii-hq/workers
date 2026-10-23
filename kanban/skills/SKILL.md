---
name: kanban
description: >-
  File-backed kanban board: create, read, move and comment on tickets by key
  or uuid, assign agent profiles, and wake on comments through the
  kanban:comment trigger instead of polling. Group tickets as subtasks and
  file them under projects and releases. Reach for it whenever work is
  planned, dispatched, reported or reviewed as tickets.
---

# kanban

The `kanban` worker owns one board file (`<data_path>/board.json`) holding
every ticket with its comments and activity timeline inline. Every ticket
carries an internal uuid and a human key (`KAN-7`, case-insensitive); both
work everywhere a ticket is addressed. Columns, priorities, the key prefix and
the done column are configuration (`kanban::config::info` prints the live
values), and the default board is `backlog`, `todo`, `in_progress`,
`in_review`, `done`.

A ticket has at most one parent (a subtask), one project and one release,
independent of each other. A parent's `progress` counts only children in the
`done_status` column (read it from `kanban::config::info`; `in_review` is not done), over
the children not in the `canceled_status` column: move a ticket that will not be done there.

Tickets are never removed by an agent: a finished ticket is moved to
`in_review` or `done`, and a human soft-deletes from the console's ticket
screen (the row stays on disk with `deleted_at` set). Writes take an `actor`
and comments an `author`; pass your own agent-profile id in both, because the
comment filters below key on exactly that string. The worker also injects a
board, a ticket, a releases and a projects page into the console.

## When to Use

- A feature is being planned, dispatched to other agent profiles, or reviewed,
  and the plan must outlive the chat session.
- You were dispatched onto a ticket and must claim it, report on it and hand
  it off where the reviewer will see it.
- You need to react to a human's or another agent's comment on one ticket
  without polling.
- The console should show a board or a ticket beside the conversation.

Pills are small badges another worker pins on a ticket, shown on the card and
the ticket page: `pills: [{ "icon": "calendar", "text": "Fri 17:00" }]` on
`ticket::create` or `ticket::update` (`icon` is a lucide icon name; the list
replaces the set, `[]` clears it). Use them for what the board does not model
itself: a due date, an alarm, a source system.

## Boundaries

- Not a general task queue or a durable job store; use `queue` for work that
  must be executed, and the board for work that must be planned and reviewed.
- No author filter on `kanban:change`; use `kanban:comment` with
  `exclude_author` for a feedback loop that must not wake on your own posts.
- Deleting and restoring tickets is the human's action; agents move tickets.
- Creating, shipping or archiving projects and releases, and converting a
  label into one, are console actions; agents list them and file tickets under them.
  The one exception is the deepwork orchestrator, which creates its run's project.
- Agent profiles are resolved through `iii-directory` (with the configured
  agents folder as the fallback); this worker does not create profiles.

## Comment discipline

Treat comments as a decision and handoff log, not an execution trace. Comment
for a material scope/acceptance decision, a new actionable blocker or human
question, a review rejection with evidence, or a consolidated final handoff.
Do not post each tool call, dispatch, phase start/end, successful check or
internal retry. Keep technical progress in session traces, state results or
the evidence artifact; status changes already appear in activity.

Read existing comments before posting, deduplicate unchanged facts, and
consolidate same-turn facts with outcome, evidence reference and next action.
A blocker warrants another comment only when something material changes.
For delegated multi-phase work, leaves use the assigned result channel and
the ticket owner consolidates the handoff. Preserve required reports and
human decisions; fewer comments must not hide failures or open questions.

## Functions

- `kanban::config::info` — resolved settings: board file path, columns, `done_status`, `canceled_status` (null when the board has none), priorities, key prefix, agents folder.
- `kanban::board::get` — every column with its ticket summaries, plus `done_status`, `projects` and `releases` with their counts; `statuses`, `updated_since`, `limit` and `compact` keep the read small, and `parent_id`, `project_id` and `release_id` narrow it (`""` means none).
- `kanban::ticket::create` — create a ticket (title, description, status, priority, assignee, labels, pills, actor); `parent`, `project` and `release` file it as a subtask or under a project or release.
- `kanban::ticket::get` — one ticket with comments and activity, by uuid or key, plus its `parent`, its `children` and a `progress` count.
- `kanban::ticket::list` — ticket summaries narrowed by column, assignee, `updated_since`, `parent_id`, `project_id` or `release_id`; `compact` rows for catch-up reads.
- `kanban::ticket::links` — the tickets one ticket mentions and the tickets that mention it, by key in title, description or comments; summary rows, by uuid or key.
- `kanban::ticket::update` — change title, description, status, priority, assignee, labels, pills, `parent`, `project` or `release` (a string sets, `null` clears, absent leaves it); `add_labels` / `remove_labels` edit the current labels (ASCII case ignored, applied after `labels`, remove before add) so concurrent label edits do not overwrite each other. A patch that changes nothing records and fires nothing. A deleted ticket's other fields can be edited, but its status cannot be changed: restore it first.
- `kanban::ticket::move` — move a ticket to a column and position (the board drag and drop); moving to where it already is changes nothing, and a deleted ticket cannot be moved.
- `kanban::ticket::delete` / `kanban::ticket::restore` — soft delete and undo; human actions. Deleting a deleted ticket or restoring a live one changes nothing.
- Writes (`ticket::create` / `update` / `move`, `comment::create`) return the whole ticket; pass `compact: true` to get only its compact row (key, title, status, priority, assignee, comment_count) when you don't need the rest.
- `kanban::comment::create` — comment or reply (`parent_id`); replies stay one level deep.
- `kanban::comment::list` — comments filtered by author, excluded author, root only, or since a timestamp.
- `kanban::activity::list` — the ticket's timeline, oldest first, optionally narrowed by type or time.
- `kanban::agent::list` — assignable agent profiles and their ids.
- `kanban::project::list` — projects with status, totals and per-column counts; `include_archived` adds the archived ones.
- `kanban::release::list` — releases with derived status (planned, active, released), totals, counts and a per-project breakdown.
- `kanban::project::create` / `kanban::project::update` / `kanban::project::archive` / `kanban::project::restore` — manage projects; human actions.
- `kanban::release::create` / `kanban::release::update` / `kanban::release::ship` / `kanban::release::reopen` / `kanban::release::delete` — manage releases; `ship` needs `move_open_to` when tickets are not done; human actions.
- `kanban::label::convert` — turn the tickets carrying a label into a project or a release, with `dry_run` buckets; human action.

A subtask has one level: the parent must be live and have no parent, and a
ticket with live children cannot get one. Attaching or detaching writes an
activity line on both tickets. A subtask does not inherit its parent's project
or release; pass them. A project or release is addressed by id or name; an
archived project refuses new tickets, a released release still accepts them.

A user may mention a ticket in chat as `@kanban(id="<uuid>")`; the message
then arrives with a `<mentions>` block carrying the ticket's summary and the
pre-verified `kanban::ticket::get` call. To point the user at a ticket in a
reply, write `@kanban(id="<uuid>")` with the `id` a function returned — the
console renders it as a ticket pill. (`kanban::mention::search` and
`kanban::mention::get` serve the composer and that block; they are internal.)

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
sync or for a done-watch on `ticket.moved`. Besides `ticket_id` and `events`
it takes `parent_id` (fires for the parent and for any of its children) and
`release_id` (fires for tickets in that release, and for the change that moves
one out of it).

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

`ticket_id` or `parent_id` is required and matches the uuid or the key;
`parent_id` fires for comments on the parent or any of its children, so one
binding replaces one per child. `author`, `exclude_author` and `root_only`
narrow further. `"ticket": "summary"` on
either trigger type delivers the board-card fields instead of the whole
ticket with its thread — the right choice for an agent that re-reads the
ticket on a wake. Mutations fire triggers; reads do not. The worker delivers only to bindings in its own in-process map,
so pair a standing wake with a `cron` check-in over `kanban::comment::list`.
For the payload shape call `get function info` on the trigger type.
