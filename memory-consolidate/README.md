# memory-consolidate

Scheduled hygiene sibling of the [`memory`](https://github.com/iii-hq/workers/tree/main/memory) worker. Finds near-duplicate memories in every bank and merges them: the duplicate retires with a `superseded_by` pointer (never a delete), the survivor absorbs the observation as corroboration. Strictly supersede-only through memory's public functions, pinned memories untouchable, every change visible as a `memory::on-item-changed` event. Install, stop, or remove it without touching stored memory.

## Install

```
iii call compose::add worker=memory-consolidate
```

Requires the `memory` worker. Scheduling reuses the cron worker: `memory-consolidate::on-tick` observes `cron::on-tick` with args `{ expression: "0 0 * * * *" }`, an hourly heartbeat, and the tick runs a pass only when `interval_hours` (default 24) have elapsed since the last one — the last completed pass persists in the state worker. **Catch-up-on-boot**: a pass missed while this worker was down runs shortly after boot instead of waiting for the next heartbeat; a slim backstop loop also keeps the schedule alive on rigs without the cron worker.

## Quickstart

Plan without writing, then look at what a pass would do:

```bash
iii call memory-consolidate::run dry_run=true
iii call memory-consolidate::status
```

Apply for one bank only:

```bash
iii call memory-consolidate::run bank=blog
```

The report names every group: the surviving memory, the retired duplicates, and anything skipped because it is pinned. The retired records stay on disk and remain queryable with `include_superseded: true`.

## Functions

| Function | Input | What it does |
|---|---|---|
| `memory-consolidate::run` | `{ bank?, dry_run? }` | One consolidation pass now, over every configured bank or one bank; returns the per-bank report |
| `memory-consolidate::status` | `{}` | The schedule and the last pass: `enabled`, `interval_hours`, `last_run`, `due`, the latest per-bank results |
| `memory-consolidate::on-tick` | the `cron::on-tick` event | Internal: the schedule heartbeat |
| `memory-consolidate::on-config-change` | the `configuration::on-updated` event | Internal: hot-reloads the configuration |

The worker hosts no observable events of its own: every change it makes is a `memory::on-item-changed` call of the memory worker. It observes:

- `cron::on-tick` with args `{ expression: "0 0 * * * *" }`, target `memory-consolidate::on-tick`;
- `configuration::on-updated`, filtered on its own entry id and `configuration:updated`, target `memory-consolidate::on-config-change`;
- `http::on-request` with args `{ method: "POST", path }` for `run` and `status` (`POST /memory-consolidate/run`, `POST /memory-consolidate/status`), so both double as REST routes through the http worker.

Each observer stays `pending` until its function registers, so the cron, configuration and http workers can boot in any order.

## Configuration

All fields hot-reload through the `configuration` worker: `enabled`, `interval_hours`, `dry_run` (scheduled passes plan-only), `banks` (allowlist, empty = all), `max_supersedes_per_run` (safety cap; remainder waits for the next pass).

## The LLM tier (optional)

`llm_assist_enabled` (off by default) adds one `router::complete` judge call per bank after the deterministic pass:

- **Reorder groups** — word-order matches the deterministic pass surfaces report-only ("The deploy runs on Fridays" vs "On Fridays the deploy runs") are merged only when the judge confirms the meaning is identical; role swaps ("Alice manages Bob") stay untouched.
- **Rule promotion** — memories re-observed `promote_corroboration_threshold`+ times are offered as candidates; ones the judge classifies as standing instructions land as one-line entries in the bank's auto-managed `learned` rule, fingerprint-deduped, append-only. Hand-authored rules are never touched, and the judge can only act on candidates it was offered — it cannot invent targets.

Fail-soft: an unavailable router just skips the tier (named in the report's `notes`, so it never blocks the checkpoint), and a malformed reply is treated as "no merges, no promotions". Either way, the deterministic pass has already completed.

## The semantic tier (on by default)

`semantic_enabled` (on by default; set it to `false` to run without model calls) runs one more tier, last, on a fresh fetch. It costs one `router::complete` call per 8 new memories, capped at `semantic_max_per_run` per bank per pass. Without a router or an embed provider, it stops with a note and never blocks the pass. A `router::complete` judge looks at each memory written since the last pass, next to its nearest neighbours, and decides:

- **merge**: both state the same fact (paraphrase, another language). The duplicate retires and the survivor absorbs its observations. The survivor must keep every detail; when both are equally complete, the older one survives.
- **supersede**: a state change ("moved to", "switched to", a rescheduled date). The outdated record retires in favour of the current one, which is the one stated last.
- **nothing**: distinct facts on one topic (two allergies, two projects). This is the normal answer.

Every change goes through `memory::supersede` and carries the judge's one-sentence `reason` on memory's `memory::on-item-changed` events. How it works:

- **Candidates.** Neighbours come from the raw lexical and raw semantic `memory::recall` arms, interleaved, because a fused rank buries the near-duplicate.
- **Targets.** The judge may only pair a memory with one of its *own* candidates, through short per-batch aliases.
- **Validation.** Before anything is written, every action is checked: pinned records are never retired; direction follows `last stated`; a merge may not drop a number the survivor lacks; the reason is required; the per-run cap applies.
- **Freshness.** Each record is re-read right before the write, so a record edited since the judge read it drops its action.
- **Failure handling.**
  - Transport, auth or quota errors stop the tier for that bank with a note, and the cursor stays put.
  - An unparseable or truncated reply bisects the batch down to one memory. A memory that still fails is listed in `semantic_failed` and skipped, so it cannot wedge the tier.
- **Cursor.** Progress is a per-bank cursor in the state worker, keyed by `(updated_at, id)`, so edited or resurrected records are judged again. It advances only over batches that were fully judged and fully applied.
- **Dry runs** judge and report the plan without writing or moving the cursor.

Config: `semantic_batch` (8 memories per LLM call), `semantic_candidates` (4 per arm), `semantic_max_per_run` (48 per bank per pass). The model comes from `llm_model`. `router::complete` has no temperature control, so the provider default applies.

**Judge eval.** `tests/judge_golden.rs` runs the real prompt and validator over 18 cases from memory's recall golden set: 8 supersedes, 4 merges and 6 keeps. It reports accuracy and **harmful** actions, which must be 0. With `claude-sonnet-5-5` the result was 18/18 with 0 harmful. `UPDATE_CONSOLIDATION=1` records the decisions into memory's `tests/recall_quality/consolidation.json`, which memory's recall eval replays in its `hybrid-consolidated` mode: traps go from 2 to 0 and duplicate pollution from 23 to 0.

```bash
III_NAMESPACE=<ns> cargo test --test judge_golden -- --ignored --nocapture
```

## What counts as a duplicate

v1 is deterministic and deliberately conservative: two live memories merge only when their normalized text matches (case, punctuation, and whitespace insensitive) or their token sets are equal (word-order shuffles). One differing word is NOT a duplicate — "always publishes" and "never publishes" must never merge. Survivor choice is stable: pinned first, then highest corroboration, then the oldest record (it carries the original provenance). Semantic near-duplicate merging is a later, LLM-assisted tier.

## Boundaries

- Never touches memory's files; every mutation goes through `memory::supersede` (with `reinforce` on merges, so the survivor absorbs the duplicate's observations in the same call) — the same append-only, last-wins contract as everything else. Every change carries a `reason` on its `memory::on-item-changed` events.
- Pinned memories are never superseded, even inside a duplicate group.
- Not extraction (the memory worker captures), not recall, not rule management. One job: keep banks clean.
