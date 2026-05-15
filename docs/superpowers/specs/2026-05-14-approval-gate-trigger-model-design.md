# Approval-gate trigger model: convert blocking approvals into deferred-result triggers

**Date:** 2026-05-14
**Status:** Design approved, ready for implementation plan
**Branch:** `spec/approval-gate-trigger-model` (off `origin/main`)
**Depends on:** PR #136 (`refactor/approval-reactive-fanout`) merging first — this spec assumes the reactive `ui::approval::*` pipeline is in place.

## Context

Today's `approval-gate` worker intercepts function calls listed in a turn's `approval_required` array, writes a pending record to state, then **blocks the agent's turn** by polling state every 250ms in `await_decision` (`approval-gate/src/lib.rs:200-238`) until a human resolves the approval. The agent literally cannot do anything else while a human thinks.

For the current consumers — `shell::fs::write`, `shell::fs::mkdir`, both write-side operations whose result the agent rarely needs — synchronous blocking is the wrong primitive. A more honest model treats the human as a **trigger source**, not a synchronous dependency: the agent submits the call, gets an immediate `pending_approval` tool result, and continues. When the human eventually resolves, the gate executes the underlying function and the outcome is stitched into the agent's next turn as a system message.

This spec describes how to make that change without altering the UI wire contract that PR #136 just stabilised.

**Outcome:** the agent's turn never stalls on a human. The gate becomes a deferred-execution worker rather than a synchronous policy filter. `await_decision` and its 250ms poll are deleted. The `ui::approval::*` event contract is unchanged.

## Scope and non-goals

**In scope**
- Replace the blocking model in `approval-gate` with a return-immediately + execute-on-resolve model.
- Add `approval::list_undelivered` and `approval::ack_delivered` RPCs for next-turn stitching.
- Extend the gate's state record with `result`, `error`, `delivered_in_turn_id` fields.
- Integrate stitching into `turn-orchestrator` so resolved approvals surface as system messages at the start of subsequent LLM turns within a session.
- Lazy timeout flipping inside any read path that surfaces pending records.
- Session-deletion cascade: pending approvals for deleted sessions become `timed_out`.

**Out of scope (deliberate)**
- Read-side approvals (where the agent needs the result to continue). Approval-gate is now write-side-only by design.
- Per-function flag for blocking vs. trigger mode. Full replacement; one mode for all gated functions.
- New iii SDK primitives (e.g. a first-class `defer` hook return). The hook protocol stays as-is; we encode `pending` inside the existing `block: true` + reason channel.
- UI changes for showing decision outcomes inline in chat. The `ui::approval::*` channels gain optional fields but the existing rendering is unchanged.
- Compare-and-set semantics on concurrent resolve. Last-write-wins continues to be acceptable; both browsers would have written the same decision in the realistic cases.
- Retention/cleanup of long-abandoned `approvals` state entries. Out of scope; a future cleanup job.

## Architecture

### Lifecycle of one approval

```
[CREATED] --(call hits gate's before_function_call hook)----------> [PENDING]
[PENDING] --(human allow)---------------------------------------> [APPROVED]
            --(gate invokes underlying function via iii.trigger)-> [EXECUTED { result }]
                                                                OR [FAILED   { error  }]
[PENDING] --(human deny)----------------------------------------> [DENIED { reason }]
[PENDING] --(now >= expires_at observed on any read)------------> [TIMED_OUT]
[EXECUTED | FAILED | DENIED | TIMED_OUT]
            --(turn-orchestrator stitches into a turn)-----------> stamps `delivered_in_turn_id`
```

`APPROVED` is an intentional intermediate state: it lets a debugger or operator distinguish "human said yes but execution hasn't started yet" from "executed cleanly" from "executed and failed." In normal operation the transition through `APPROVED` is sub-second.

`delivered_in_turn_id` is a stamp, not a state transition. Records with a non-null stamp are excluded from `list_undelivered` results but remain readable for auditing and UI hydration.

### Gate's new responsibilities

1. **Intercept (policy):** as today, match against `approval_required` and write a pending record. Different: return immediately with `{block: true, reason: "approval-gate: pending_approval", status: "pending", call_id, function_id}`. **No `await_decision`.**
2. **Execute (new):** in `approval::resolve` with `decision=allow`, invoke `iii.trigger(<original function_id>, <original arguments>)`, capture the result or error, write the record to `executed`/`failed`. With `decision=deny`, write `denied { reason }` and never invoke.
3. **Replay (new):** serve `approval::list_undelivered` and `approval::ack_delivered` so turn-orchestrator can stitch resolved approvals into the next turn.

The gate stays a single iii worker. No new processes, no new infra.

### Turn-orchestrator's new responsibility

Before each LLM turn within a run, turn-orchestrator calls `approval::list_undelivered({session_id})`, prepends one system message per entry to the turn request, and on successful LLM completion calls `approval::ack_delivered({session_id, call_ids, turn_id})`. If the LLM call fails before ack, the next turn re-surfaces the same messages (at-least-once; idempotent on the LLM side because the content is deterministic and the underlying function is not re-executed).

## Data flow

### (1) Gate hook reply

Today's two-shape contract (removed):
```json
{ "block": false }
{ "block": true, "reason": "approval-gate: <reason>" }
```

New single-shape contract on intercept:
```json
{
  "block": true,
  "reason": "approval-gate: pending_approval",
  "status": "pending",
  "call_id": "<function_call_id>",
  "function_id": "<original function id>"
}
```

The turn-orchestrator's function-call result builder (in `turn-orchestrator/src/states/functions.rs`) reads `status` and `call_id`. When `status == "pending"`, the result envelope going to the LLM is built from this shape rather than treated as a generic block.

### (2) LLM-facing tool result

```json
{
  "status": "pending_approval",
  "call_id": "<function_call_id>",
  "function_id": "shell::fs::write",
  "message": "Awaiting human approval. The result will be reported in a future turn."
}
```

Plain JSON content the LLM reads as a tool_call_result. No LLM-side protocol changes; the agent treats this as informational input and decides whether to continue the turn, end it, or ask the user a clarifying question.

### (3) `agent::events` stream

Unchanged from PR #136. `approval_requested` already fires on intercept; `approval_resolved` already fires on resolve. The latter gains optional `result` / `error` / `decision_reason` fields. Existing consumers (web reducer, harness-tui) ignore unknown fields; the fields are reserved for future UI work that surfaces outcomes inline.

### (4) State record shape

```
scope:  approvals
key:    {session_id}/{call_id}
value:  {
  function_call_id: string
  function_id:      string
  args:             object
  status:           "pending" | "approved" | "executed" | "failed" | "denied" | "timed_out"
  expires_at:       u64    (ms since epoch)
  result:           any?   (set when status == "executed")
  error:            string? (set when status == "failed")
  decision_reason:  string? (set when status == "denied" or "timed_out")
  delivered_in_turn_id: string?  (stamped by approval::ack_delivered)
}
```

### (5) Stitched system message format

One message per entry. Deterministic for `(call_id, terminal_status, result|error|decision_reason)` so re-delivery on retry is idempotent from the LLM's perspective.

```
[approval-gate] Earlier call_id <X> (function_id=<fn>, args=<args>):
  decision: <allow|deny|timeout>
  status: <executed|failed|denied|timed_out>
  result: <result JSON, omitted on deny/timeout/failed>
  error: <error string, present only on failed>
  reason: <decision_reason, present on denied/timed_out>
```

**Args formatting rule.** `<args>` is the original arguments object serialised as compact JSON (no pretty-printing) and truncated to **512 characters** with a trailing `… (truncated)` marker if longer. Rationale: 512 chars fits a typical `shell::fs::write` of a short file inline, longer payloads get summarised to preserve LLM context budget. The truncation must keep the JSON visibly truncated (don't pretend it's a complete object) so the LLM doesn't try to parse it. The full args are still queryable from state by `call_id` if a future flow needs them.

**Result formatting rule.** Same: compact JSON, truncated at 512 chars with marker. `result: <truncated>` is acceptable for the LLM's reasoning context; the agent can re-derive the actual value if it needs to (it likely doesn't, since the action is write-side).

## New APIs on approval-gate

### `approval::list_undelivered`

```
payload: { session_id: string }
returns: { entries: [
  {
    call_id: string,
    function_id: string,
    args: object,
    status: "executed" | "failed" | "denied" | "timed_out",
    result?: any,
    error?: string,
    decision_reason?: string,
  }
] }
```

Pure read. Lazily flips any `pending` records whose `expires_at` is past to `timed_out` before deciding whether to include them. Records with non-null `delivered_in_turn_id` are excluded.

### `approval::ack_delivered`

```
payload: { session_id: string, call_ids: string[], turn_id: string }
returns: { ok: true, stamped: number }
```

Stamps `delivered_in_turn_id = turn_id` on each named record for the given session. Idempotent: re-acking already-stamped records is a no-op (does not overwrite the original turn_id). Unknown call_ids are silently skipped, not an error.

### `approval::list_pending` (unchanged)

Still serves UI hydration on reconnect, per PR #136. Internally the gate now also lazily flips expired pendings here (see "lazy timeout" below).

### `approval::resolve` (semantics changed)

Signature unchanged. New behavior:
- On `decision=allow`: write `status=approved`, then invoke `iii.trigger(<function_id>, <args>)`. On success, write `executed { result }`. On error, write `failed { error }`.
- On `decision=deny`: write `denied { reason }`. Underlying function is never invoked.
- Lazy timeout flip: if the record is past `expires_at` and still `pending` at the moment of resolve, flip to `timed_out` first and return `{ ok: false, error: "timed_out" }` without honoring the late decision.

## Turn-orchestrator integration

### Where the hook lives

`turn-orchestrator/src/states/functions.rs` already drives the per-turn LLM request. Two integration points:

1. **Before constructing the LLM messages array:** call `approval::list_undelivered({session_id})`, prepend one system message per entry to the array.
2. **After a successful LLM response is materialised into the turn output:** fire-and-forget `approval::ack_delivered({session_id, call_ids, turn_id})`.

Stitching runs at every LLM turn within a run, not only at run start. A run with 5 turns where an approval resolves during turn 2's lifetime surfaces the resolution at the start of turn 3.

### When no run is active

If a session has resolved-undelivered approvals but no current run, those entries simply wait. The next time the user prompts the agent on that session, the messages surface at the start of the resulting turn. If the session is never resumed, the entries stay in state indefinitely (acceptable; revisit only if storage cost becomes a real issue).

### Backwards compatibility with running sessions

When the new code deploys, in-flight sessions may have records written by the old `await_decision` path. Those records:
- Have no `delivered_in_turn_id` — read paths treat absent as null.
- Have no `result` or `error` field.
- Have `status` in `{ pending | allow | deny }` (old enum).

Migration rules at read time:
- **Old `pending`** → flip to `timed_out { decision_reason: "migration" }` so it stitches cleanly and doesn't hang. The agent learns it didn't get an approval. Operators wanting to honor a still-relevant pending must re-trigger the call.
- **Old `allow`** → treat as `executed` with no `result` (the underlying function already ran in the old flow; the result was returned to the agent at that time). Stitched message: "approved (legacy record; original result delivered in-band when the call was made; no replay)." Most agents will have moved on by now and not need it; the entry exists for completeness.
- **Old `deny`** → treat as `denied`, carrying the existing `reason` field.

These transformations happen lazily on read, identical in mechanism to the timeout flip. No bulk migration script.

**Operational mitigation:** before the production deploy, drain pending approvals in the dev environment (resolve or let timeout); prefer rolling out during low gated-call activity to minimise the number of legacy records the migration rules have to handle.

## Error handling and edge cases

| # | Case | Handling |
|---|------|----------|
| 1 | Timeout while pending | Lazy: every read path that surfaces a `pending` record checks `now >= expires_at` and flips to `timed_out` before returning. No periodic sweeper. |
| 2 | Underlying function fails after approval | Record transitions `pending → approved → failed { error }`. Stitched message says "approved but execution failed: <error>." Agent can retry by emitting a new call (new call_id). |
| 3 | Hook reply stream gone by resolve time | Expected. Gate no longer replies to the hook on resolve; it directly invokes the function. No `write_hook_reply` after resolve. |
| 4 | Same function_id called twice while first pending | Allowed; each call has a unique call_id; two independent records, two UI rows, two resolutions. |
| 5 | Session deleted while approvals pending | On session deletion, sweep its pending approvals to `timed_out { reason: "session_deleted" }`. Skip stitching for sessions that no longer exist in `state::list scope=agent prefix=session/`. |
| 6 | `approval::resolve` on already-resolved record | Returns `{ ok: false, error: "already_resolved" }`, unchanged. |
| 7 | `approval::resolve(allow)` but function id no longer registered | Gate's `iii.trigger` returns `function_not_found`. Record transitions `pending → approved → failed { error: "function_not_found" }`. |
| 8 | Crash between `list_undelivered` (no ack) and LLM turn | Next turn re-surfaces. `list_undelivered` is a pure read; idempotent. |
| 9 | Crash between LLM turn success and `ack_delivered` | Next turn re-surfaces the same messages. Underlying function is not re-executed (already `executed`). LLM sees deterministic content; rare double-acknowledgement in agent prose is acceptable. |
| 10 | Concurrent resolves from two browsers | Gate's existing `status == "pending"` guard wins one; the loser sees `already_resolved`. Last-write-wins on the actual decision string in the rare race. Accepted; CAS out of scope. |

## Testing strategy

Same TDD/pure-helper-first style as PR #136. Tests precede implementation.

### approval-gate unit tests (Rust, pure functions where possible)

- `handle_intercept_returns_pending_envelope` — matching `approval_required` produces the `block: true + status: pending + call_id + function_id` envelope synchronously. No state-bus await.
- `handle_intercept_no_match_returns_block_false` — non-gated functions pass through unchanged.
- `handle_resolve_allow_invokes_underlying_function` — fake III records the `iii.trigger` call; record transitions `pending → approved → executed { result }`.
- `handle_resolve_allow_records_failed_on_function_error` — underlying function returns error; record reaches `failed { error }`.
- `handle_resolve_deny_does_not_invoke_function` — `iii.trigger` is never called; record is `denied { reason }`.
- `handle_resolve_on_already_resolved_returns_error` — unchanged from today.
- `handle_resolve_on_expired_pending_flips_to_timed_out` — lazy flip wins over the late decision.
- `lazy_timeout_flip_in_list_undelivered` — expired pendings surface as `timed_out`, never as `pending`.
- `lazy_timeout_flip_in_list_pending` — expired pendings are excluded from the UI hydration result.
- `function_not_found_during_execute_records_failed` — gate's `iii.trigger` returns `function_not_found`; record reaches `failed`.

### approval-gate new APIs (Rust)

- `list_undelivered_returns_resolved_with_null_delivered_in_turn_id` — happy path.
- `list_undelivered_excludes_pending` — only terminal states surface.
- `list_undelivered_is_pure_read` — calling twice returns the same set when nothing else changed.
- `ack_delivered_stamps_delivered_in_turn_id` — after ack, `list_undelivered` returns empty for those call_ids.
- `ack_delivered_is_idempotent` — re-acking same call_ids is a safe no-op; does not overwrite the original turn_id.
- `ack_delivered_skips_missing_call_ids_silently` — unknown ids are ignored, not an error.

### Turn-orchestrator stitching (Rust, pure helper)

Mirrors PR #136's pattern: extract the system-message construction as a pure function, test exhaustively, leave the async glue thin.

- `stitch_entries_one_message_per_entry` — happy path.
- `stitch_entries_executed_includes_result` — executed entries carry the `result:` line.
- `stitch_entries_failed_includes_error` — failed entries carry the `error:` line.
- `stitch_entries_denied_omits_result_and_error` — denied entries surface only `decision` + `reason`.
- `stitch_entries_timed_out_uses_timeout_reason` — `timed_out` is its own case.
- `stitch_entries_empty_input_returns_empty` — empty input produces no system messages.
- `stitch_message_is_deterministic_for_same_input` — re-delivery on retry yields byte-identical content.
- `stitch_truncates_args_over_512_chars_with_marker` — long args payload is truncated with `… (truncated)` and the JSON is visibly incomplete (not silently re-parseable).
- `stitch_truncates_result_over_512_chars_with_marker` — same rule applied to the `result:` line for executed entries.

### Integration test — full pending→resolve→next-turn cycle

In `turn-orchestrator/tests/`. Highest-value test: pins the entire contract end-to-end.

1. Stand up a real `iii::InProcessClient` (or equivalent test harness), register approval-gate, register a fake `shell::fs::write` function that records its inputs and returns `{ ok: true }`.
2. Drive one synthetic LLM turn that emits a `shell::fs::write` tool call. Assert the agent's tool_call_result is the `pending_approval` envelope. Assert the fake `shell::fs::write` was NOT invoked.
3. Call `approval::resolve({ decision: "allow" })`. Assert the fake `shell::fs::write` IS invoked exactly once with the original args. Assert the record reaches `status=executed`.
4. Drive a second synthetic LLM turn. Assert the LLM's incoming message array contains a single system message naming the call_id, status=executed, and the recorded result. Assert `ack_delivered` was called for that call_id with the new turn_id.
5. Drive a third synthetic LLM turn. Assert no stitched message is prepended (record is delivered, excluded by `list_undelivered`).

Plus two variants:

- **Deny variant.** Step 3 with `deny`; step 4 message says denied; fake function is never invoked across all turns.
- **Timeout variant.** Skip step 3, advance virtual clock past `expires_at`, drive step 4; assert lazy flip surfaces `timed_out` and stitches accordingly.

Cost estimate: ~150 lines of test scaffolding on top of `approval-gate/tests/integration.rs`'s existing harness. The fake LLM is a scripted turn sequence — no real provider call.

### Web (TypeScript)

The UI surface is unchanged. One regression pin only:

- `useStatus` test: `ui::approval::resolved` payload now optionally carries `result` / `error` / `decision_reason` fields. The reducer ignores them today; pin that with an explicit test so we notice the day we add inline outcome rendering.

### Manual smoke checklist

- Prompt the agent to write a file. Verify the agent's tool_call_result in the messages stream is `pending_approval` (not a blocked turn).
- Click allow in the UI. Verify the file is actually created on disk after the click.
- Send another prompt. Verify the new turn's request includes a system message acknowledging the prior approval and its result.
- Click deny on a fresh request. Verify the next turn's system message says denied.
- Don't click for 5 minutes. Verify the next turn surfaces `timed_out`.
- Reload the browser mid-pending. Verify the row hydrates correctly (PR #136's contract preserved).

## Out-of-scope explicit declarations (so we don't regret it later)

- **Race tests on concurrent resolve** — punted to last-write-wins with `status == "pending"` guard.
- **Stress on many simultaneous pending approvals** — measure if/when we see real load.
- **LLM behavior on receiving pending tool results** — not unit-testable; observed in manual smoke and adjusted via prompt engineering if the agent confuses itself.
- **Decision-outcome rendering inline in chat (web)** — future UI work; the data is now available on `approval_resolved` but rendering is deferred.
- **Per-function trigger/block mode flag** — full replacement; reintroduce only if a concrete read-side approval use case appears.

## Open questions

None blocking. The two judgement calls already made:

- **Two-phase ack vs. atomic stamp:** chose two-phase (list → LLM → ack) for at-least-once correctness on crash.
- **Lazy timeout vs. sweeper task:** chose lazy. No periodic task; readers flip on the fly.

## Decision log

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Agent awareness of gating | Yes — agent sees `pending_approval` tool result | Lets the agent plan around the wait instead of stalling. |
| Resolution delivery | System message in next LLM turn | Simpler than tool_call_result replay; no turn-orchestrator state-machine changes. |
| Migration | Full replacement | One mode, less code, no flag. APPROVAL_REQUIRED set is small enough that breaking the synchronous contract is acceptable. |
| Implementation approach | A1: reuse hook protocol with `pending` reason | No iii SDK changes; ships in one PR. Semantic smell (block-with-not-really-blocked) contained to one comment. |
| Ack protocol | Two-phase (list, LLM, ack) | At-least-once on crash; idempotent on LLM side. |
| Timeout enforcement | Lazy in every read path | Removes the polling shape PR #136 just deleted. |
