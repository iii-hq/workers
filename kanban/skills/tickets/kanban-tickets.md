---
name: kanban-tickets
description: >-
  Work tickets on the kanban board from an agent session: find and claim a
  ticket, comment, and register a kanban:comment trigger as a feedback loop
  without waking on your own comments. Finish by moving the ticket to in
  review or done — deleting a ticket is a human action.
type: how-to
---

# Working kanban tickets

The `kanban` worker owns a file-backed board. Every ticket mutation is a
registered function, so you work a ticket with ordinary function calls and
learn about new comments through a trigger instead of polling.

The board file is `<data_path>/board.json`, resolved from the `kanban`
configuration entry; `kanban::config::info` prints the absolute path, the
column list, the priorities and the key prefix. Tickets are soft-deleted when
a **human** removes one from the ticket screen: the row stays in that file
with `deleted_at` set, so a person can undo it.

## Address a ticket by uuid or by key

Every ticket carries an internal `id` (uuid) and a human `key` (`KAN-7`).
Both work everywhere a ticket is addressed and the key is case-insensitive,
so `kanban::ticket::get { "id": "kan-7" }` and `{ "id": "<uuid>" }` are the
same call. Prefer the key in prose and in trigger configs; it is the form a
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
`high`, `urgent` by default).

**Attribute yourself.** On a write `actor` defaults to the literal `user`,
and on a comment so does `author`. Pass your own agent-profile id in both:
the activity timeline then reads honestly, and — the part that matters — the
comment filters below key on exactly that string.

### Which profile id

`kanban::agent::list {}` returns the assignable agent profiles
(`{ id, name, logo, color, model, description }`) resolved through
iii-directory. A ticket stores the profile **id** (for example
`senior-software-engineer`), never the display name.

## Feedback loop: wake on comments

Register a binding on the `kanban:comment` trigger type; it fires once per
comment created on one ticket.

| config field | meaning |
| --- | --- |
| `ticket_id` | **Required** — uuid or human key. A binding without it never fires. |
| `author` | Fire only for comments by exactly this author. |
| `exclude_author` | Fire for every author **except** this one. |
| `root_only` | Ignore replies; fire only for top-level comments. |
| `ticket` | `full` (default) delivers the whole ticket, comments and activity inline; `summary` delivers only the board-card fields. Agents that re-read the ticket on a wake should pass `summary`. |

The payload carries the ticket plus the new comment — the whole ticket by
default, or its summary when the binding says `"ticket": "summary"` (the
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

Author your comments with your profile id, and exclude that same id on the
binding:

```json
engine::register_trigger {
  "trigger_type": "kanban:comment",
  "config": {
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

The two strings must match exactly. The binding then fires for every comment
whose `author` is **not** that id — a human's reply, another agent's note —
and stays silent on your own posts, so answering the ticket cannot re-enter
your own handler.

Two details of that registration:

- A **wake** (no `function_id`) defaults to firing **once**. `once: false`
makes it standing; give it a `lifecycle` so a forgotten binding cannot run
forever. Omit `function_id` to be woken in your session, or name a function
to have the event call it instead — that second shape runs token-free but
its result is discarded and it cannot reach you.
- Teardown is `harness::triggers::unregister`; `harness::triggers::list`
audits what is armed.

### Avoid a wake that can never fire

A registration response, and a row in `engine::registered-triggers::list`, mean the
engine took the binding — **not** that the `kanban` worker will ever emit to it.
The worker delivers only to bindings in its own in-process subscriber map, so a
binding it is missing never fires and reports nothing at all. Build the loop so
that cannot leave you parked:

- **Never let the wake be your only way back.** Pair it with a check-in you own: a
`cron` binding that reads the same filtered list. It goes through the board file
rather than the worker's emit path, so it cannot fall silent with it.

```json
engine::register_trigger {
  "trigger_type": "cron",
  "config": { "expression": "0 * * * *" },
  "once": false,
  "label": "KAN-7-check-in"
}
```

On each fire, read what you missed and move the watermark forward:

```json
kanban::comment::list {
  "ticket_id": "KAN-7",
  "exclude_author": "senior-software-engineer",
  "since": "2026-09-10T13:04:29.656Z"
}
```

- **Arm a binding for a hand-off, not for the ages.** Bound its life so it cannot
outlive its usefulness: `{ "max_fires": 20, "expires_in_ms": 3600000 }` stops it
after a fixed window and wakes you with an expiry notice instead of leaving you
parked. Re-arm it at your next hand-off if the work is still open; a bare
`once: false` with no bound is the shape that fails silently.

- **The worker's copy of your binding dies with the worker process** while the
engine's row survives, so re-arm a long-running loop when you pick the ticket
back up — or let a `cron` check-in carry it so nothing depends on that copy.

- If the trigger type itself looks missing, that is the namespace, not the type:
`engine::triggers::info { "id": "kanban:comment" }` alone reports it that way
because these types live in the **worker's** namespace. Read the namespace off
`engine::triggers::list` and pass it along.

### Tighter and looser variants

- **Only the human.** `"author": "user"` fires only for comments the console
  UI writes, and is the strictest loop guard available.
- **Whole ticket.** Drop `exclude_author` and you are woken by every comment
  on that ticket, your own included — do that only when your handler never
  comments back on the same ticket.
- **Top-level only.** Add `"root_only": true` to ignore thread replies.

`kanban::comment::list` accepts the **same** filters, which is how you catch
up after a wake without re-reading everything:

```json
kanban::comment::list {
  "ticket_id": "KAN-7",
  "exclude_author": "senior-software-engineer",
  "since": "2026-09-10T13:04:29.656Z"
}
```

### Do not reach for kanban:change when you need author filtering

`kanban:change` is the board-wide trigger: config `{ ticket_id?, events?, ticket? }`,
where `events` is any of `ticket.created`, `ticket.updated`, `ticket.moved`,
`ticket.deleted`, `ticket.restored`, `comment.created`, and `ticket` takes the
same `full` / `summary` choice. It carries the ticket and is the right
primitive for keeping a board view in sync or for a done-watch.

It filters by **ticket and event name only** — there is no `author` and no
`exclude_author`. A binding of
`{ "ticket_id": "KAN-7", "events": ["comment.created"] }` fires for your own
comments too, which is exactly the loop the `kanban:comment` filters exist to
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
- Pass `actor` on writes and `author` on comments — your agent-profile id.
- Read column ids and priorities from the board instead of assuming them.
- Finish by moving the ticket: `in_review` when it needs a check, `done` when
  it is finished and verified. **Never delete a ticket** — only a human does.
- For a feedback loop, register `kanban:comment` with
  `exclude_author: "<your id>"`, `"ticket": "summary"` and `once: false`, and
  never let the handler comment back on the same ticket unconditionally.
- Read the board once per planning pass, narrowed (`statuses`, `compact`),
  never on a wake; catch up with `updated_since`.
- Catch up with `kanban::comment::list` using the same filters.
- Never make a wake your only way back: pair it with a `cron` check-in
  reading `kanban::comment::list`, and bound the binding's life (`max_fires` +
  `expires_in_ms`) so a binding the worker never took cannot park you.
