# approval-gate

Owns approval state. Registers `approval::resolve` and a state-trigger adapter
on the `approvals` scope that wakes `turn::step` on every decision write.

## Purpose

The approval gate is the runtime resolution surface for
[iii-permissions.yaml](iii-permissions.yaml). It does **not** intercept
function calls on the bus — the turn-orchestrator consults
`policy::check_permissions` directly inside `consultBefore`. The gate's job
is to accept human decisions and release parked turns.

| Policy outcome (in orchestrator) | Orchestrator effect |
|---|---|
| `allow` | dispatch proceeds immediately |
| `deny` | dispatch short-circuits with a `DenialEnvelope` |
| `needs_approval` | orchestrator parks the call in `function_awaiting_approval` and stops re-publishing `turn::step_requested` until a decision lands |

Resume happens when an operator calls `approval::resolve`. The resolve
handler writes the decision to `approvals/<sid>/<call_id>`; the state
trigger (`approval::on_decision_written`, gated by
`approval::is_decision_write`) then directly triggers `turn::step` for
the affected session — the same primitive the orchestrator uses in its own
`on-record-written` pattern. The gate itself owns no in-process pending map.

## Topology

approval-gate owns two pieces of state surface:

1. The `approval::resolve` function — bus entry point the UI calls to write a
   decision into `approvals/<session_id>/<function_call_id>`.
2. A state trigger on `scope: approvals` (condition: `new_value.decision` is a
   string) whose handler directly triggers `turn::step` for the affected
   session — the same primitive the orchestrator uses for `on-record-written`.

The orchestrator consults policy directly (`policy::check_permissions`) inside
`consultBefore`; approval-gate is no longer a hook subscriber and there is no
`agent::before_function_call` topic.

## Registered functions

- `approval::resolve` — Write the final approval decision (`allow` or `deny`) for a pending call. The state write is itself the wake-up event.
- `approval::is_decision_write` — Condition function bound to the approvals state trigger; returns `true` only for `state:created`/`state:updated` events whose `new_value.decision` is a string.
- `approval::on_decision_written` — State trigger adapter: extracts `session_id` from the `<sid>/<cid>` key and directly triggers `turn::step` for that session.

## Triggers

- **State trigger** on `scope: approvals` gated by
  `condition_function_id: approval::is_decision_write` →
  `approval::on_decision_written`. This is what wakes the orchestrator
  after a human resolves an approval.

## State keys

All keys live under the iii state scope configured by
`approval_gate.approval_state_scope` (default `approvals`). From
[src/approval-gate/types.ts](harness-node/src/approval-gate/types.ts) and
[src/approval-gate/pending.ts](harness-node/src/approval-gate/pending.ts):

| Key shape | Value | Purpose |
|---|---|---|
| `<session_id>/<function_call_id>` | `{ decision: 'allow' \| 'deny' \| 'aborted', reason: string \| null }` | One record per **resolved** approval. The record only exists once a decision (or abort) lands; pending is implicit in the absence of a record. The orchestrator's `function_awaiting_approval` state reads this key. |

`'aborted'` decisions are written by
[src/turn-orchestrator/abort.ts](harness-node/src/turn-orchestrator/abort.ts)
when `router::abort` fires while the turn is paused on approvals.

## Pending-approval signalling

The frontend does not consume signal events for approvals. Instead, the
orchestrator emits a `turn_state_changed` event on every `turn_state`
write (see `harness-node/docs/workers/turn-orchestrator.md`). The
console reads `awaiting_approval` from the new record to render
approve/deny buttons, and uses `function_execution_end` (which already
carries the blocked result for denied calls via the orchestrator's
`handleExecute` blocked-result branch) to close the card. On page
reload the console fires a one-shot `state::get { scope: 'agent', key:
'session/<sid>/turn_state' }` to recover any modals that were pending
when the page loaded.

## Configuration

From the `approval_gate` section of
[config.yaml](harness-node/config.yaml):

- `approval_state_scope` (default `approvals`) — iii state scope for
  decision records. The state trigger that wakes the orchestrator is
  registered against this same scope.

The policy function id is owned by the orchestrator's config slice, not
approval-gate's — set it at top level as `policy_function_id` (default
`policy::check_permissions`). See `harness-node/config.yaml` and
[workers/turn-orchestrator.md](workers/turn-orchestrator.md).

There is no `default_timeout_ms`: the turn record sits in
`function_awaiting_approval` until a decision is written, so the only relevant
timeout is the durable bus's own backstop.

## Dependencies

From
[src/approval-gate/iii.worker.yaml](harness-node/src/approval-gate/iii.worker.yaml):
no explicit dependency block (the gate reads/writes iii state).

## Source layout

| File | Purpose |
|---|---|
| [src/approval-gate/main.ts](harness-node/src/approval-gate/main.ts) | Binary entry point (`iii-approval-gate`). |
| [src/approval-gate/register.ts](harness-node/src/approval-gate/register.ts) | Registers `approval::resolve` and the decision-written trigger pair. |
| [src/approval-gate/config.ts](harness-node/src/approval-gate/config.ts) | Loads the `approval_gate` config section. |
| [src/approval-gate/types.ts](harness-node/src/approval-gate/types.ts) | Wire types and constants: `DenialEnvelope`, `MatchedConstraint`, `WireDecision`, `DeniedBy`, `FN_RESOLVE`, `STATE_SCOPE`, `pendingKey`. |
| [src/approval-gate/policy-consult.ts](harness-node/src/approval-gate/policy-consult.ts) | Calls `policy::check_permissions` and decodes the decision (imported by the orchestrator's `hook.ts`). |
| [src/approval-gate/pending.ts](harness-node/src/approval-gate/pending.ts) | `handleResolve` writes `{decision, reason}` to `approvals/<sid>/<cid>`. |
| [src/approval-gate/on-decision-written.ts](harness-node/src/approval-gate/on-decision-written.ts) | State trigger adapter — `approval::is_decision_write` (condition) + `approval::on_decision_written` (handler) — directly triggers `turn::step` when a decision lands. |
| [src/approval-gate/denial.ts](harness-node/src/approval-gate/denial.ts) | Builds `DenialEnvelope` instances (permissions, user, gate_unavailable). |
| [src/approval-gate/iii.worker.yaml](harness-node/src/approval-gate/iii.worker.yaml) | Worker manifest. |
