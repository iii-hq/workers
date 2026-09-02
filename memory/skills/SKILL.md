---
type: index
name: memory
tags: memory, banks, rules, recall, cross-session, pinning
description: >-
  Durable cross-session memory: named banks of always-injected markdown rules
  and auto-extracted memories with hybrid recall. Reach for it when the user
  asks you to remember, forget, or inspect what is remembered.
---

# memory

Cross-session memory is automatic here. The session's bank injects its rules
and relevant memories into your context each turn, new memories are extracted
in the background after each turn, and standing instructions (style directives,
workflow corrections) graduate into the bank's auto-managed `learned` rule on
their own. When the user asks you to remember something, acknowledge it — no
save call is required, though an explicit `memory::save` makes it durable
immediately instead of after the turn.

**Rules** are markdown documents injected whole into the system prompt on every
turn (`memory::rule::*`, on disk under the bank's `rules/` folder). **Memories**
are recalled records, ranked against the current question
(`memory::save/list/recall`). Durable identity-grade guidance belongs in a
rule; point-in-time information belongs in memories.

## When to Use

- The user says "remember X", or corrects you in a way that should stick —
  save it now: `memory::save { "text": "...", "entities": [...], "pinned": true }`.
  Pin anything they explicitly ask to keep. Re-saving reinforces, never duplicates.
- The user asks what you remember about a topic — `memory::recall { "query": "..." }`
  returns the exact ranked memories a turn on that topic would be given.
- The user asks to fix or forget something — find it via recall/list, then
  `memory::update` (corrected text) or `memory::delete` (tombstone), and
  `memory::pin` to protect the corrected version.
- The user wants a style or convention set enforced everywhere —
  `memory::rule::set { "bank": "...", "name": "style", "content": "..." }`.
- Separate contexts deserve separate banks (`blog` vs `coding`): a session
  selects its bank via session metadata `memory_bank` (`session::set-meta`)
  or the console composer's bank picker; `memory::bank::create` makes one.
- Memory seems absent — `memory::doctor {}` runs a real save→recall roundtrip
  and names which sibling (router, session-manager) is degraded.

## Boundaries

- Not a transcript store: session history lives in session-manager; memory
  keeps distilled records that point back at their source session.
- Not per-turn context compression: that is context-manager's job. This worker
  is the durable cross-session sibling.
- Deletes are tombstones and bank deletion moves to `.trash/` — nothing is
  destroyed; `include_superseded: true` on list/recall shows history.
- Extraction never edits or removes existing memories (ADD-only, fingerprint
  reinforced); dedup/merge/promote hygiene belongs to the `memory-consolidate`
  sibling worker.
- Injection is bounded (`max_rule_chars`, `recall_budget_tokens`) — an
  over-budget rule is truncated visibly, never silently dropped.

## Functions

- `memory::save` — explicit save; content-fingerprinted, repeat saves reinforce.
- `memory::get / list / update / delete / pin` — memory CRUD; delete tombstones,
  update bumps a revision, pinned records are untouchable by automatic paths.
- `memory::recall` — rank a bank's memories against a query (the injection
  hook's exact scorer); response names the retrieval mode that ran; `tag`
  narrows to one label.
- `memory::preview` — the FULL injection dry-run for a hypothetical chat
  message: the system-prompt rules section, the memories a turn would be
  handed (floor + budget applied), and the appended message verbatim.
- `memory::tags` — a bank's distinct tags with counts. Add `tags` on save
  when the user wants memories organized within a bank.
- `memory::bank::create / list / delete` — named memory scopes; delete trashes.
- `memory::rule::list / set` — always-injected markdown rules; empty content
  removes a rule.
- `memory::supersede` — retire one memory in favor of another (tombstone +
  pointer); the consolidation seam, agent-denied by default.
- `memory::doctor` — end-to-end self-test plus sibling reachability.
- `memory::reload` — re-read every bank from disk after hand-editing files.

## Reactive triggers

The worker emits two trigger types; bind to them for live views instead of
polling `memory::list` / `memory::bank::list`.

1. **Why bind** — `memory::item-changed` fires on every memory create / update /
   supersede / delete (`{ event_type, bank, memory }`); `memory::bank-changed`
   fires on bank create / trash and rule changes (`{ event_type, bank }`). A
   consumer that mutates memory already has the new record in its return
   payload — bind only when you need to observe changes made by OTHERS
   (extraction, other sessions, the console).
2. **When to bind** — a UI or index that must stay current with background
   extraction; a worker reacting to new memories in a specific bank (use the
   `bank` filter in the binding config).
3. **How to bind** — two steps: register a handler function, then register a
   trigger of the wanted type pointing at it, optionally with
   `config: { "bank": "blog" }` to filter. Delivery is fire-and-forget,
   at-least-once, unordered.
