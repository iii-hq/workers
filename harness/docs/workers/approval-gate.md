# approval-gate

Registers `approval::resolve`, the per-session approval-settings handlers
(`approval::set_mode`, `approval::add_always_allow`, …), and shared wire
schemas for the approval path. The turn-orchestrator reacts via the reactive
`turn::on_approval` state trigger.

## Purpose

The approval gate is the bus entry point for human decisions on parked tool
calls. It does **not** intercept function calls on the bus — the
turn-orchestrator consults `policy::check_permissions` directly inside
`consultBefore`, then applies per-session mode + always-allow before falling
back to the yaml policy. The gate's job is to accept operator input from the
console and persist the decisions (resolutions, mode changes, allow-list
mutations) where the orchestrator can read them.

## Permission modes (per-session)

Each session has a permission mode stored at
`approval_settings/<session_id>`. The turn-orchestrator's `consultBefore`
snapshots this record at the start of each call, then evaluates in order:

1. **human-only block.** If the agent tries to call any of
   `approval::set_mode`, `approval::add_always_allow`,
   `approval::remove_always_allow`, `approval::approve_always`,
   `approval::get_settings`, `approval::clear_settings`, the call is
   denied with rule `human_only_function`. Those handlers are only
   reachable from the user-initiated RPC path; the agent's
   `dispatchWithHook` route can never self-escalate.
2. **`mode === 'full'`** → allow. No safety floor; the agent may call any
   function. The console renders a persistent banner while this mode is
   active.
3. **`function_id ∈ approved_always`** → allow, **in every mode**. These
   are per-session "approve always" grants made from an approval prompt
   ("approve always" button). They are remembered human decisions, not an
   auto-policy, so they hold under Manual as well as Auto.
4. **`mode === 'auto'` AND `function_id ∈ always_allow`** → allow. The
   `always_allow` list is a user-curated trust profile, seeded from the
   Configuration screen's default allowlist and consulted **only in
   Auto**. Dormant under Manual.
5. **Fallback** → `policy::check_permissions` against `iii-permissions.yaml`.
   Manual mode never short-circuits past this step; everything except
   yaml `allow` rules and `approved_always` grants prompts.

### Mode summary

| Mode | yaml `allow` | approved_always | always_allow | yaml `deny` | otherwise |
|---|---|---|---|---|---|
| `manual` | allow | allow | dormant | deny | prompt |
| `auto` | allow | allow | allow | deny | prompt |
| `full` | allow | allow | allow | allow | allow |

| Policy outcome (in orchestrator) | Orchestrator effect |
|---|---|
| `allow` | dispatch proceeds immediately |
| `deny` | dispatch short-circuits with a `DenialEnvelope` |
| `needs_approval` | orchestrator parks the call in `function_awaiting_approval` |

## Resolution flow

1. While parked, the orchestrator keeps pending calls in `awaiting_approval[]` on the turn record.
2. The console calls `approval::resolve` with `{ session_id, function_call_id, decision, reason? }`.
3. `approval::resolve` writes `approvals/<sid>/<cid>` via `state::set`.
4. The `turn::on_approval` state trigger (scope `approvals`) enqueues `turn::function_awaiting_approval`.
5. `function_awaiting_approval` executes each resolved call immediately, removes it from `awaiting_approval[]`, and stays parked until none remain; then finalizes the batch or returns to `function_execute`.

## Registered functions

- `approval::resolve` — Validates the payload and persists the decision to scope `approvals`. Returns `{ ok: true }` or `{ ok: false, error: 'invalid_payload' | 'resume_failed' }`.
- `approval::set_mode` — Persists `{ mode }` to scope `approval_settings`. **Human-only**: the orchestrator hook denies this id when called by the agent.
- `approval::add_always_allow` — Idempotent append to the auto-mode allow-list. **Human-only**.
- `approval::remove_always_allow` — Remove an entry from the auto-mode allow-list. **Human-only**.
- `approval::approve_always` — Idempotent append to the per-session `approved_always` grants (honored in every mode). **Human-only**.
- `approval::get_settings` — Read current settings, returning defaults if none persisted. **Human-only**.
- `approval::clear_settings` — Drop the session's record on conversation delete. **Human-only**.

Reactive wake is owned by the turn-orchestrator:

- `turn::on_approval` — State trigger on scope `approvals`; enqueues `turn::{state}` for the parked session.

## State keys

Decision records use scope `approvals` (constant `STATE_SCOPE` in
[src/approval-gate/schemas.ts](harness/src/approval-gate/schemas.ts));
per-session permission settings live in scope `approval_settings`
(constant `SETTINGS_STATE_SCOPE`):

| Scope | Key | Value | Writer | Purpose |
|---|---|---|---|---|
| `approvals` | `<session_id>/<function_call_id>` | `{ decision: 'allow' \| 'deny' \| 'aborted', reason: string \| null }` | `approval::resolve` | Decision pickup for parked calls. |
| `approval_settings` | `<session_id>` | `{ mode, always_allow: AlwaysAllowEntry[], approved_always: AlwaysAllowEntry[], mode_set_at }` | `approval::set_mode`, `approval::add_always_allow`, `approval::remove_always_allow`, `approval::approve_always` | Snapshot read by `consultBefore` before consulting the yaml policy. |

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
| [src/approval-gate/resolve.ts](harness/src/approval-gate/resolve.ts) | Registers `approval::resolve`; persists decisions to scope `approvals`. |
| [src/approval-gate/settings/](harness/src/approval-gate/settings/) | Per-session mode/allow-list store, mutations, and handler registration (`readSettings`, `isHumanOnlyApprovalFunction`, `registerSettingsHandlers`). |
| [src/approval-gate/schemas.ts](harness/src/approval-gate/schemas.ts) | `STATE_SCOPE`, `SETTINGS_STATE_SCOPE`, wire schemas, `ApprovalSettingsSchema`, `parsePolicyReply`, `pendingKey`, `ApprovalDecisionSchema`, `ResolvePayloadSchema`. |
| [src/approval-gate/denial.ts](harness/src/approval-gate/denial.ts) | `permissionsDenyEnvelope` and related helpers. |
| [src/approval-gate/redact.ts](harness/src/approval-gate/redact.ts) | `redact` / `clip` for safe `args_excerpt` on denials. |
| [src/approval-gate/iii.worker.yaml](harness/src/approval-gate/iii.worker.yaml) | Worker manifest. |

Related orchestrator code:
[function-awaiting-approval/process.ts](harness/src/turn-orchestrator/function-awaiting-approval/process.ts) (registers `turn::on_approval`),
[hook.ts](harness/src/turn-orchestrator/hook.ts).
