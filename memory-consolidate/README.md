# memory-consolidate

Scheduled hygiene sibling of the [`memory`](../memory) worker. Finds near-duplicate memories in every bank and merges them: the duplicate retires with a `superseded_by` pointer (never a delete), the survivor absorbs the observation as corroboration. Strictly supersede-only through memory's public functions, pinned memories untouchable, every change visible as a `memory::item-changed` event. Install, stop, or remove it without touching stored memory.

## Install

```
iii worker add memory-consolidate
```

Requires the `memory` worker. Scheduling reuses the engine's cron trigger infrastructure: an hourly heartbeat binds `memory-consolidate::on-tick`, and the tick runs a pass only when `interval_hours` (default 24) have elapsed since the last one — the last completed pass persists in the state worker. **Catch-up-on-boot**: a pass missed while this worker was down runs shortly after boot instead of waiting for the next heartbeat; a slim backstop loop also keeps the schedule alive on rigs with no cron trigger owner.

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

## The LLM tier (optional)

`llm_assist_enabled` (off by default) adds one `router::complete` judge call per bank after the deterministic pass:

- **Reorder groups** — word-order matches the deterministic pass surfaces report-only ("The deploy runs on Fridays" vs "On Fridays the deploy runs") are merged only when the judge confirms the meaning is identical; role swaps ("Alice manages Bob") stay untouched.
- **Rule promotion** — memories re-observed `promote_corroboration_threshold`+ times are offered as candidates; ones the judge classifies as standing instructions land as one-line entries in the bank's auto-managed `learned` rule, fingerprint-deduped, append-only. Hand-authored rules are never touched, and the judge can only act on candidates it was offered — it cannot invent targets.

Fail-soft: no router or a malformed reply just skips the tier (named in the report's errors); the deterministic pass has already completed.

## What counts as a duplicate

v1 is deterministic and deliberately conservative: two live memories merge only when their normalized text matches (case, punctuation, and whitespace insensitive) or their token sets are equal (word-order shuffles). One differing word is NOT a duplicate — "always publishes" and "never publishes" must never merge. Survivor choice is stable: pinned first, then highest corroboration, then the oldest record (it carries the original provenance). Semantic near-duplicate merging is a later, LLM-assisted tier.

## Boundaries

- Never touches memory's files; every mutation goes through `memory::supersede` and `memory::save` — the same append-only, last-wins contract as everything else.
- Pinned memories are never superseded, even inside a duplicate group.
- Not extraction (the memory worker captures), not recall, not rule management. One job: keep banks clean.
