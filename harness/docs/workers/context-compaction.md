# context-compaction

A thin compatibility wrapper exposing the user-initiated **`/compact`** action
(`context-compaction::compact_session`) on top of the standalone Rust
**`context-manager`** worker. All real context logic — token counting,
function-result pruning, and history summarisation — now lives in
`context-manager` (`context::*`); see
[context-manager/architecture/integration.md](../../../context-manager/architecture/integration.md).

This worker exists only to preserve the console's existing `/compact` RPC
contract. It owns no compaction policy and binds no triggers.

> Migration note: the previous version was an out-of-band compactor with four
> functions (`on_turn_end`, `compact_now`, `prune_tool_outputs`,
> `compact_session`) plus a turn-orchestrator pre-flight (`preflight.ts`) and a
> read-time provider-window reconstruction (`context-view.ts`). All of that is
> retired. The harness now compacts **inline on the hot path** by calling
> `context::assemble` once per turn (see
> [turn-orchestrator.md](turn-orchestrator.md)); deciding *when* to compact
> off the hot path is a future sibling, not this worker
> (tech-specs/2026-06-agentic/harness.md § Out of scope).

## Core invariant: the transcript is never modified

`/compact` summarises the conversation **context**, not the stored
conversation. No message entry is rewritten or deleted. The only write is an
additive `kind:"custom"` bookkeeping entry (`custom_type: "compaction"`), which
is invisible to `session::messages` (unless `include_custom`), never counted in
`message_count`, and never matches a role filter. It exists so the next turn can
anchor on the summary (`options.previous_summary`) and so the console can render
its compaction marker.

## Registered function

### `context-compaction::compact_session`

User-initiated synchronous compaction. Invoked by the console `/compact` slash
command.

**Payload:**
```ts
{
  session_id: string;                 // required
  model?: { id: string; providerID?: string;
            limit?: { context: number; input?: number; output?: number } };
}
```

**Response** (the console `CompactResult` wire shape — unchanged):
```ts
| { status: 'ok'; tail_start_id: string | null; tokens_before: number;
    auto_continued: boolean; summary_text: string }
| { status: 'busy' }      // a compaction lease is held; retry later
| { status: 'empty' }     // nothing to compact
| { status: 'overflow'; message: string }  // summariser unavailable / failed
| { status: 'error'; message: string }     // model missing, or an unexpected failure
```

`auto_continued` is always `false` — `/compact` runs against a conversation at
rest, so there is no in-flight user message to replay.

**Sequence:**
1. `loadAssembleWindow(session_id)` reads the active path, finds the latest
   compaction bookkeeping entry, and returns the post-compaction message window
   (from `tail_start_entry_id` onward) plus its summary as `previousSummary`.
2. `context::compact({ messages: window, model, options: { lease_key: session_id,
   previous_summary } })` summarises the head, keeping a recent tail verbatim.
   The summary is **returned**, not persisted — `context-manager` is
   storage-agnostic.
3. On `ok`, `persistCompactionRoundTrip` appends the compaction bookkeeping
   entry (`{ summary, tokens_before, tail_start_entry_id }`), mapping the
   worker's `tail_start_index` onto the entry id at that position in the window.
4. The `context::compact` union is mapped onto the response above.

The shared helpers live in
[runtime/compaction.ts](../../src/runtime/compaction.ts); the wrapper is
[context-compaction/register.ts](../../src/context-compaction/register.ts).

## Configuration

This worker reads no configuration of its own. Token-budget knobs (reserve,
tail turns, prune thresholds, lease TTL, summariser timeout) live in the
`context-manager` worker's `config.yaml`; per-call options (`lease_key`,
`previous_summary`) are passed by the wrapper.

## Dependencies

Worker manifest deps (`iii.worker.yaml`): `session-manager ^0.1.0`,
`context-manager ^0.1.0`.

session-manager endpoints used (via [runtime/session.ts](../../src/runtime/session.ts)):

| Endpoint | Purpose |
|---|---|
| `session::messages` | Load the active path (paginated; `include_custom: true` to find the latest compaction entry). |
| `session::append` | Append the additive compaction bookkeeping entry (`custom_type: "compaction"`, data `{ summary, tokens_before, tail_start_entry_id, details, timestamp }`). |

context-manager function used: `context::compact`.

## Source layout

| File | Purpose |
|---|---|
| `src/context-compaction/main.ts` | Binary entry point (`iii-context-compaction`). |
| `src/context-compaction/register.ts` | Registers the single `compact_session` wrapper. |
| `src/runtime/compaction.ts` | Shared round-trip helpers: `loadAssembleWindow`, `assembleContext`, `persistCompactionRoundTrip`, `compactWindow`, `modelInputFromConsole`. |
