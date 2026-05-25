# approval-gate

Registers `approval::resolve` and shared wire schemas for the approval path.
Per-call resume functions (`turn::approval_resume::<session>/<call>`) live in
the turn-orchestrator.

## Purpose

The approval gate is the bus entry point for human decisions on parked tool
calls. It does **not** intercept function calls on the bus — the
turn-orchestrator consults `policy::check_permissions` directly inside
`consultBefore`. The gate's job is to accept operator input from the console
and route it to the correct per-call resume function.

| Policy outcome (in orchestrator) | Orchestrator effect |
|---|---|
| `allow` | dispatch proceeds immediately |
| `deny` | dispatch short-circuits with a `DenialEnvelope` |
| `needs_approval` | orchestrator parks the call in `function_awaiting_approval` and registers a resume fn |

## Resolution flow

1. While parked, the orchestrator calls `registerApprovalResume` for each
   pending call (see [approval-resume.ts](harness/src/turn-orchestrator/approval-resume.ts)).
2. The console calls `approval::resolve` with `{ session_id, function_call_id, decision, reason? }`.
3. `approval::resolve` triggers `turn::approval_resume::<sid>/<cid>` with the decision payload.
4. The resume handler writes `approvals/<sid>/<cid>` (if not already set), invokes `turn::step`, and unregisters the resume fn.
5. `handleAwaitingApproval` reads all decisions, folds them into the prepared snapshot, and returns to `function_execute`.

## Registered functions

- `approval::resolve` — Validates the payload and triggers the per-call resume function. Returns `{ ok: true }` or `{ ok: false, error: 'invalid_payload' | 'resume_failed' }`.

Per-call resume functions are registered by the turn-orchestrator, not this worker:

- `turn::approval_resume::<session_id>/<function_call_id>` — Persists the decision to scope `approvals` and wakes `turn::step`.

## State keys

All decision records use scope `approvals` (constant `STATE_SCOPE` in
[src/approval-gate/schemas.ts](harness/src/approval-gate/schemas.ts)):

| Key shape | Value | Purpose |
|---|---|---|
| `<session_id>/<function_call_id>` | `{ decision: 'allow' \| 'deny' \| 'aborted', reason: string \| null }` | Written by the resume handler when an operator resolves. `handleAwaitingApproval` reads these keys while the turn is in `function_awaiting_approval`. |

Pending calls are tracked on the turn record (`awaiting_approval[]`), not as
separate rows under `approvals` until a decision lands.

## Denial envelopes

[src/approval-gate/denial.ts](harness/src/approval-gate/denial.ts) builds
`DenialEnvelope` values for policy denies (`permissionsDenyEnvelope` via
`consultBefore` in [hook.ts](harness/src/turn-orchestrator/hook.ts)).
[src/approval-gate/redact.ts](harness/src/approval-gate/redact.ts)
sanitizes tool args for `args_excerpt` on those envelopes.

## Pending-approval signalling

The frontend does not consume a dedicated approval signal event. The
orchestrator emits `turn_state_changed` on every `turn_state` write; the
console derives pending approvals from `awaiting_approval` on the mirrored
record. On reload it uses `turn::get_state` (not direct iii state reads).

## Configuration

There is no `approval_gate` section in [config.yaml](harness/config.yaml).
Scope `approvals` is fixed in code (`STATE_SCOPE`).

Policy consultation is a direct `iii.trigger` to `policy::check_permissions`
from turn-orchestrator `consultBefore`. See
[workers/turn-orchestrator.md](workers/turn-orchestrator.md).

## Dependencies

From
[src/approval-gate/iii.worker.yaml](harness/src/approval-gate/iii.worker.yaml):
no explicit dependency block.

## Source layout

| File | Purpose |
|---|---|
| [src/approval-gate/main.ts](harness/src/approval-gate/main.ts) | Binary entry point (`iii-approval-gate`). |
| [src/approval-gate/resolve.ts](harness/src/approval-gate/resolve.ts) | Registers `approval::resolve`; triggers per-call resume fns. |
| [src/approval-gate/schemas.ts](harness/src/approval-gate/schemas.ts) | `STATE_SCOPE`, wire schemas, `parsePolicyReply`, `pendingKey`, `approvalResumeFnId`, `ResolvePayloadSchema`. |
| [src/approval-gate/denial.ts](harness/src/approval-gate/denial.ts) | `permissionsDenyEnvelope` and related helpers. |
| [src/approval-gate/redact.ts](harness/src/approval-gate/redact.ts) | `redact` / `clip` for safe `args_excerpt` on denials. |
| [src/approval-gate/iii.worker.yaml](harness/src/approval-gate/iii.worker.yaml) | Worker manifest. |

Related orchestrator code:
[approval-resume.ts](harness/src/turn-orchestrator/approval-resume.ts),
[hook.ts](harness/src/turn-orchestrator/hook.ts).
