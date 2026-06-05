# context-compaction

Out-of-band session-history compactor (v2). Subscribes to the dedicated
`agent::turn_end` stream (mirrored by the event producer) and summarises older
turns when the session approaches the model's usable context limit — one wake
per turn instead of one per `agent::events` frame. Also exposes a sync
pre-turn path that the turn-orchestrator calls to compact before a turn that
would overflow.

## Purpose

Keeps sessions alive indefinitely by summarising older turns into a compact
Compaction entry. The next turn reads only the summary plus a configurable
tail of recent turns. Two independent entry points share the same
summarise-and-append core:

- **Async** (`on_agent_event`) — fires after each `TurnEnd`; cheap if not
  overflowing; does not block the turn in progress.
- **Sync** (`compact_now`) — called by the turn-orchestrator pre-flight when a
  turn would overflow; blocks until compaction completes, then reinjects the
  user's message so the turn can proceed on a clean context.

A third entry point (`prune_tool_outputs`) strips verbose tool outputs without
summarisation — a cheaper first-pass before summarising.

This worker is optional. Without it, sessions keep their full transcript.

## Registered functions

### `context-compaction::on_agent_event`

Internal stream subscriber on `agent::turn_end` — fires once per turn (kept
under the historical `on_agent_event` name).

**Payload** (camelCase or snake_case envelope):
```
{ groupId | group_id: string, event: { data: EventObj } | data: EventObj }
```

Short-circuits unless the event is a `TurnEnd` carrying a `usage` object whose
running token count (`input + output + cache_read + cache_write`) meets or
exceeds `usable(model)`. When it does: acquires the compaction lease, runs
`prune`, runs `summarizeAndAppend`, releases the lease.

**Response:** `null`

---

### `context-compaction::compact_now`

Sync pre-turn compaction called by the turn-orchestrator.

**Payload:**
```ts
{
  session_id: string;
  projected_tokens: number;           // informational; not consumed in v2
  last_user_message_id: string;       // entry_id of the user message to replay
  model: {
    id: string;
    providerID: string;
    limit: { context: number; input: number; output: number };
  };
}
```

**Response** (discriminated union):
```ts
| { status: 'ok';       tail_start_id: string | null; tokens_before: number; auto_continued: boolean }
| { status: 'busy' }    // lease held by async path; caller should retry
| { status: 'overflow'} // summariser itself failed with overflow
| { status: 'empty' }   // session is empty; nothing to compact
```

Sequence: lease-with-wait → load messages → extract replay target →
prune tool outputs → summarise → reinject user message → append synthetic
"Continue…" prompt → release lease.

---

### `context-compaction::prune_tool_outputs`

Standalone prune without summarisation.

**Payload:** `{ session_id: string }`

**Response:** `{ pruned_tokens: number; pruned_parts: number; scanned_parts: number; busy?: true }`

Acquires the `prune` lease (separate from `compaction` lease), walks
`function_result` entries from newest to oldest, nulls out outputs that fall
outside the `COMPACT_PRUNE_PROTECT` token window when freeing them would yield
≥ `COMPACT_PRUNE_MIN_FREE` tokens. Returns immediately with `busy: true` if
the prune lease is held.

---

### `context-compaction::compact_session`

User-initiated synchronous compaction. Intended for UI `/compact` actions.
Takes only a `session_id`; resolves the model and last user message internally.

**Payload:**
```ts
{ session_id: string }
```

**Response** (same discriminated union as `compact_now`):
```ts
| { status: 'ok';       tail_start_id: string | null; tokens_before: number; auto_continued: boolean }
| { status: 'busy' }    // lease held; caller may retry
| { status: 'overflow'} // summariser itself overflowed
| { status: 'empty' }   // session is empty; nothing to compact
```

**Errors thrown** (non-2xx-style, propagated to caller):
- `session_id is required` — when `session_id` is missing or empty.
- `could not resolve model for session <id>` — when no assistant message with
  provider/model fields exists in the session, or `models::get` fails.

**Sequence:**
1. Resolves the model: explicit `payload.model.limit` → session-tree scan
   → orchestrator `run_request` → conservative fallback. Avoids forcing
   `models::get` when the UI already knows the context window.
2. Calls `handleSync` with `projected_tokens: 999_999` and
   `last_user_message_id: ''` to force compaction unconditionally and
   skip the replay / auto-continue branch. `/compact` runs against a
   conversation at rest — there is no in-flight user message to re-inject.
   Without this, single-turn sessions with one user message at index 0
   collapsed to `truncatedMessages = []` and surfaced as `empty`.
3. Returns the same `CompactNowResult` shape as `compact_now`, but
   `auto_continued` is always `false` and no synthetic "Continue…" prompt
   is appended.

## Model-adaptive threshold

The overflow threshold is not a flat constant. `usable(model)` computes the
effective limit per-call:

```
usable = max(0, model.input_limit − COMPACT_RESERVED_TOKENS)
```

If `model.input_limit` is zero, it falls back to
`model.context_window − model.output_tokens`.

A session with a 200 k-token model reserves 20 k by default and triggers at
180 k. A 32 k model triggers at 12 k with the same defaults.

## Summarisation template

Summaries follow a fixed Markdown template with eight sections:

```
## Goal
## Constraints
## Progress
## Key Decisions
## Tool Calls Made
## Next Steps
## Critical Context
## Relevant Files
```

The system prompt requires the summariser to keep identifiers verbatim and
fill every section. When a prior compaction exists, a second anchored prompt
instructs the summariser to update the previous summary rather than start from
scratch, so the summary converges rather than growing without bound.

## Tool-output prune path

`prune_tool_outputs` is separate from summarisation. It:

1. Loads `session-tree::messages`.
2. Walks `function_result` entries from newest to oldest, skipping the two
   most-recent user turns.
3. Accumulates token estimates. Any output whose cumulative total exceeds
   `COMPACT_PRUNE_PROTECT` goes into the prune queue.
4. If the queue would free fewer than `COMPACT_PRUNE_MIN_FREE` tokens, it
   skips entirely (no-op).
5. Calls `session-tree::update_parts` to null out each pruned output (batched, one load).

Tools listed in `COMPACT_PRUNE_PROTECTED_TOOLS` are never pruned.

## Replay + auto-continue (`compact_now` only)

Only the turn-orchestrator overflow path uses replay. `compact_session`
runs against a conversation at rest and passes `last_user_message_id: ''`
so this branch is skipped — `auto_continued` is always `false` on that path.

When `compact_now` runs:

1. The entry matching `last_user_message_id` is extracted from the message
   list before it is passed to the summariser (so it is not summarised away).
2. That user message already sits on the active path as the compaction node's
   parent, so it needs no reinjection — `context-view.ts` reconstructs the
   window as `[summary, ...tail]`, which already ends with it.
3. A synthetic user prompt ("Continue if you have next steps, or stop and
   ask for clarification.") is appended via `session-tree::append_synthetic`
   as a child of the compaction so the model picks up where it left off.
4. `CompactNowResult.auto_continued` is `true` when a replay target existed.

## Backward compatibility

Pre-v2 deployments may have free-form summaries (not Markdown-templated).
`session-tree::compactions` returns all existing Compaction entries. The last
entry's `summary` field is used as the `previousSummary` anchor regardless of
its format, so old summaries are updated into the structured template on the
next compaction cycle.

## Configuration

All knobs are env-driven; no `config.yaml` fields are read.

| Env var | Default | Purpose |
|---|---|---|
| `COMPACT_RESERVED_TOKENS` | `20000` | Tokens reserved for model output and overhead; subtracted from `model.input_limit` to derive the overflow threshold. |
| `COMPACT_TAIL_TURNS` | `2` | Number of complete user+assistant turn pairs kept verbatim after the summary. |
| `COMPACT_PRESERVE_RECENT_TOKENS` | _(adaptive)_ | Override the tail budget in tokens. When unset, defaults to 25% of `usable(model)`, clamped to [2 000, 8 000]. |
| `COMPACT_PRUNE_PROTECT` | `40000` | Tokens of tool output to preserve from the tail before pruning. |
| `COMPACT_PRUNE_MIN_FREE` | `20000` | Minimum tokens the prune pass must free; skips if below this. |
| `COMPACT_TOOL_OUTPUT_MAX_CHARS` | `2000` | Per-output character cap applied before sending to the summariser. |
| `COMPACT_BUSY_TIMEOUT_MS` | `30000` | Max ms `compact_now` / `compact_session` waits for the compaction lease before returning `{ status: 'busy' }`. Sized to cover a typical summariser stream (10–30s) so user-initiated `/compact` doesn't race the async TurnEnd path. |
| `COMPACT_PRUNE_PROTECTED_TOOLS` | _(empty)_ | Comma-separated function IDs whose outputs are never pruned. |

The summariser provider and model are always inherited from the session's
own selection. Routing goes through `turn-orchestrator/provider-router`,
so adding a provider there automatically covers `/compact`.

## State scopes

Compaction-related keys use dedicated scopes (key = `session_id`):

| Scope | Purpose |
|---|---|
| `compaction_lease` | `{ nonce, ts }` — held for up to `LEASE_TTL_SECS = 300 s`. |
| `prune_lease` | Same nonce-and-readback pattern, separate scope so the prune path does not block async compaction. |
| `last_compaction_at` | Wall-clock ms of the most recent successful compaction. Stamped by `stampLastCompaction`. |

Compaction appends a `Compaction` entry to `session-tree` (and optionally replay + synthetic continue). The turn FSM reconstructs the compacted provider window at read time via [context-view.ts](harness/src/turn-orchestrator/state-runtime/context-view.ts); there is no separate flat `messages` scope.

## Observability

Three OTel spans are emitted per invocation (no-op when OTel is not
initialized):

| Span name | Attributes |
|---|---|
| `compaction.async` | `session_id`, `tokens_before`, `used_prior_summary` |
| `compaction.sync` | `session_id`, `tokens_before`, `replayed`, `auto_continued`, `lease_wait_ms` |
| `compaction.prune` | `session_id`, `pruned_tokens`, `pruned_parts`, `scanned_parts` |

`lease_wait_ms` measures the time spent waiting for the compaction lease in the
sync path. All spans inherit `iii.session.id` from baggage when set by the
outer `instrumentHandler` wrapper.

## Dependencies

`session-tree` endpoints used:

| Endpoint | Purpose |
|---|---|
| `session-tree::messages` | Load active path with entry IDs. |
| `session-tree::compact` | Append a Compaction entry (summary + `tail_start_id` + `tokens_before`). |
| `session-tree::compactions` | Load existing Compaction entries for prior-summary anchor. |
| `session-tree::append_synthetic` | Append the "Continue…" prompt after sync compaction. |
| `session-tree::update_parts` | Null out pruned tool outputs in-place (batched). |
| `models::get` | Resolve `context_window` / `max_output_tokens` for model-adaptive threshold. |

Worker manifest deps (`iii.worker.yaml`):
`session ^0.2.0`, `provider-anthropic ^0.2.0`, `provider-openai ^0.2.0`.

## Source layout

| File | Purpose |
|---|---|
| `src/context-compaction/main.ts` | Binary entry point (`iii-context-compaction`). |
| `src/context-compaction/register.ts` | Registers the three functions + stream subscriber. |
| `src/context-compaction/config.ts` | Reads all `COMPACT_*` env vars. |
| `src/context-compaction/handler-async.ts` | Async TurnEnd path: envelope decode, overflow check, lease, prune, summarise. |
| `src/context-compaction/handler-sync.ts` | Sync pre-turn path: lease-with-wait, extract replay, prune, summarise, reinject. |
| `src/context-compaction/handler-pipeline.ts` | Shared prune → summarise pipeline used by both handlers. |
| `src/context-compaction/flat-state.ts` | `buildSummaryMessage` helper for the compacted provider window. |
| `src/context-compaction/model-resolver.ts` | Shared model-resolution helpers: `fetchModelLimit` (catalog lookup) and `resolveModelFromSession` (session-scan + catalog lookup). |
| `src/context-compaction/prune.ts` | Tool-output pruning (`prune`). |
| `src/context-compaction/summarize.ts` | `summarizeAndAppend`: load → select tail → summarise → append Compaction entry. |
| `src/context-compaction/overflow.ts` | `usable`, `isOverflow`, `preserveRecentBudget` — model-adaptive math. |
| `src/context-compaction/selection.ts` | `selectWithEntryIds`, `completedCompactions` — tail selection with entry ID tracking. |
| `src/context-compaction/template.ts` | `SUMMARY_TEMPLATE` + `buildPrompt` — structured prompt construction. |
| `src/context-compaction/replay.ts` | `extractReplayTarget` — locates the last user message so it is excluded from the summary on the sync path. |
| `src/context-compaction/lease.ts` | `acquireLease`, `acquireLeaseWithWait`, `releaseLease`, `stampLastCompaction`. |
| `src/context-compaction/stream-collect.ts` | Drives `provider::<name>::stream` via in-process channel and collects the final message. |
| `src/context-compaction/strip-media.ts` | Strips images and truncates tool outputs before sending to the summariser. |
