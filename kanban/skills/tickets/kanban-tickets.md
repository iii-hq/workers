---
name: kanban-tickets
description: >-
  Work tickets on the kanban board from an agent session: find and claim a
  ticket, comment, and observe kanban::on-comment as a feedback loop without
  waking on your own comments. Finish by moving the ticket to in
  review or done — deleting a ticket is a human action.
type: how-to
---

# Working kanban tickets

The `kanban` worker owns a file-backed board. Every ticket mutation is a
registered function, so you work a ticket with ordinary function calls and
learn about new comments by observing `kanban::on-comment` instead of polling.

The board file is `<data_path>/board.json`, resolved from the `kanban`
configuration entry; `kanban::config::info` prints the absolute path, the
column list, the `done_status` column, the priorities and the key prefix. Tickets are soft-deleted when
a **human** removes one from the ticket screen: the row stays in that file
with `deleted_at` set, so a person can undo it.

## Address a ticket by uuid or by key

Every ticket carries an internal `id` (uuid) and a human `key` (`KAN-7`).
Both work everywhere a ticket is addressed and the key is case-insensitive,
so `kanban::ticket::get { "id": "kan-7" }` and `{ "id": "<uuid>" }` are the
same call. Prefer the key in prose and in observer args; it is the form a
human reads.

## The working loop

1. **Find work** — `kanban::board::get {}` for the whole board (every column
   with its tickets), or `kanban::ticket::list { "status": "todo" }`.
   Read the column ids from the response or from `kanban::config::info`; they
   are configurable, and the default board is `backlog`, `todo`,
   `in_progress`, `in_review`, `done`.

   Both reads return ticket **summaries** (no description, comments or
   activity), so they cost roughly a hundred tokens per ticket — and the
   `done` column grows forever. Keep them small: `statuses` picks the
   columns, `limit` caps each one, `updated_since` keeps only what changed
   after a timestamp, and `compact: true` shrinks each row to key, title,
   status, priority, assignee and comment_count (about a third of the size).
   `ticket_count` stays board-wide, so you always know how much the filters
   hid.

   ```json
   kanban::board::get { "statuses": ["todo", "in_progress", "in_review"], "compact": true }
   kanban::ticket::list { "status": "done", "updated_since": "2026-09-10T00:00:00.000Z", "compact": true }
   ```

   Read the board once per planning pass. A wake already carries its ticket,
   so reading the board on every wake only inflates your context.
2. **Claim it** — assign yourself and move it:
   `kanban::ticket::update { "id": "KAN-7", "assignee": "<your-profile-id>", "status": "in_progress", "actor": "<your-profile-id>" }`
3. **Report** — `kanban::comment::create { "ticket_id": "KAN-7", "body": "…", "author": "<your-profile-id>" }`
4. **Hand off** — move it to the column the work deserves:
   `kanban::ticket::move { "id": "KAN-7", "status": "in_review", "actor": "<your-profile-id>" }`
   when it needs a check, `"status": "done"` when it is finished and
   verified. Never delete it — see *Finishing a ticket* below.

`kanban::ticket::update` also takes `title`, `description`, `priority` and
`labels`; `priority` is one of the configured values (`low`, `medium`,
`high`, `urgent` by default). `labels` replaces the whole set: to add or
drop a label pass `add_labels` / `remove_labels` instead
(`{ "id": "KAN-7", "add_labels": ["bug"], "remove_labels": ["triage"] }`),
which edit the labels the ticket has when the write runs, so a person's or
another agent's concurrent label edit is not overwritten.

**Attribute yourself.** On a write `actor` defaults to the literal `user`,
and on a comment so does `author`. Pass your own agent-profile id in both:
the activity timeline then reads honestly, and — the part that matters — the
comment filters below key on exactly that string.

### Subtasks, projects and releases

A ticket has at most one **parent**, one **project** and one **release**.
They are independent facets; nothing is inherited from one to another.
`kanban::ticket::create` and `kanban::ticket::update` take all three:

```json
kanban::ticket::create { "title": "Persist filters in the URL", "parent": "KAN-7", "release": "Phase 2", "actor": "<your-profile-id>" }
kanban::ticket::update { "id": "KAN-9", "parent": "KAN-7", "project": "iii cloud", "actor": "<your-profile-id>" }
kanban::ticket::update { "id": "KAN-9", "release": null, "actor": "<your-profile-id>" }
```

A string sets the field, `null` clears it, an absent field leaves it alone.
`parent` takes a key or uuid; `project` and `release` take an id or a name
(`kanban::project::list {}` and `kanban::release::list {}` return both, with
counts).

- **One level.** The parent must be live and have no parent of its own; a
  ticket with live children cannot get a parent, and a ticket is never its own
  parent. A feature that needs a second level is two feature tickets.
- **A subtask is its own ticket.** It has its own status, assignee, comments
  and report. Moving it does not move the parent, and moving the parent does
  not move it.
- **Progress counts only the done column.** `kanban::ticket::get` on a parent
  returns `children` (live ones) and `progress: { done, total }`; `done` is the
  children sitting in `done_status`, so a child in `in_review` is not done.
  Deleted children are not counted, and neither are canceled ones (in
  `canceled_status`): move a ticket that will not be done to that column. A deleted parent leaves its children; their
  `parent` brief then has `deleted_at`.
- **Attaching writes on both tickets.** Setting or clearing `parent` adds an
  activity line to the child and to the parent, and the worker calls
  `kanban::on-change` for both.
- **Project and release are filed, not created.** You may set them on tickets
  and read the lists, but creating, renaming, archiving, shipping or deleting a
  project or a release, and converting a label into one, are console actions
  denied to agents. An archived project refuses new tickets; a released release
  still accepts them. Creating a ticket with a `parent` does not copy the
  parent's project or release: pass them if they apply.
- **Narrow the reads.** `kanban::ticket::list` and `kanban::board::get` take
  `parent_id`, `project_id` and `release_id` (`""` means none), so a feature's
  tickets are one small read:
  `kanban::ticket::list { "parent_id": "<uuid of KAN-7>", "compact": true }`.

### Which profile id

`kanban::agent::list {}` returns the assignable agent profiles
(`{ id, name, logo, color, model, description }`) resolved through
iii-directory. A ticket stores the profile **id** (for example
`senior-software-engineer`), never the display name.

## Feedback loop: wake on comments

Observe `kanban::on-comment`; the worker calls it once per comment created,
and an observer gets the comments its args select.

| arg | meaning |
| --- | --- |
| `ticket_id` | uuid or human key. An observer with neither `ticket_id` nor `parent_id` is rejected (`TICKET_OR_PARENT_REQUIRED`). |
| `parent_id` | Instead of `ticket_id`: a comment on that ticket **or on any of its children**. One observer then covers a feature and every subtask. |
| `author` | Only comments by exactly this author. |
| `exclude_author` | Every author **except** this one. |
| `root_only` | Ignore replies; only top-level comments. |
| `ticket` | `full` (default) delivers the whole ticket, comments and activity inline; `summary` delivers only the board-card fields. Agents that re-read the ticket on a wake should pass `summary`. |

The event carries the ticket plus the new comment — the whole ticket by
default, or its summary when the args say `"ticket": "summary"` (the
`comment` field is complete either way):

```json
{
  "event": "comment.created",
  "ticket_id": "8ec5c3fc-21bd-4142-b503-31bdb9e1ce70",
  "ticket_key": "KAN-7",
  "ticket": { "…": "the ticket with all comments and activity" },
  "comment": {
    "id": "5241fadd-cca7-43a3-966b-5e0c21acd2b0",
    "parent_id": null,
    "body": "…",
    "author": "user",
    "created_at": "2026-09-10T13:04:29.656Z"
  },
  "author": "user",
  "at": "2026-09-10T13:04:29.656Z"
}
```

### Keep your own comments from waking you

Author your comments with your profile id, and exclude that same id in the
observer's args:

```json
harness::observe {
  "function_id": "kanban::on-comment",
  "args": {
    "ticket_id": "KAN-7",
    "exclude_author": "senior-software-engineer",
    "ticket": "summary"
  },
  "once": false,
  "lifecycle": { "max_fires": 20, "expires_in_ms": 3600000 }
}
```

```json
kanban::comment::create {
  "ticket_id": "KAN-7",
  "body": "Pushed the fix; ready for review.",
  "author": "senior-software-engineer"
}
```

The two strings must match exactly. The observer then gets every comment
whose `author` is **not** that id — a human's reply, another agent's note —
and stays silent on your own posts, so answering the ticket cannot re-enter
your own handler.

Two details of that observer:

- A **wake** (no `target`) defaults to firing **once**. `once: false` makes
it standing; give it a `lifecycle` so a forgotten observer cannot run
forever. Omit `target` to be woken in your session, or name a function in
`target` to have the event call it instead — that second shape runs
token-free but its result is discarded and it cannot reach you.
- `harness::observe` answers `{ observer_id, once }`. Teardown is
`harness::unobserve { "id": "<observer_id>" }`; `harness::observers::list`
audits what is armed.

### Avoid a wake that can never fire

An `observer_id` means the engine holds the observer; args the kanban worker
refuses (an unknown arg, neither `ticket_id` nor `parent_id`) come back from
`harness::observe` as an error. While the kanban worker is not connected the
observer is pending; it becomes active when the worker registers
`kanban::on-comment`. Delivery is at most once and fire-and-forget, so a
dropped delivery or an expired observer reports nothing at all. Build the loop
so that cannot leave you parked:

- **Never let the wake be your only way back.** Pair it with a check-in you own:
a `cron::on-tick` observer that reads the same filtered list. It goes through
the board file rather than the worker's event calls, so it cannot fall silent
with them.

```json
harness::observe {
  "function_id": "cron::on-tick",
  "args": { "expression": "0 0 * * * *" },
  "once": false,
  "label": "KAN-7-check-in"
}
```

`expression` has six fields (sec min hour day month weekday, UTC):
`0 0 * * * *` fires at the top of every hour.

On each fire, read what you missed and move the watermark forward:

```json
kanban::comment::list {
  "ticket_id": "KAN-7",
  "exclude_author": "senior-software-engineer",
  "since": "2026-09-10T13:04:29.656Z"
}
```

- **Arm an observer for a hand-off, not for the ages.** Bound its life so it
cannot outlive its usefulness: `{ "max_fires": 20, "expires_in_ms": 3600000 }`
stops it after a fixed window and wakes you with an expiry notice instead of
leaving you parked. Re-arm it at your next hand-off if the work is still
open; a bare `once: false` with no bound is the shape that fails silently.

- An observer of a function that is not registered in its namespace stays
pending and never fires. If `engine::functions::info { "function_id":
"kanban::on-comment" }` does not find it, the kanban worker runs in another
namespace: read it off `engine::functions::list` and pass it as `namespace`
to `harness::observe`.

### Tighter and looser variants

- **Only the human.** `"author": "user"` keeps only comments the console
  UI writes, and is the strictest loop guard available.
- **Whole ticket.** Drop `exclude_author` and you are woken by every comment
  on that ticket, your own included — do that only when your handler never
  comments back on the same ticket.
- **Top-level only.** Add `"root_only": true` to ignore thread replies.

`kanban::comment::list` accepts the same `ticket_id` and author filters (not
`parent_id`; list each ticket of a feature), which is how you catch up after a
wake without re-reading everything:

```json
kanban::comment::list {
  "ticket_id": "KAN-7",
  "exclude_author": "senior-software-engineer",
  "since": "2026-09-10T13:04:29.656Z"
}
```

### Do not reach for kanban::on-change when you need author filtering

`kanban::on-change` is the board-wide event: args
`{ ticket_id?, parent_id?, release_id?, events?, ticket? }`, where `parent_id`
keeps the parent itself and any of its children, `release_id` keeps tickets in
that release and the change that moved one out of it, and `events` is any of
`ticket.created`, `ticket.updated`, `ticket.moved`, `ticket.deleted`,
`ticket.restored`, `comment.created`, and `ticket` takes the same `full` /
`summary` choice. It carries the ticket and is the right function for keeping
a board view in sync or for a done-watch.

It selects by **ticket and event name only** — there is no `author` and no
`exclude_author`. An observer with args
`{ "ticket_id": "KAN-7", "events": ["comment.created"] }` gets your own
comments too, which is exactly the loop the `kanban::on-comment` args exist to
prevent.

## Reading the history

- `kanban::activity::list { "ticket_id": "KAN-7" }` — field changes, lane
  moves, deletion and comment events, oldest first. Narrow it with `types`
  and `since`.
- `kanban::ticket::get { "id": "KAN-7" }` — the ticket with its `comments`
  and `activity` inline.
- Replies are one level deep: passing the id of a reply as `parent_id`
  attaches your comment to the same root thread rather than nesting deeper.

## Finishing a ticket: review or done — never delete

**An agent never deletes a ticket.** Deleting is the human's call, made from
 the ticket screen in the console. Leave `kanban::ticket::delete` and
`kanban::ticket::restore` alone: a finished ticket is *moved*, not removed.

Finish by moving it to the column the work deserves:

- **Needs review** → `kanban::ticket::move { "id": "KAN-7", "status": "in_review", "actor": "<you>" }`
  Use this whenever someone else should check the result before it counts:
  anything user-visible, anything that changes shared state or someone else's
  work, and any task whose ticket says a review is expected.
- **Does not need review** → `kanban::ticket::move { "id": "KAN-7", "status": "done", "actor": "<you>" }`
  Reserve `done` for work you actually finished and verified — a build you
  ran, a test that passed, a file you read back. Never mark `done` on the
  strength of having merely written something.

If you are unsure which applies, pick `in_review` and say why in a comment;
that is the safe direction, because a reviewer can always move it on.

After `in_review`, the reviewer either moves the ticket to `done` or sends it
back to `in_progress` with a comment saying what is missing — which is
exactly when the comment feedback loop above earns its keep. Read the column
ids from `kanban::config::info` if the board has been reconfigured.

## Checklist

- Address tickets by `KAN-n` or by uuid; both work every time.
- File subtasks with `parent` (one level); set `project` / `release` on tickets
  if they apply, never create or ship them. Progress counts only `done_status`.
- Pass `actor` on writes and `author` on comments — your agent-profile id.
- Read column ids and priorities from the board instead of assuming them.
- Finish by moving the ticket: `in_review` when it needs a check, `done` when
  it is finished and verified. **Never delete a ticket** — only a human does.
- For a feedback loop, observe `kanban::on-comment` with args `ticket_id` (or
  `parent_id` for a feature and its subtasks), `exclude_author: "<your id>"`
  and `"ticket": "summary"`, and `once: false`, and never let the handler
  comment back on the same ticket unconditionally.
- Read the board once per planning pass, narrowed (`statuses`, `compact`),
  never on a wake; catch up with `updated_since`.
- Catch up with `kanban::comment::list` using the same filters.
- Never make a wake your only way back: pair it with a `cron::on-tick`
  check-in reading `kanban::comment::list`, and bound the observer's life
  (`max_fires` + `expires_in_ms`) so an observer that stays pending cannot park
  you.
