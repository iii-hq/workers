# approval-gate

Hook subscriber on `agent::before_function_call` that consults
`policy::check_permissions` and pauses for `approval::resolve` when a call
matches no rule.

## Purpose

The approval gate is the runtime enforcement of
[iii-permissions.yaml](iii-permissions.yaml). Every dispatch from
`agent::call` publishes the call envelope to `agent::before_function_call`;
the gate is the durable subscriber on that topic. It asks the harness
worker's `policy::check_permissions` for a decision; an explicit `allow` or
`deny` is replied immediately, a `needs_approval` decision parks the call
in iii state and runs a polling await-loop until either the operator
resolves it via `approval::resolve` or the configured timeout elapses.

Fail-closed: if `policy::check_permissions` is unavailable or errors out,
the gate replies `deny` with a `gate_unavailable` denial envelope so the
caller (the orchestrator's `dispatchWithHook`) never executes an
unverified call.

## Registered functions

- `approval::resolve` — Flip a pending approval entry to allow or deny.
- `approval::list_pending` — Return pending approvals for a session.
- `policy::approval_gate` — Consult `policy::check_permissions` and either allow, deny, or pause for user resolution via `approval::resolve`.

## Triggers

- Durable subscriber on `agent::before_function_call` (configurable via the
  `approval_gate.topic` config key) → `policy::approval_gate`. Registered
  in [src/approval-gate/register.ts](harness-node/src/approval-gate/register.ts).

## State keys

All keys live under the iii state scope configured by
`approval_gate.approval_state_scope` (default `approvals`). From
[src/approval-gate/types.ts](harness-node/src/approval-gate/types.ts):

| Key shape | Purpose |
|---|---|
| `<session_id>/<function_call_id>` | One record per pending / resolved approval. Status is `pending`, `allow`, or `deny`. Each record holds `function_call_id`, `function_id`, `args`, `status`, `expires_at`, optional `reason`. |

`approval::list_pending` returns every record under
`<session_id>/` whose `status === 'pending'`.

## Configuration

From the `approval_gate` section of
[config.yaml](harness-node/config.yaml):

- `topic` (default `agent::before_function_call`) — durable topic the gate
  subscribes to.
- `approval_state_scope` (default `approvals`) — iii state scope for
  pending records.
- `default_timeout_ms` (default `300000`) — how long to wait for an
  operator decision before replying `deny` with `reason: timeout`.
- `policy_function_id` (default `policy::check_permissions`) — function id
  consulted for the rule decision.

## Dependencies

From
[src/approval-gate/iii.worker.yaml](harness-node/src/approval-gate/iii.worker.yaml):
no explicit dependency block (the gate calls `policy::check_permissions`
on the harness worker over the bus and reads/writes iii state).

## Source layout

| File | Purpose |
|---|---|
| [src/approval-gate/main.ts](harness-node/src/approval-gate/main.ts) | Binary entry point (`iii-approval-gate`). |
| [src/approval-gate/register.ts](harness-node/src/approval-gate/register.ts) | Registers the three functions + the durable subscriber. |
| [src/approval-gate/config.ts](harness-node/src/approval-gate/config.ts) | Loads the `approval_gate` config section. |
| [src/approval-gate/types.ts](harness-node/src/approval-gate/types.ts) | Wire types: `IncomingCall`, `DenialEnvelope`, `MatchedConstraint`, `pendingKey`, `buildPendingRecord`, `extractCall`. |
| [src/approval-gate/state-bus.ts](harness-node/src/approval-gate/state-bus.ts) | `StateBus` interface + `IiiStateBus` implementation. |
| [src/approval-gate/policy-consult.ts](harness-node/src/approval-gate/policy-consult.ts) | Calls `policy::check_permissions` and decodes the decision. |
| [src/approval-gate/gate-subscriber.ts](harness-node/src/approval-gate/gate-subscriber.ts) | Main subscriber: extracts the call, consults policy, either resolves immediately or parks + awaits, writes the hook reply. |
| [src/approval-gate/pending.ts](harness-node/src/approval-gate/pending.ts) | `handleResolve` / `handleListPending` + the `awaitDecision` poll loop. |
| [src/approval-gate/denial.ts](harness-node/src/approval-gate/denial.ts) | Builds `DenialEnvelope` instances (permissions, user, gate_unavailable). |
| [src/approval-gate/iii.worker.yaml](harness-node/src/approval-gate/iii.worker.yaml) | Worker manifest. |
