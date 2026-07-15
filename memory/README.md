# memory

Durable cross-session agent memory. Named **banks** hold two kinds of content: **rules** (markdown documents injected whole into every turn's system prompt) and **memories** (auto-extracted records recalled on demand). Everything is a plain file you can open, a function you can call, and an event you can watch: memory that acts visibly, not magically.

## Install

```
iii worker add memory
```

The default bank `main` materializes on first use. No configuration required; without `llm-router` extraction degrades to explicit `memory::save` calls, and without the harness the worker still serves its full RPC surface.

## Quickstart

Save something, then see exactly what a turn on that topic would be given:

```bash
iii trigger memory::save text="User publishes blog posts on Tuesday mornings" pinned=true
iii trigger memory::recall query="when do I publish articles"
```

Give the bank a standing instruction — injected into the system prompt of every session using it:

```bash
iii trigger memory::rule::set name=style content="# Style\nFormal register. No em-dashes."
```

With the harness running there is nothing else to wire: chat normally and memory captures in the background. Say "never start replies with the word Certainly" in a conversation and it lands in the bank's auto-managed `learned` rule; state something durable and it appears as a memory after the turn. If memory seems absent, `iii trigger memory::doctor` runs a real save→recall roundtrip and names what is degraded.

## Configuration

All fields hot-reload through the `configuration` worker (rendered as a form in the console); `data_dir` reopens the store on the fly. Fields: `data_dir`, `default_bank`, `inject_rules`, `inject_memories`, `recall_limit`, `recall_budget_tokens`, `extraction_enabled`, `extraction_model`, `extraction_window`, `extraction_timeout_ms`, `max_memories_per_turn`, `rule_learning_enabled`, `max_rule_chars`, `decay_half_life_days`, `embeddings_enabled`, `embedding_model`.

## Why this shape

- **Files are the source of truth.** `rules/*.md` and `memories.jsonl` under the data dir. Edit them in any editor; `memory::reload` (or a restart) picks the edits up. The search index is a RAM-only cache rebuilt from the files at boot, so store and index can never diverge across restarts.
- **Crash-safe by construction.** Every mutation appends one fsynced JSONL line before touching RAM. There is no shutdown flush to get wrong. An unwritable data dir is boot-fatal: this worker never silently runs in RAM.
- **Supersede, never delete.** Updates append revisions; deletes append tombstones; trashed banks move to `.trash/`. Any state is recoverable.
- **Pinning.** A pinned memory ranks higher in recall and is untouchable by every automatic path.
- **One LLM call per turn, zero at query time.** Extraction runs in the background after a turn completes (ADD-only, content-fingerprinted so redelivery reinforces instead of duplicating). Recall is BM25 + entity match + corroboration + recency, plus a semantic signal when `router::embed` is available: sub-millisecond at this scale.
- **Rules learn from corrections.** Extraction classifies standing instructions (style directives, workflow corrections) separately from memories and appends them to the bank's auto-managed `learned` rule — correcting the agent in chat updates the system prompt for every later turn. Hand-authored rules are never touched; dedup by content fingerprint; `rule_learning_enabled` turns it off.
- **Honest health.** `memory::doctor` runs a real save→recall→trash roundtrip and reports sibling reachability. `memory::recall` names the retrieval mode it ran. Degradation is explicit, never silent.

## How it hooks into the harness

| Seam | What happens |
|---|---|
| `harness::hook::pre-generate` (fail-open, priority 100) | Injects the session bank's rules into the system prompt (stable per session: keeps the provider prompt cache warm) and up to `recall_limit` recalled memories as one appended message |
| `harness::turn-completed` | Spawns one background `router::complete` extraction pass over the last `extraction_window` user/assistant messages |

Bank selection order: turn metadata `memory_bank` → session metadata `memory_bank` (`session::set-meta`) → configured `default_bank`. A session-lookup failure injects nothing rather than falling back across banks.

## Functions

| Function | Purpose |
|---|---|
| `memory::bank::create / list / delete` | Banks as first-class named scopes ("blog" vs "coding" vs "personal"); delete moves to `.trash/` |
| `memory::save` | Explicit save ("remember this"): fingerprinted, reinforces on repeat |
| `memory::get / list / update / delete / pin` | Memory CRUD; delete tombstones; update bumps a revision in the log |
| `memory::recall` | The exact scorer the hook uses: preview what a turn would be given |
| `memory::rule::list / set` | The always-injected markdown rules; empty content removes |
| `memory::doctor` | End-to-end self-test (roundtrip + sibling reachability) |
| `memory::reload` | Reload every bank from disk after hand-editing files |

## Custom trigger types

Bind live views here instead of polling; both are filterable by `bank` in the binding config, delivery is fire-and-forget, at-least-once, unordered.

- `memory::item-changed` — a memory was created / updated / superseded / deleted. Payload: `{event_type, bank, memory}`.
- `memory::bank-changed` — a bank was created / trashed, or its rules changed. Payload: `{event_type, bank}`.

## Storage layout

```text
~/.iii/data/memory/
  main/
    bank.yaml          # description
    rules/style.md     # always-injected markdown
    memories.jsonl     # append-only memory log (full records, last-wins by id)
    vectors.jsonl      # derived embedding sidecar (rebuildable)
  .trash/              # trashed banks, timestamped
```

## Permissions

Suggested defaults ship in [`iii-permissions.yaml`](iii-permissions.yaml): agents get `memory::recall`, `memory::save`, `memory::get`, `memory::list`, `memory::update`, `memory::delete`, `memory::pin`; the hook/pipeline internals are denied, and bank/rule writes plus `memory::reload` are human-owned surfaces (console, REST, CLI) by default.

## The memory family

Like `llm-router` and its provider workers, memory is a family of narrow siblings rather than one grown monolith — this worker owns the store, per-turn injection, capture, and recall, and stays deliberately small:

| Worker | Role | Status |
|---|---|---|
| `memory` (this) | Banks, rules, memories: storage, harness injection, background capture, hybrid recall, live trigger types | shipped |
| `memory-consolidate` | Background hygiene on a schedule: dedup and merge near-duplicate memories, promote corroborated clusters, catch-up-on-boot cron semantics. Supersede-only, never touches pinned records, emits audit events | planned |

The split is the point: consolidation can be installed, stopped, or removed without touching stored memory, and this worker never grows a scheduler.

## Boundaries

Context-manager compresses one history for one turn; this worker is the durable cross-session sibling its spec reserves (see [tech-specs/2026-06-agentic/memory.md](../tech-specs/2026-06-agentic/memory.md)). Recall fuses BM25 with a semantic signal when `router::embed` has an embed-capable provider; `memory::recall.retrieval` names the mode either way. Consolidation belongs to the `memory-consolidate` sibling, not here.
