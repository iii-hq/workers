---
name: memory
description: Remember and recall durable facts across sessions using the memory worker: explicit saves, previews of what will be injected, bank management, and fixing wrong memories.
---

# memory

Cross-session memory is automatic: the session's bank's blocks and relevant facts arrive in your context each turn, and new facts are extracted after each turn. Use the functions below when the user asks you to remember, forget, or inspect memory.

## Remember something now

When the user says "remember X" (or corrects you in a way that should stick), save it immediately instead of waiting for extraction:

```
memory::save { "text": "Mike writes blog posts in a formal register, no em-dashes", "entities": ["mike", "blog"], "pinned": true }
```

Pin anything the user explicitly asks to keep. Re-saving the same text reinforces instead of duplicating.

## See what memory knows

- `memory::recall { "query": "<topic>" }`: the exact ranked facts a turn on this topic would be given.
- `memory::list { "limit": 20 }`: newest facts in the current bank.
- `memory::bank::list {}`: all banks with counts.

## Fix a wrong memory

1. Find it: `memory::recall` or `memory::list`.
2. Correct it: `memory::update { "id": "<fact id>", "text": "<corrected>" }`, or tombstone it with `memory::delete { "id": "<fact id>" }`.
3. Protect it: `memory::pin { "id": "<fact id>", "pinned": true }`.

Deletes are tombstones; nothing is destroyed. `include_superseded: true` on list/recall shows history.

## Banks

Separate contexts get separate banks (e.g. `blog`, `coding`, `personal`). The session picks its bank via session metadata `memory_bank` (`session::set-meta { "session_id": "...", "metadata": { "memory_bank": "blog" } }`). Create one with `memory::bank::create { "name": "blog" }`.

## Standing instructions (blocks)

Durable identity-grade guidance (writing style, coding preferences) belongs in a block, not a fact: blocks are injected whole every turn:

```
memory::block::set { "bank": "blog", "name": "style", "content": "# Style\nFormal register. Short paragraphs. Never: 'dive in', em-dashes." }
```

## Health

If memory seems absent, run `memory::doctor {}`: it performs a real save→recall roundtrip and reports which siblings are unreachable, naming what is degraded.
