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

- **Captured** (the user's own session, extraction on): new memories are
  extracted in the background after each turn, and standing instructions
  (style directives, workflow corrections) graduate into the bank's
  auto-managed `learned` rule on their own. When the user asks you to remember
  something, acknowledge it — no save call is needed.
- **Not captured** (a sub-agent, extraction off): nothing is saved
  automatically. If `memory::save` is available to you, save what a later
  session would need with it.
- **Scripted** (an automation or e2e run): nothing is saved automatically, and
  the header does not ask you to save. A scripted run does not write into the
  user's memory.

**Rules** are markdown documents injected whole into the system prompt on every
turn (`memory::rule::*`, on disk under the bank's `rules/` folder). **Memories**
are recalled records, ranked against the current question
(`memory::save/list/recall`). Durable identity-grade guidance belongs in a
rule; point-in-time information belongs in memories.

## When to Use

- The header asks you to save — if `memory::save` is available to you, save
  what a later session would need:
  `memory::save { "text": "...", "entities": [...] }`, one self-contained
  sentence each. It records your session and turn as its source; re-saving
  reinforces, never duplicates.
- The user asks to keep something for good — in a captured session
  acknowledge it (extraction keeps an explicit ask) and, to pin it,
  `memory::pin` it once `memory::recall` shows it. Where the header asks you
  to save, `memory::save { "text": "...", "pinned": true }`; add
  `"entities": ["user"]` when it is a fact about the user: a save from a turn
  is not tagged `user` on its own.
- The user asks what you remember about a topic — `memory::recall { "query": "..." }`
  ranks the bank's memories for it. A turn is given fewer: only the hits with
  evidence, one line per fact and the newer of a stated change;
  `memory::preview` shows exactly what a turn on that topic would be given.
  A turn recalls on the user's message (or the earlier one it continues);
  a notification turn recalls nothing, so call `memory::recall` when one needs it.
- The user asks to fix or forget something — find it via recall/list, then
  `memory::update` (corrected text) or `memory::delete` (tombstone), and
  `memory::pin` to protect the corrected version.
- The user wants a style or convention enforced everywhere — in a captured
  session acknowledge it: a standing instruction graduates into `learned`.
  Hand-written rules are the user's to edit on the console's memory page; rule
  writes are not granted to agents by default.
- The user asks you never to remember something — `memory::delete` what is
  already stored (find it via `memory::recall` or `memory::list`), and tell
  them that keeping it out for good takes a rule they add on the memory page.
- Separate contexts deserve separate banks (`blog` vs `coding`): a session
  selects its bank via session metadata `memory_bank` (`session::set-meta`)
  or the console composer's bank picker. The user creates banks on the
  console's memory page: bank writes are not granted to agents by default.
  Calls without `bank` act on your session's bank (the one named in your
  `# Memory` header; a sub-agent shares its parent's), so leave it out unless
  the user names another bank.
- Memory seems absent — `memory::doctor {}` runs a real save→recall roundtrip
  and names what is degraded: a sibling (router, session-manager, state),
  embeddings, or extraction passes that keep failing. Its `extraction` reports
  calls, tokens and cost since boot.

## Boundaries

- Not a transcript store: session history lives in session-manager; memory
  keeps distilled records that point back at their source session.
- Not per-turn context compression: that is context-manager's job. This worker
  is the durable cross-session sibling.
- Deletes are tombstones and bank deletion moves to `.trash/` — nothing is
  destroyed; `include_superseded: true` on list/recall shows history.
- Extraction never edits a memory in place: a restatement reinforces the
  known memory, and a correction supersedes it (a recoverable tombstone;
  pinned memories are never touched). Wider dedup/merge/promote hygiene
  belongs to the `memory-consolidate` sibling worker.
- Extraction reads only what arrived after the session's confirmed cursor,
  at most 4 model calls per pass; the rest retries from the last confirmed
  run. A redelivered job adds no second revision (each effect is appended
  with a source receipt). A long message is read from its head and tail,
  and never blocks the turns after it. Item events are
  best-effort, at most once. Deleting a session or trashing a bank purges its
  in-flight extraction state.
- Injection is bounded (`max_rule_chars`, `recall_budget_tokens`) — an
  over-budget rule is truncated visibly, never silently dropped.
- Recalled memories are dated by when their fact was last stated; trust a
  newer memory over an older one that contradicts it. Dated events are stored
  with absolute dates.

## Functions

- `memory::save` — explicit save; content-fingerprinted, repeat saves reinforce;
  from a turn it records that session and turn.
- `memory::get / list / update / delete / pin` — memory CRUD; delete tombstones,
  update bumps a revision, pinned records are untouchable by automatic paths.
- `memory::recall` — rank a bank's memories against a query (the injection
  hook's exact scorer); response names the retrieval mode that ran; `tag`
  narrows to one label.
- `memory::preview` — the FULL injection dry-run for a hypothetical chat
  message: the system-prompt rules section, the memories a turn would be
  handed (past the relevance cut and token budget), and the appended message verbatim;
  naming a directory's bank (`<name>-<6 hex>`) previews it with
  `default_bank` read under it, as its turns get it.
- `memory::tags` — a bank's distinct tags with counts. Add `tags` on save
  when the user wants memories organized within a bank.
- `memory::bank::list` — named memory scopes; creating or deleting one
  (`memory::bank::create / delete`, delete trashes) is a human-owned surface,
  denied to agents by default.
- `memory::rule::list` — the bank's always-injected markdown rules; writing
  one (`memory::rule::set`) is a human-owned surface, denied to agents by
  default.
- `memory::supersede` — retire one memory in favor of another (tombstone +
  pointer); the consolidation seam, agent-denied by default. `reinforce`
  makes it a merge; `reason` rides the `memory::on-item-changed` events.
- `memory::doctor` — end-to-end self-test plus sibling, embeddings and
  extraction health; `extraction` reports calls, tokens and cost since boot.
- `memory::reload` — re-read every bank from disk after hand-editing files.

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
