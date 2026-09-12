---
title: ticket-worker
description: Work a kanban ticket you were dispatched onto — claim it, arm your own comment wake and its done-watch before reporting, report on the ticket, and reap every binding the moment the ticket lands in done.
type: how-to
---

# Working a ticket you own

You were dispatched onto a ticket by someone who cannot talk to you: this engine has no
`harness::send` (confirm with `engine::functions::list { "prefix": "harness::" }`). Every
instruction you get arrived in your task or in a ticket comment, and every answer you give has
to land as a ticket comment. Your chat is not a channel.

## Open these first

- `kanban::config::info {}` — the real column ids (never assume `in_review` / `done`), the key
  prefix, and the board file path.
- `engine::functions::info { "function_ids": ["kanban::ticket::get", "kanban::ticket::update",
  "kanban::comment::create", "kanban::comment::list", "kanban::ticket::move",
  "engine::register_trigger", "harness::triggers::list", "harness::triggers::unregister"] }` —
  the exact schemas behind every call below.
- `directory::skills::get { "id": "kanban/tickets/kanban-tickets" }` — the deep reference: comment threads,
  the activity timeline, `kanban:change`, the soft-delete rules. Fetch it when you need any of
  those; skip it otherwise.
- `kanban::ticket::get { "id": "KAN-8" }` — read the ticket you were dispatched onto, whole,
  its `comments` and `activity` included, before you change anything. The key (`KAN-8`) and the
  uuid both work, case-insensitively.

## The loop

Your profile id is the string in your frontmatter (`backend-engineer`, `frontend-engineer`).
Use it verbatim as `actor` on every write and `author` on every comment — it is also the string
the wake filters key on, so a typo silently breaks the loop.

1. **Claim.** `kanban::ticket::update { "id": "KAN-8", "assignee": "<your id>", "status":
   "in_progress", "actor": "<your id>" }`.
2. **Arm your wake and its teardown — before you report.** See *The pair* below. Reporting
   first is the failure this step exists to prevent: the reviewer's answer arrives after your
   turn ended and wakes nobody.
3. **Work**, then verify it the way your own profile requires.
4. **Report on the ticket** — `kanban::comment::create { "ticket_id": "KAN-8", "author":
   "<your id>", "body": "..." }` — with the calls you made and what they returned. A summary
   in your chat is invisible to the reviewer.
5. **Hand off** — `kanban::ticket::move` to `in_review` when someone else must check it, and to
   `done` only when the ticket's own `Verify:` targets passed and you observed them.
6. **On a wake** (a review comment, a rejection): re-read the ticket with `kanban::ticket::get`
   first — a late or re-armed fire can deliver a comment for work that already closed. Answer
   on the ticket, then re-arm the pair if the work is still open.

## The pair: never arm a wake without its done-watch

Two bindings per ticket, registered together. The first is how you hear back; the second is
what removes the first when the ticket lands.

```json
engine::register_trigger {
  "trigger_type": "kanban:comment",
  "config": { "ticket_id": "KAN-8", "exclude_author": "backend-engineer", "ticket": "summary" },
  "once": false,
  "label": "KAN-8-wake",
  "lifecycle": { "max_fires": 20, "expires_in_ms": 3600000 }
}
```

```json
engine::register_trigger {
  "trigger_type": "kanban:change",
  "config": { "ticket_id": "KAN-8", "events": ["ticket.moved"], "ticket": "summary" },
  "once": false,
  "label": "KAN-8-teardown",
  "lifecycle": { "max_fires": 50, "expires_in_ms": 86400000 }
}
```

- `exclude_author` must equal your `author` string character for character, or your own comment
  re-enters your handler.
- `"ticket": "summary"` keeps the wake small: you re-read the ticket with `kanban::ticket::get`
  on every wake anyway, so the full thread in the payload is paid twice.
- **Label every binding `<KEY>-<purpose>`.** The reap below is mechanical only if you can
  recognise your own bindings by label.
- **Bound every binding's life.** A session that dies between arming and `done` leaves nothing
  to reap but the deadline.

### The teardown handler

`kanban:change` fires on your own moves too, so the first act is always a status read:

```json
kanban::ticket::get { "id": "KAN-8" }
```

If `status` is not `done`, stop — you were woken by a move made for another reason. If it **is**
`done`, reap:

```json
harness::triggers::list {}
```

Unregister every binding whose label starts with `KAN-8-` — the wake, the cron backstop, any
re-armed duplicate — then the teardown binding itself **last**, because it has to stay alive
while you reap:

```json
harness::triggers::unregister { "subscription_id": "sub_..." }
```

Finish by listing again and confirming none of your `KAN-8-*` labels remain. That read-back is
the proof; the unregister response alone is not.

**Why you reap your own.** `engine::unregister_trigger` removes "a trigger subscription owned
by the current harness session"; a foreign `session_id` is not honoured (a listing call carrying
someone else's session id returns your own bindings), and agent wakes do not appear in
`engine::registered-triggers::list` at all. No dispatcher, reviewer, or reaper can clean up
after you. Yours are yours to retire, and the `done` event is the moment.

## The backstop: a wake is not a guarantee

The kanban worker delivers only to bindings in its own in-process subscriber map, and that map
dies with the worker process. A binding it no longer holds **never fires and reports nothing at
all** — silence indistinguishable from "no news". So never let the wake be your only way back.

```json
engine::register_trigger {
  "trigger_type": "cron",
  "config": { "expression": "0 * * * *" },
  "once": false,
  "label": "KAN-8-check-in"
}
```

On each fire, read what you missed and move the watermark forward:

```json
kanban::comment::list {
  "ticket_id": "KAN-8",
  "exclude_author": "<your id>",
  "since": "<the last comment timestamp you saw>"
}
```

## Traps

- **The reviewer rejects and nothing happens.** Your turn had already ended and no binding was
  armed on the ticket. Cause: you reported before arming the pair. Fix: arm first, report
  second.
- **Your own comment wakes you, forever.** The `exclude_author` string is not byte-identical to
  your `author`. Fix: copy the profile id out of your frontmatter; do not retype it from memory.
- **A binding fires for a ticket that is already closed.** A late, re-armed, or duplicated fire.
  Fix: re-read the ticket and no-op unless it is still open and assigned to you.
- **A ticket is `done` and bindings are still armed.** Cause: no done-watch was armed, or the
  reap stopped after the first unregister. Fix: arm the pair together, unregister every
  `<KEY>-*` label with the teardown binding last, and read the list back.
- **You cannot tell your bindings apart while reaping.** Cause: labels like `wake` or `check-in`
  with no ticket key. Fix: `<KEY>-<purpose>`, always.
- **The ticket sits in `in_progress` and nobody answers.** Cause: your report was a chat message,
  or `author` was left at its `user` default so the reviewer's filters did not see it. Fix:
  comment on the ticket, and pass `author` every time.
- **You reached for a function that does not exist.** There is no `harness::send`; the wire is
  the ticket. Fix: `engine::functions::list` / `directory::search_functions` before naming an id.

## Checklist

- [ ] `kanban::config::info` read; column ids taken from the board, not assumed.
- [ ] Ticket read whole — description, comments, activity — before any write.
- [ ] Claimed with `assignee`, `status`, and `actor` all set to your profile id.
- [ ] `kanban:comment` wake armed with `exclude_author` = your `author`, labelled `<KEY>-wake`,
      bounded by `lifecycle`.
- [ ] `kanban:change` done-watch armed on the same ticket, labelled `<KEY>-teardown`, before
      the first report.
- [ ] A `cron` check-in reading `kanban::comment::list` with the same filters, so the loop
      cannot fall silent.
- [ ] Report posted as a ticket comment naming what you ran and what it returned.
- [ ] Ticket moved to the lane the evidence earned.
- [ ] On `done`: every `<KEY>-*` binding unregistered, teardown binding last, then
      `harness::triggers::list` re-read to confirm none remain.
