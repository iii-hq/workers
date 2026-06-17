# HarnessBench — Design

**Status:** Approved (design) · **Date:** 2026-06-17 · **Ticket:** [MOT-3534](https://linear.app/motia/issue/MOT-3534/harnessbench)
**Scope of this spec:** MVP — the core `harnessbench` worker + an internal comparison view in `console`.

## 1. Problem & goal

We want to compare how multiple model/config variants perform on the **same task** when run
through the iii `harness`. For each variant a user should be able to read the full session
conversation and inspect the trace, and compare quantitative performance:

- total time spent for the work (wall time),
- token consumption (input/output/cache/reasoning + cost),
- context size at the end of the work,
- trace shape: total spans, average span time.

The ticket frames three eventual surfaces — a public showcase at `harnessbench.iii.dev`, an
internal eval tool, and a CI/regression benchmark. All three are built on **one shared core**.
This spec delivers that core plus the **internal comparison view** (the first surface). The
standalone showcase site and the CI/regression surface are explicitly deferred (see §10).

### Key insight: read + orchestrate, not instrument

Every metric in the ticket is derivable from data the platform **already persists** — so
HarnessBench adds **no new instrumentation**. It orchestrates runs and reads results:

- Every persisted assistant message carries `usage { input, output, cache_read, cache_write,
  reasoning, cost_usd }` plus `model` / `provider` / `timestamp`
  (`session-manager/src/types.rs`, mirrored in `harness/src/types.rs`). `cost_usd` is filled by
  `llm-router` from catalog pricing.
- The agent loop already exposes a hold-open run (`harness::run`) and a fire-and-forget send
  (`harness::send`) that both accept `{ message, model, provider, options }`
  (`harness/src/functions/send.rs`, `run.rs`).
- The harness emits `harness::turn-completed` with the full `TurnRecord` when a turn goes
  terminal (`harness/src/events.rs`).
- OTel spans are queryable by `session_id` via the same trace API the console `Traces` page uses.

## 2. Decisions (locked during brainstorming)

| # | Decision | Choice |
|---|----------|--------|
| 1 | Purpose | All three surfaces eventually; shared core; build one first. |
| 2 | First surface | Internal comparison view (core worker + console page). |
| 3 | Task shape | Single prompt, performance-only. No correctness grading. |
| 4 | Architecture | **C** — new `harnessbench` worker owns orchestration + metrics + functions (no UI); console renders the view. |
| 5 | Comparison axis | A leg is a full **config variant** — vary model/provider/thinking-level **and** any `SendOptions` (system_prompt, max_turns, output, functions, metadata). |
| 6 | Metrics | **Two-tier**: Tier-1 from the transcript (always), Tier-2 from traces (best-effort enrichment). |

## 3. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      console (web page)                       │
│   /harnessbench — runs list + side-by-side comparison view    │
│   calls harnessbench::* and reuses session/trace components   │
└───────────────┬───────────────────────────────────────────────┘
                │ iii function calls (WS)
┌───────────────▼───────────────────────────────────────────────┐
│                    harnessbench  (new Rust worker)             │
│   create-run · status · get · list   (public)                  │
│   on-turn-completed (trigger) · aggregate-leg · sweep (internal)│
│   durable BenchmarkRun records in state                        │
└───────┬───────────────────────┬───────────────────────────────┘
        │ harness::send          │ session::messages / session::get
        │ (one per leg)          │ (read transcripts → Tier-1 metrics)
┌───────▼────────┐      ┌────────▼─────────┐   ┌──────────────────┐
│    harness     │─emits│  session-manager │   │  trace/otel read │
│ turn loop      │ turn-│  (usage persisted│   │  (Tier-2 enrich:  │
│ (existing)     │ compl│   per assistant) │   │  spans / timing)  │
└────────────────┘ eted └──────────────────┘   └──────────────────┘
```

`harnessbench` is a standard binary worker per [`docs/sops/binary-worker.md`](../../docs/sops/binary-worker.md):
typed schemas (never a `Value` handler), kebab-case function ids, golden-catalog schema test,
Path B reactive config via the `configuration` worker. It adds no instrumentation; it orchestrates
and reads.

## 4. Data model — durable `BenchmarkRun`

Stored in worker state, keyed by `run_id`. Results are an **immutable snapshot**: each leg's
metrics are computed once, when that leg goes terminal, and frozen into the record. `session_id`
is retained so the view can always deep-link into the live transcript and trace.

```jsonc
BenchmarkRun {
  id,                              // run_id
  created_at,                      // ms epoch
  status,                          // "running" | "complete" | "partial" | "failed"
  prompt,                          // the single user message (held fixed across all legs)
  defaults,                        // base config legs inherit (model + SendOptions fields)
  legs: [
    {
      label,                       // names the variant, e.g. "opus-maxturns-40";
                                   // defaults to a deterministic summary of overrides vs defaults
      config: {                    // resolved = defaults merged with this leg's overrides
        model, provider?,
        system_prompt?, max_turns?, thinking_level?, output?, functions?, metadata?
      },
      session_id, turn_id,
      status,                      // mirrors harness TurnStatus
      metrics: {                   // null until the leg is terminal
        // ── Tier-1: from session::messages (always available) ──
        wall_time_ms,              // last assistant.timestamp − first user.timestamp
        turn_count,                // # assistant messages
        function_calls,            // # FunctionCall content blocks
        tokens: { input, output, cache_read, cache_write, reasoning, total },
        cost_usd,                  // Σ usage.cost_usd
        final_context_tokens,      // last assistant message usage.input (≈ final prompt size)
        // ── Tier-2: from traces (best-effort; null if unavailable) ──
        span_count,
        avg_span_ms,
        critical_path_ms
      },
      error?                       // set if the leg failed/aborted (status terminal, no metrics)
    }
  ]
}
```

Run status: `running` while any leg is non-terminal; `complete` when all legs succeeded;
`partial` when all legs are terminal but ≥1 errored; `failed` when no leg produced metrics.

## 5. Worker surface (functions)

Naming follows `harnessbench::<verb>` (kebab-case), typed request/response schemas, golden catalog.

### Public

- **`harnessbench::create-run`** `{ prompt, defaults?, legs: [LegSpec] }` → `{ run_id }`
  Seeds the `BenchmarkRun`, then for each leg resolves `config = merge(defaults, leg)` and fires
  `harness::send { message: prompt, model, provider, options: <SendOptions from config> }`
  (async, non-blocking — does **not** hold the call open). Records each returned
  `session_id` / `turn_id` on its leg. Returns immediately with `run_id`.
  - `LegSpec = { label?, model?, provider?, system_prompt?, max_turns?, thinking_level?, output?, functions?, metadata? }`
  - At least one of `defaults.model` / `leg.model` must resolve per leg (validated; fail fast).
- **`harnessbench::status`** `{ run_id }` → the record with current per-leg `status` (live).
- **`harnessbench::get`** `{ run_id }` → full record including computed metrics.
- **`harnessbench::list`** `{ limit?, cursor? }` → recent runs (summary rows), newest first.

### Internal

- **Trigger on `harness::turn-completed`**: filters the event for a `session_id` tracked by some
  open run; on a match marks that leg terminal, invokes leg aggregation, and re-evaluates run
  status. Idempotent (a leg already terminal is a no-op).
- **`harnessbench::aggregate-leg`** (or inline in the trigger): reads `session::messages` for the
  leg's session → computes Tier-1 metrics; attempts the trace read → fills Tier-2 (or leaves
  null). Writes the frozen `metrics` onto the leg.
- **`harnessbench::sweep`**: periodic/fallback re-check of `harness::status` for legs that never
  emitted a `turn-completed` (engine restart / dropped event), so runs cannot hang forever.

### Calling out

- Fan-out fire: `harness::send` (one per leg).
- Transcript read: `session::messages` (paginated, hard cap 500/page) + `session::get` for meta.
- Trace read: the same trace/observability read the console `Traces` page uses, filtered by
  `session_id`. (Exact function id confirmed during planning; Tier-2 is best-effort regardless.)

## 6. Metrics sourcing — two tiers

**Tier-1 — transcript only.** Read the leg's assistant messages from `session::messages` and fold:

- `wall_time_ms` = last assistant `timestamp` − first user `timestamp`.
- `turn_count` = count of assistant messages.
- `function_calls` = count of `FunctionCall` content blocks across the transcript.
- `tokens.*` = Σ of each `usage` field over assistant messages; `total` = input + output.
- `cost_usd` = Σ `usage.cost_usd`.
- `final_context_tokens` = `usage.input` of the **last** assistant message (the prompt size the
  model saw on its final turn ≈ end-of-work context size).

This covers the entire ticket metric list except raw span data, and has **no dependency on trace
retention**. Aggregation is a **pure function** `transcript → Tier1Metrics` — the highest-value
unit to test.

**Tier-2 — traces (enrichment, best-effort).** Query spans for the leg's `session_id`:

- `span_count` = total spans.
- `avg_span_ms` = mean span duration.
- `critical_path_ms` = longest dependent span chain (or root span duration as a v1 approximation).

If traces are absent or expired, the leg keeps full Tier-1 metrics and Tier-2 fields are `null`.
The view renders Tier-2 columns as "—" when null rather than failing.

## 7. Orchestration flow (durable, event-driven)

1. `create-run` validates, writes the `running` record, fires N `harness::send` calls, stores each
   `session_id`/`turn_id`, returns `run_id`. The call returns in well under a second regardless of
   how long the legs take.
2. Each leg runs independently inside the harness. On terminal status the harness emits
   `harness::turn-completed { session_id, TurnRecord }`.
3. The `harnessbench` trigger matches the `session_id` to an open run/leg, marks the leg terminal,
   aggregates its metrics, and re-evaluates run status.
4. `harnessbench::sweep` is a safety net: for any leg still non-terminal past a threshold it polls
   `harness::status` and reconciles, so a dropped event or engine restart cannot strand a run.

This mirrors the repo's existing durable fan-out conventions (reactive completion via emitted
events, never a long-held synchronous call).

## 8. Console comparison view (`/harnessbench`)

A new page in the existing `console` worker. No new transcript/trace rendering — drill-down
deep-links into console's existing session and `Traces` views.

- **Runs list**: rows of recent runs (`harnessbench::list`) — prompt snippet, leg/label chips,
  status, created time.
- **Run detail** (`harnessbench::get`): one **column per leg**, each with:
  - header: `label`, model/provider, and the config diff vs `defaults`;
  - a compact metric table (Tier-1 then Tier-2; nulls render as "—");
  - inline bar comparisons across legs for the headline dimensions (wall-time, total tokens,
    cost) so the best leg per dimension is visible at a glance;
  - "open transcript" / "open trace" links into console's existing views for that `session_id`.
- Live updates: while `status === running`, poll `harnessbench::status` (or subscribe if a stream
  is readily available) so legs fill in as they finish.
- Uses console's iii-Schematic design system (instrument-panel aesthetic; density over decoration).

## 9. Testing

- **Worker**
  - Golden schema catalog test (mandatory per SOP) for all `harnessbench::*` request/response types.
  - **Unit**: Tier-1 aggregation as a pure function over transcript fixtures (multi-turn, tool
    calls, missing-usage edge cases, single-message). Tier-2 aggregation over span fixtures incl.
    the empty/missing-traces case.
  - **Integration/BDD**: a two-leg run with stubbed `harness::send` and `session::messages`,
    asserting fan-out, event-driven completion, frozen metrics, and `complete`/`partial` status.
    `@engine`-tagged scenarios soft-skip when no local engine is reachable.
- **Console**: unit tests for the metric-table + cross-leg diff rendering with fixture records,
  including null Tier-2 fields and a still-`running` partial record.

## 10. Out of scope (YAGNI for this MVP)

Each is a clean later layer on the same worker functions:

- Correctness grading / LLM-judge / pass-fail.
- Multi-turn scripted scenarios.
- Standalone `harnessbench.iii.dev` site (showcase surface #2) — built later on the same functions.
- CI / regression baselines + trend tracking + regression alerts (surface #3).
- Public-run cost/abuse guards (only needed once runs are publicly triggerable).

## 11. Open items to confirm during planning

- Exact trace/observability read function id + its filter-by-`session_id` shape (Tier-2 only;
  does not block Tier-1 or the MVP).
- Whether `harness::turn-completed` payload is sufficient to key legs directly, or the trigger
  must also read `session::get` for correlation.
- State key layout + retention policy for `BenchmarkRun` records (and an index for
  `harnessbench::list`).
- `console` routing/registration for the new `/harnessbench` page and any shared-client wiring.
