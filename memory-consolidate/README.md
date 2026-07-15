# memory-consolidate

Scheduled hygiene sibling of the [`memory`](../memory) worker. Finds near-duplicate memories in every bank and merges them: the duplicate retires with a `superseded_by` pointer (never a delete), the survivor absorbs the observation as corroboration. Strictly supersede-only through memory's public functions, pinned memories untouchable, every change visible as a `memory::item-changed` event. Install, stop, or remove it without touching stored memory.

## Install

```
iii worker add memory-consolidate
```

Requires the `memory` worker. Runs one pass every `interval_hours` (default 24) with **catch-up-on-boot** semantics: the last completed pass is persisted in the state worker, so a pass missed while this worker was down runs shortly after boot instead of waiting a full interval.

## Quickstart

Plan without writing, then look at what a pass would do:

```bash
iii trigger memory-consolidate::run dry_run=true
iii trigger memory-consolidate::status
```

Apply for one bank only:

```bash
iii trigger memory-consolidate::run bank=blog
```

The report names every group: the surviving memory, the retired duplicates, and anything skipped because it is pinned. The retired records stay on disk and remain queryable with `include_superseded: true`.

## Configuration

All fields hot-reload through the `configuration` worker: `enabled`, `interval_hours`, `dry_run` (scheduled passes plan-only), `banks` (allowlist, empty = all), `max_supersedes_per_run` (safety cap; remainder waits for the next pass).

## What counts as a duplicate

v1 is deterministic and deliberately conservative: two live memories merge only when their normalized text matches (case, punctuation, and whitespace insensitive) or their token sets are equal (word-order shuffles). One differing word is NOT a duplicate — "always publishes" and "never publishes" must never merge. Survivor choice is stable: pinned first, then highest corroboration, then the oldest record (it carries the original provenance). Semantic near-duplicate merging is a later, LLM-assisted tier.

## Boundaries

- Never touches memory's files; every mutation goes through `memory::supersede` and `memory::save` — the same append-only, last-wins contract as everything else.
- Pinned memories are never superseded, even inside a duplicate group.
- Not extraction (the memory worker captures), not recall, not rule management. One job: keep banks clean.
