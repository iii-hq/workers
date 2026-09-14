---
title: acceptance-review
description: Run the final review of a ticket in in_review against its acceptance criteria — gather real evidence per criterion, and send the ticket back to in_progress the moment any criterion is unmet, partial, or newly caveated.
type: how-to
---

# Final review against acceptance criteria

You are the gate, not a reader. A ticket leaves `in_review` for `done` **only** when you have evidence for every acceptance criterion. Anything else goes back to `in_progress` with the specific gap named.

## When this runs

A ticket is reviewable when it sits in `in_review` and the last mover was not you. Read the board; do not wait to be told:

```json
kanban::ticket::list { "status": "in_review" }
kanban::ticket::get { "id": "KAN-7" }        // description, comments, activity inline
kanban::activity::list { "ticket_id": "KAN-7", "types": ["ticket.moved", "comment.created"] }
```

Copy the acceptance criteria out of the description **before** reading the comments, so the engineer's summary cannot reframe what you are checking. Then read the comments.

## Three verdicts per criterion

| verdict | meaning |
| --- | --- |
| **met** | you personally observed the `Verify:` result, or its equivalent |
| **not met** | it does not hold, or holds only partially |
| **cannot verify** | no `Verify:` target exists, the thing is not reachable, or the only support is a claim |

`cannot verify` is **not met**. It is the most common false pass in a review.

## Evidence that counts

Each criterion's `Verify:` line names the check — run it.

- A URL in a browser: `browser::sessions::start` when the user should watch it, `browser::screenshot-url` when they should not.
- An HTTP endpoint or a JSON API: `web::fetch`; read `status` and the body, not just `ok`.
- A file, a config, a migration: read it with the coder/`shell` toolchain and quote the line.
- "The tests pass": run them.

A comment saying "done, works" is a claim. An activity row saying the lane moved is a claim. Neither is evidence.

## The verdict, as two calls

**Reject** — any criterion not met or unverifiable. Always in this order, comment first so the reason is on the ticket before the move event:

```json
kanban::comment::create {
  "ticket_id": "KAN-7",
  "body": "**Review: back to in progress.**\n\n- **AC 2 — not met.** Expected the empty state to read `No filters yet`; observed `undefined`. Verify: /tickets?filters=0\n- **AC 4 — cannot verify.** No `Verify:` target and no test covers it.\n\nRe-open this when AC 2 renders and AC 4 has a check attached.",
  "author": "product-manager"
}
```

```json
kanban::ticket::move { "id": "KAN-7", "status": "in_progress", "actor": "product-manager" }
```

Name the criterion number, what you expected, what you observed, and what would make it pass — one bullet per failed criterion. "Needs more work" is a rejection that costs a round trip.

**Accept** — every criterion met:

```json
kanban::comment::create {
  "ticket_id": "KAN-7",
  "body": "**Review: accepted.** AC 1–4 verified against the stated checks.",
  "author": "product-manager"
}
```

```json
kanban::ticket::move { "id": "KAN-7", "status": "done", "actor": "product-manager" }
```

A criterion that passed with a caveat is not passed — a caveat is a not-met. That is the whole point of the gate.

Never delete a ticket, and never move a ticket you did not review.

## Changing the criteria is a planning act, not a review act

If the criteria were ambiguous, wrong, or the work legitimately revealed new scope, **do not silently rewrite them to match what was built, and do not fail the engineer for building to a moving target.** Comment that the criteria changed, and take it back to the user as a planning decision — then edit the description with `kanban::ticket::update` and re-review. A criterion rewritten after the fact makes every future acceptance claim worthless.

## Keeping the loop armed

### Per-ticket wake

```json
engine::register_trigger {
  "trigger_type": "kanban:change",
  "config": { "ticket_id": "KAN-7", "events": ["ticket.moved"], "ticket": "summary" },
  "once": false,
  "label": "KAN-7-review",
  "lifecycle": { "max_fires": 20, "expires_in_ms": 3600000 }
}
```

**Guard against your own move, and reap on the landing.** `kanban:change` filters by ticket and event name only — there is no actor filter, so your own moves fire this binding. The handler's first act is `kanban::ticket::get`:

- `status` is `done` → **unregister this ticket's bindings and stop**. Your own accept move fires it, and a review binding left armed on a closed ticket wakes you with nothing to do. `harness::triggers::list {}` for the `subscription_id`s, then `harness::triggers::unregister { "subscription_id": "..." }` for each — this binding **last**, so it stays alive while you reap — then list again to confirm none of your `KAN-7-*` labels remain.
- `status` is `in_review` → review it.
- anything else → do nothing and stop. Without that check you re-review your own rejection forever.

Nobody can reap for you, here or anywhere: `engine::unregister_trigger` removes only bindings owned by the calling session, a foreign `session_id` is not honoured, and agent wakes do not appear in `engine::registered-triggers::list`. Label every binding `<KEY>-review` / `<KEY>-sweep` so the reap is mechanical.

### Cron sweep — the backstop that cannot fall silent

The wake only fires if the kanban worker still holds the binding in its in-process subscriber map; a binding it lost reports nothing at all. Pair it with a check-in that reads the board file directly:

```json
engine::register_trigger {
  "trigger_type": "cron",
  "config": { "expression": "0 * * * *" },
  "once": false,
  "label": "in-review-sweep"
}
```

Each fire: `kanban::ticket::list { "status": "in_review" }` and review anything whose last move actor is not you. This is also what catches the ticket that was moved while your session was offline.

Teardown when the ticket lands: `harness::triggers::list {}` to find the `subscription_id`, then `harness::triggers::unregister { "subscription_id": "..." }`. Unregister the review binding when its ticket reaches `done`, and retire the sweep itself once nothing you were watching is left in `in_review` — a sweep with an empty watch is a session woken to read an empty list. Never leave a review binding armed on a `done` ticket.

## Traps

- **You accepted on a summary.** Symptom: the ticket is `done` and the bug it described is still reproducible. Cause: the engineer's comment was read as evidence. Fix: one verdict per criterion, evidence per criterion.
- **You rejected your own rejection.** Symptom: the same review comment appears on the ticket repeatedly. Cause: the `kanban:change` binding fired on your own move back to `in_progress`. Fix: re-read the ticket and no-op unless the status is `in_review`.
- **A closed ticket keeps waking you.** Symptom: a wake for a ticket that is `done` and reviewed, with nothing to do. Cause: the handler read the status and returned without tearing the bindings down. Fix: on `done`, unregister `<KEY>-*` before you stop.
- **`done` with a caveat.** Symptom: the ticket is closed and the caveat lives only in a comment. Cause: "met, but…". Fix: a caveat is a not-met — comment and move to `in_progress`.
- **The criteria moved after the work.** Symptom: the ticket passes against a description nobody built to. Cause: the description was edited during review. Fix: criteria changes go through the user as planning, in a separate turn.
- **Reviewed stale work.** Symptom: you approve a commit that was superseded. Cause: you checked the artefact before the last activity row. Fix: read the activity timeline, then review the newest state.
- **The binding dies and the ticket sits forever.** Symptom: a ticket in `in_review` for days, no review comment. Cause: the worker's copy of the binding went away with the process. Fix: the cron sweep.

## Checklist

- [ ] `kanban::ticket::get` read; criteria copied out **before** the comments.
- [ ] Activity timeline read — you are reviewing the newest state.
- [ ] Every criterion has an explicit verdict: met / not met / cannot verify.
- [ ] Every "met" has an observation you can point at — a URL, a command, a quoted file line.
- [ ] Any not-met or cannot-verify → comment naming the criterion, expected vs observed, then move to `in_progress`.
- [ ] Any accept → comment, then move to `done`.
- [ ] `author` and `actor` are `product-manager` on every write.
- [ ] On `done`: the ticket's review binding unregistered (this binding last), and `harness::triggers::list` re-read to confirm nothing of yours is left armed for it.
- [ ] The sweep retired once nothing you were watching is still in `in_review`.
- [ ] Bindings labelled `<KEY>-review` / `<KEY>-sweep` so the reap is mechanical.
- [ ] No ticket deleted, no criterion rewritten to fit the implementation.
