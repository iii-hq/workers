# approval-gate

Hook subscriber on `agent::before_function_call` that consults
`policy::check_permissions` and parks calls for `approval::resolve` when a
call matches no rule. Resume is fully reactive — writing the decision to
state is itself the wake-up event.

## Purpose

The approval gate is the runtime enforcement of
[iii-permissions.yaml](iii-permissions.yaml). Every dispatch from
`agent::call` publishes the call envelope to `agent::before_function_call`;
the gate is the durable subscriber on that topic. It asks the harness
worker's `policy::check_permissions` for a decision and returns
**synchronously** — there is no polling await-loop. The three outcomes:

| Policy outcome | Gate reply | Orchestrator effect |
|---|---|---|
| `allow` | `{ block: false, subscriber, approval_gate }` | dispatch proceeds |
| `deny` | `{ block: true, reason, denial, subscriber, approval_gate }` + emits `function_call_denied` | dispatch short-circuits with the denial |
| `needs_approval` | `{ block: true, status: 'pending', subscriber, approval_gate }` + emits `approval_requested` | orchestrator parks the call on its turn record (`function_awaiting_approval`) and stops stepping until a decision lands |

Resume happens when an operator calls `approval::resolve`. The resolve
handler writes the decision to `approvals/<sid>/<call_id>`; a `state`
trigger on the approvals scope (`approval::on_decision_written`, gated by
the condition function `approval::is_decision_write`) then publishes
`turn::step_requested` so the orchestrator's FSM picks up where it paused
and reads the decision back out of state. The gate itself owns no
in-process pending map.

Fail-closed: the orchestrator's `consultBefore` denies the call with a
`gate_unavailable` envelope if the hook fanout fails or no subscriber
replies, so a missing/erroring gate never lets a call through.

## Registered functions

- `approval::resolve` — Write the final approval decision (`allow` or `deny`) for a pending call. The state write is itself the wake-up event.
- `policy::approval_gate` — Consult `policy::check_permissions` and reply allow, deny, or pending.
- `approval::is_decision_write` — Condition function bound to the approvals state trigger; returns `true` only for `state:created`/`state:updated` events whose `new_value.decision` is a string.
- `approval::on_decision_written` — State trigger adapter: extracts `session_id` from the `<sid>/<cid>` key and publishes `turn::step_requested`.

## Triggers

- **Durable subscriber** on `agent::before_function_call` (configurable via
  `approval_gate.topic`) → `policy::approval_gate`. Registered in
  [src/approval-gate/register.ts](harness-node/src/approval-gate/register.ts).
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

## Approval / agent events

Bound to `agent::events` so the UI can render the gate's lifecycle:

| Event type | When | Written by |
|---|---|---|
| `approval_requested` | needs_approval outcome | `gate-subscriber.ts` |
| `function_call_denied` | deny outcome | `gate-subscriber.ts` |
| `approval_resolved` | `approval::resolve` succeeds | `handleResolveWithEvents` in `pending.ts` |

## Configuration

From the `approval_gate` section of
[config.yaml](harness-node/config.yaml):

- `topic` (default `agent::before_function_call`) — durable topic the gate
  subscribes to.
- `approval_state_scope` (default `approvals`) — iii state scope for
  decision records. The state trigger that wakes the orchestrator is
  registered against this same scope.
- `policy_function_id` (default `policy::check_permissions`) — function id
  consulted for the rule decision.

There is no `default_timeout_ms`: the gate returns synchronously and the
turn record sits in `function_awaiting_approval` until a decision is
written, so the only relevant timeout is the durable bus's own backstop.

## Dependencies

From
[src/approval-gate/iii.worker.yaml](harness-node/src/approval-gate/iii.worker.yaml):
no explicit dependency block (the gate calls `policy::check_permissions`
on the harness worker over the bus and reads/writes iii state).

## Source layout

| File | Purpose |
|---|---|
| [src/approval-gate/main.ts](harness-node/src/approval-gate/main.ts) | Binary entry point (`iii-approval-gate`). |
| [src/approval-gate/register.ts](harness-node/src/approval-gate/register.ts) | Registers `approval::resolve`, `policy::approval_gate`, the decision-written trigger pair, and the durable subscriber. |
| [src/approval-gate/config.ts](harness-node/src/approval-gate/config.ts) | Loads the `approval_gate` config section. |
| [src/approval-gate/types.ts](harness-node/src/approval-gate/types.ts) | Wire types: `IncomingCall`, `DenialEnvelope`, `MatchedConstraint`, `GateBlockReply`, `pendingKey`, `blockReplyFor`, `extractCall`. |
| [src/approval-gate/state-bus.ts](harness-node/src/approval-gate/state-bus.ts) | `StateBus` interface + `IiiStateBus` implementation. |
| [src/approval-gate/policy-consult.ts](harness-node/src/approval-gate/policy-consult.ts) | Calls `policy::check_permissions` and decodes the decision. |
| [src/approval-gate/gate-subscriber.ts](harness-node/src/approval-gate/gate-subscriber.ts) | `policy::approval_gate` handler: extracts the call, consults policy, emits the matching `agent::events` frame, returns the hook reply synchronously. |
| [src/approval-gate/pending.ts](harness-node/src/approval-gate/pending.ts) | `handleResolve` writes `{decision, reason}` to `approvals/<sid>/<cid>`; `handleResolveWithEvents` adds the `approval_resolved` agent event. |
| [src/approval-gate/on-decision-written.ts](harness-node/src/approval-gate/on-decision-written.ts) | State trigger adapter — `approval::is_decision_write` (condition) + `approval::on_decision_written` (handler) — publishes `turn::step_requested` when a decision lands. |
| [src/approval-gate/denial.ts](harness-node/src/approval-gate/denial.ts) | Builds `DenialEnvelope` instances (permissions, user, gate_unavailable). |
| [src/approval-gate/iii.worker.yaml](harness-node/src/approval-gate/iii.worker.yaml) | Worker manifest. |
