---
name: memory
tags: memory, banks, rules, recall, cross-session, pinning
description: >-
  Durable cross-session memory: named banks of always-injected markdown rules
  and auto-extracted memories with hybrid recall. Reach for it when the user
  asks you to remember, forget, or inspect what is remembered.
---

# memory

The session's bank injects its rules and relevant memories into your context
each turn. Later sessions in the bank start with only what is recalled from it,
and the `# Memory` header says which case applies to yours:

- **Captured** (the user's own session, extraction on, on a turn the harness
  reports with no parent): new memories are extracted in the background after
  each turn, and standing instructions (style directives, workflow
  corrections) graduate into the bank's auto-managed `learned` rule on their
  own. Past `learned_max_chars` its oldest lines move back to memories tagged
  `demoted-rule`; a line still in `learned` that the user states again moves
  to the newest position. When the user asks you to remember something,
  acknowledge it — no save call is needed.
- **Not captured** (a sub-agent's turn: its session has a parent, or the
  harness reports a parent for the turn; or extraction off): nothing is saved
  automatically. If `memory::save` is available to you, save short notes of
  what a later session would need with it.
- **Scripted** (an automation or e2e run): nothing is saved automatically, and
  the header does not ask you to save. A scripted run does not write into the
  user's memory: its saves, updates and deletes are refused.

**Rules** are markdown documents injected whole into the system prompt on every
turn (`memory::rule::*`, on disk under the bank's `rules/` folder). **Memories**
are recalled records, ranked against the current question
(`memory::save/list/recall`). Durable identity-grade guidance belongs in a
rule; point-in-time information belongs in memories.

## When to Use

- The header asks you to save — if `memory::save` is available to you, save
  what a later session would need: `memory::save { "text": "..." }`, one
  self-contained sentence of 3 to 500 characters each. It is your note: it
  lands in your session's bank with your session and turn as its source, at
  most `max_memories_per_turn` new notes per turn; re-saving never duplicates
  and does not make a memory more certain: only the user restating it does.
- The user asks you to remember something (not in a scripted run) — in a
  captured session acknowledge it: extraction saves it after the turn, and no
  save call is needed. Where the header asks you to save, save it as a note.
  Pinning, another bank and the `user` entity are the user's actions on the
  console's memory page, refused from inside a turn: when the user wants it
  kept for good, tell them to pin it there.
- The user asks what you remember about a topic — `memory::recall { "query": "..." }`
  ranks the bank's memories for it. A turn is given fewer: only the hits with
  evidence, one line per fact and the newer of a stated change;
  `memory::preview` shows exactly what a turn on that topic would be given.
  A turn recalls on the user's message (or the earlier one it continues);
  a notification turn recalls nothing, so call `memory::recall` when one needs it.
- The user asks to forget something — find it via recall/list, then
  `memory::delete` it (a recoverable tombstone), with the `bank` recall
  returned for it. A pinned memory is the user's to delete on the memory page.
- The user corrects something — in a captured session acknowledge it:
  extraction supersedes the old memory. Elsewhere `memory::delete` the wrong
  memory and save the corrected note. `memory::update` revises only a note
  saved from a turn, never a pinned one. A save or an update refused with
  `memory_retired` names the live memory that replaced it: act on that one.
- The user wants a style or convention enforced everywhere — in a captured
  session acknowledge it: a standing instruction graduates into `learned`.
  Hand-written rules are the user's to edit on the console's memory page; rule
  writes are refused from inside a turn.
- The user asks you never to remember something — `memory::delete` what is
  already stored (find it via `memory::recall` or `memory::list`), and tell
  them that keeping it out for good takes a rule they add on the memory page.
- Separate contexts deserve separate banks (`blog` vs `coding`): a session
  selects its bank via session metadata `memory_bank` (`session::set-meta`)
  or the console composer's bank picker. The user creates banks on the
  console's memory page: bank writes are refused from inside a turn. Calls
  without `bank` act on your session's bank (the one named in your `# Memory`
  header; a sub-agent shares its parent's). A save or an update acts only on
  that bank, so leave `bank` out of them; a read may name another bank.
- Memory seems absent — `memory::doctor {}` runs a real save→recall roundtrip
  and names what is degraded: a sibling (router, session-manager, state),
  embeddings, or extraction passes that keep failing. Its `extraction` reports
  calls, tokens and cost since boot.

## Boundaries

- Not a transcript store: session history lives in session-manager; memory
  keeps distilled records that point back at their source session and the
  entries they came from.
- Not per-turn context compression: that is context-manager's job. This worker
  is the durable cross-session sibling.
- Deletes are tombstones and bank deletion moves to `.trash/` — nothing is
  destroyed; `include_superseded: true` on list/recall shows history.
- Extraction never edits a memory in place: a restatement reinforces the
  known memory, and a correction supersedes it (a recoverable tombstone;
  pinned memories are never touched). Wider dedup/merge/promote hygiene
  belongs to the `memory-consolidate` sibling worker. Its opt-in verify tier
  may replace a note its source transcript contradicts; the old note stays
  visible with `include_superseded: true`.
- Extraction reads only what arrived after the session's confirmed cursor,
  at most 4 runs per pass; the rest retries from the last confirmed run. A
  redelivered job adds no second revision (each effect is appended with a
  source receipt). A long user message is read whole up to 80,000 bytes
  (past that, its head and tail); any other line past 2,000 bytes keeps
  its head and tail. A long message never blocks the turns after it. Item events are
  best-effort, at most once. Deleting a session or trashing a bank purges
  its in-flight extraction state.
- Injection is bounded (`max_rule_chars`, `learned_max_chars`,
  `recall_budget_tokens`) — an over-budget rule is truncated visibly, never
  silently dropped. Its marker counts the hidden lines; read them with
  `memory::rule::list { "bank": "<bank>" }`, the bank the marker names.
- Recalled memories are dated by when their fact was last stated; trust a
  newer memory over an older one that contradicts it. Dated events are stored
  with absolute dates.

## Functions

- `memory::save` — explicit save; content-fingerprinted, a repeat from a turn
  adds no corroboration;
  from a turn it is your note in your session's bank (3 to 500 characters, at
  most `max_memories_per_turn` per turn), and a pin, another bank, the `user`
  entity and the `demoted-rule` tag are refused.
- `memory::get / list / update / delete / pin` — memory CRUD; delete tombstones,
  update bumps a revision, pinned records are untouchable by automatic paths.
  From a turn, update revises only a note saved from a turn, delete refuses a
  pinned memory, and `memory::pin` is refused.
- `memory::recall` — rank a bank's memories against a query (the injection
  hook's exact scorer); response names the retrieval mode that ran; `tag`
  narrows to one label. A query matching a superseded memory's words returns
  the live memory that replaced it, never the retired text (the raw `mode:
  lexical | semantic` arms do not). When the query was embedded,
  `vector_coverage {covered, live}` counts the bank's live memories the semantic signal can
  reach; `covered < live` means some vectors have not landed yet.
- `memory::preview` — the FULL injection dry-run for a hypothetical chat
  message: the system-prompt rules section, the memories a turn would be
  handed (past the relevance cut and token budget), and the appended message verbatim;
  naming a directory's bank (`<name>-<6 hex>`) previews it with
  `default_bank` read under it, as its turns get it.
- `memory::tags` — a bank's distinct tags with counts. Add `tags` on save
  when the user wants memories organized within a bank.
- `memory::bank::list` — named memory scopes; creating or deleting one
  (`memory::bank::create / delete`, delete trashes) is a human-owned surface,
  refused from inside a turn.
- `memory::rule::list` — the bank's always-injected markdown rules, redacted
  as injected; writing one (`memory::rule::set`) is a human-owned surface,
  refused from inside a turn. A `set` with `expected` (the content as last
  listed; `""` to create only) is refused with `rule_changed` when the rule
  changed since: list again and merge.
- `memory::supersede` — retire one memory in favor of another (tombstone +
  pointer); the consolidation seam, refused from inside a turn. `reinforce`
  makes it a merge; `reason` rides the `memory::on-item-changed` events.
- `memory::doctor` — end-to-end self-test plus sibling, embeddings and
  extraction health; `extraction` reports calls, tokens and cost since boot.
- `memory::reload` — re-read every bank from disk after hand-editing files;
  refused from inside a turn.

## Observing changes

The worker hosts two change functions; observe them for live views instead of
polling `memory::list` / `memory::bank::list`.

1. **What they carry** — `memory::on-item-changed` is called on every memory
   create / update / supersede / delete and returns
   `{ event_type, bank, memory, reason? }`; `memory::on-bank-changed` is
   called on bank create / trash and rule changes and returns
   `{ event_type, bank }`. A consumer that mutates memory already has the new
   record in its return payload — observe only when you need changes made by
   OTHERS (extraction, other sessions, the console).
2. **When to observe** — a UI or index that must stay current with background
   extraction; a worker reacting to new memories in a specific bank (filter
   on `input.bank`).
3. **How to observe** — neither function takes args; select calls with
   filters: `harness::observe { "function_id": "memory::on-item-changed",
   "filters": { "input.bank": { "$eq": "blog" } }, "once": false, "lifecycle":
   { "expires_in_ms": 3600000 } }` wakes this session on each change for an
   hour, and a `target: { "function_id": ... }` calls that function
   instead. Delivery is fire-and-forget and unordered.
