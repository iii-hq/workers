# memory

Durable cross-session agent memory: named **banks** of always-injected markdown **rules** and
auto-extracted **memories**, with hybrid recall. Fills the slot the context-manager spec reserves —
context-manager compresses one history for one turn; this worker is the durable cross-session
sibling. Memory acts visibly, not magically: everything is a plain file you can open, a function
you can call, and an event you can watch.

## Definition

Two record kinds per bank, chosen by durability semantics:

- **Rules** (`rules/<name>.md`) — markdown documents injected whole into the system prompt on every
  turn for sessions using the bank. Guaranteed presence; use for what must always hold (writing
  style, coding conventions, answer format, project constants). Bounded by `max_rule_chars` with a
  visible truncation marker.
- **Memories** (`memories.jsonl`) — one-line records recalled on demand, ranked against the current
  question. Append-only full-record log: fsync before RAM, last-wins replay by id, updates append
  revisions, deletes append tombstones. Content-fingerprinted ids (`fp` + FNV-1a of normalized
  text) make re-observation reinforce (`corroboration += 1`) instead of duplicating.

Bank selection: turn metadata `memory_bank` → session metadata `memory_bank` → configured
`default_bank`. A session-lookup failure injects nothing — never a cross-bank fallback.

Recall fuses BM25 (unicode tokenizer, CJK bigrams) + entity match + corroboration + pinned bonus +
recency half-life, plus a semantic cosine signal against per-bank vector sidecars when
`router::embed` has an embed-capable provider. Zero LLM at query time; responses name the
retrieval mode that actually ran (`bm25-entity` vs `bm25-entity-semantic`) — degradation is
explicit.

## Standalone use

`iii worker add memory`. The default bank materializes on first use. Without `llm-router`,
extraction degrades to explicit `memory::save` calls; without the harness, the full RPC surface
still works (the hook bindings just never fire). Every public function doubles as a REST route via
the `http` worker.

## Capture pipeline

One LLM call per completed turn, off the hot path:

1. `harness::turn-completed` enqueues one durable extraction job (`engine::queue::enqueue`,
   receipt-id deduped per turn; inline spawn fallback when no queue worker is present).
2. The job walks `session::messages` incrementally — a per-session cursor in the `state` worker
   (scope `memory_cursor`) means each pass reads only messages newer than the last pass. The cursor
   advances only after every durable write lands; commit failures propagate so the queue retries
   (fingerprints keep retries idempotent).
3. One `router::complete` call classifies each extracted item as a **memory** (durable information about
   the user/projects/world) or a **rule** (standing instruction about how the assistant should behave).
   Memories are saved ADD-only; rule-kind items append to the bank's auto-managed `learned` rule,
   fingerprint-deduped, capped per pass (`rule_learning_enabled` to disable). Hand-authored rules
   are never touched by any automatic path.

Embeddings are derived data: `vectors.jsonl` sidecars carry the memory revision and embedding
model; an edit or a model change invalidates the vector and background backfill re-embeds through
`router::embed`. Loss of a sidecar costs a re-embed, never memory integrity.

## Functions

Consumer-facing:

- `memory::save` — explicit save; fingerprinted, repeat saves reinforce.
- `memory::get` / `memory::list` / `memory::update` / `memory::delete` / `memory::pin` — memory
  CRUD; delete tombstones, update bumps a revision, pinned records are untouchable by every
  automatic path.
- `memory::recall` — rank a bank's memories against a query with the injection hook's exact
  scorer; `include_superseded` ranks history too.
- `memory::preview` — the full injection dry-run: the system-prompt memory section (rules,
  budgets, truncation markers), the memories a turn would be handed post ambient floor and token
  budget, and the appended message verbatim. Shares the hook's code, so it cannot drift.
- `memory::bank::create` / `memory::bank::list` / `memory::bank::delete` — named scopes; delete
  moves the folder to `.trash/` (recoverable).
- `memory::rule::list` / `memory::rule::set` — the always-injected markdown rules; empty content
  removes a rule (an empty set against a nonexistent bank is a no-op).
- `memory::tags` — distinct tags across a bank's live memories with counts. Tags are topical
  labels for filtering WITHIN a bank (set on save/update, suggested by extraction); `memory::list`
  and `memory::recall` accept a `tag` filter. Organization, not ranking.
- `memory::supersede` — retire one memory in favor of another: tombstone with a `superseded_by`
  pointer, never a plain delete. The consolidation seam; pinned memories cannot be superseded.
  Agent-denied by default.
- `memory::doctor` — real save→recall→trash roundtrip plus sibling reachability; names degraded
  states instead of a bare process-up health.
- `memory::reload` — drop RAM state and re-read every bank from disk (the recovery hatch after
  hand-editing files).

Internal (agent-denied, see Security):

- `memory::hook::pre-generate` — harness injection hook.
- `memory::on-turn-completed` / `memory::extract-job` — capture pipeline.
- `memory::on-session-deleted` — extraction-cursor GC.
- `memory::on-config-change` — configuration hot reload (store reopen on `data_dir` change,
  last-good on failure).

## Triggers

### Trigger types emitted

- **`memory::item-changed`** — a memory was created / updated / superseded / deleted. Payload:
  `{ event_type, bank, memory }`. Filterable by `bank` in the binding config.
- **`memory::bank-changed`** — a bank was created / trashed, or its rules changed. Payload:
  `{ event_type, bank }`.

Consoles and live views bind these instead of polling; delivery is fire-and-forget,
at-least-once, unordered.

### Triggers bound

- `harness::hook::pre-generate` (priority 100, `on_error: fail_open`) — injects the bank's rules
  into the system prompt (stable per session: keeps the provider prompt cache warm) and up to
  `recall_limit` recalled memories as ONE appended message (varying content never invalidates the
  cached system-prompt prefix). Hook annotations (`memory_bank`, `memory_recalled`, `memory_ids`,
  `memory_rules`, `memory_rules_truncated`, `memory_retrieval`) land on the entry origin — which
  bank and which memories fed a turn is product surface, not plumbing.
- `harness::turn-completed` — spawns the extraction pass.
- `session::deleted` — drops the session's extraction cursor.
- `durable:subscriber` on queue `memory-extraction` — durable extraction jobs with retries + DLQ.

A boot-order safety net polls `engine::triggers::info` and re-requests each binding until live:
in an orderly startup wave the trigger-type owners (harness, session-manager, queue) may register
after this worker.

## Storage

```text
<data_dir>/<bank>/bank.yaml         # description
<data_dir>/<bank>/rules/*.md        # always-injected markdown rules
<data_dir>/<bank>/memories.jsonl    # append-only memory log (full records)
<data_dir>/<bank>/vectors.jsonl     # derived embedding sidecar (rebuildable)
<data_dir>/.trash/<bank>-<ms>/      # trashed banks (never destroyed)
```

Crash-safety by construction: every mutation appends one fsynced line before touching RAM; the
search index is RAM-only, rebuilt at boot, so store and index cannot diverge. An unwritable
`data_dir` is boot-fatal — memory must never silently run in RAM.

## Security

Suggested defaults ship in `memory/iii-permissions.yaml`: agents get `memory::recall`,
`memory::save`, `memory::get`, `memory::list`, `memory::update`, `memory::delete`, `memory::pin`;
the hook/pipeline internals are denied, and bank/rule writes plus `memory::reload` are human-owned
surfaces (console, REST, CLI) by default — the always-injected rules shape every future turn's
system prompt, so writing them is a privileged operation.

## The memory family

Memory follows the same decomposition as `llm-router` + its provider workers: a family of narrow
siblings over one on-disk contract, not a monolith with modes.

- **`memory`** (this spec) — banks, rules, memories: storage, injection, capture, recall, trigger
  types. Owns the files and the wire surface; runs no scheduler.
- **`memory-consolidate`** — background hygiene on the engine's cron trigger infrastructure (an
  hourly heartbeat tick; the pass runs only when `interval_hours` elapsed) with catch-up-on-boot
  semantics (last completed pass persisted in the state worker; a pass missed while down runs
  shortly after boot, via a backstop loop that also covers cron-less rigs): deterministic dedup of near-duplicate memories (normalized-text or token-set equality;
  one differing word never merges), survivor = pinned > corroboration > oldest. Strictly
  supersede-only through `memory::supersede` + `memory::save`, never touches pinned records; every
  change lands as a `memory::item-changed` event. `dry_run` plans without writing;
  `max_supersedes_per_run` caps a pass. Promotion of corroborated clusters toward rules is its
  An optional LLM tier (`llm_assist_enabled`, off by default) runs after the deterministic pass:
  one `router::complete` judge per bank confirms which reorder groups really are equivalent
  (merged through the same seam) and promotes standing instructions observed
  `promote_corroboration_threshold`+ times into the bank's `learned` rule — append-only,
  fingerprint-deduped, and the judge can only act on candidates it was offered. Installable,
  stoppable, and removable without touching stored memory.

The seam is the store's append-only contract: any sibling that mutates memory goes through the
same `memory::*` functions and the same last-wins replay, so the family shares one durability
story.

Why this split and not finer: an earlier decomposition had store, recall, and hook-observer as
three workers, but all three share the hot in-RAM index — separating them puts a wire hop on the
pre-generate path or duplicates the index per process. They fused into this worker; consolidation
stays a sibling precisely because it is the one concern that never needs the hot index. The other
planned concerns became reuse instead of new workers: transcripts (session-manager), extraction
and embeddings (llm-router + providers), cursors (state), durable jobs (queue), REST (http),
configuration (configuration worker).

## Boundaries

- Not per-turn context compression (context-manager) and not a transcript store (session-manager);
  every extracted memory carries provenance pointing back at its source session.
- Consolidation (dedup/merge/promote across memories) belongs to the `memory-consolidate` sibling
  (see the family section) — this worker never grows a scheduler.
- Extraction never rewrites or deletes existing records; pinned records are untouchable by every
  automatic path including rule learning.
