# turn-orchestrator

Durable `run::start` state machine that drives each agent turn through
provisioning, assistant, function-execute, steering, and session finish.

## Purpose

This is the heart of the bundle. `run::start` opens a session and returns
immediately; the rest of the work happens inside per-state durable functions
(`turn::provisioning`, `turn::assistant_streaming`, …), each enqueued onto
the `turn-step` FIFO queue inline from `saveRecord`.
Saving the record with a new non-terminal, non-parking state automatically
enqueues the next handler (`saveRecord` in
[state-runtime/store.ts](harness/src/turn-orchestrator/state-runtime/store.ts) calls `shouldWakeStep` then enqueues on the `turn-step` FIFO).

Every per-state handler is wrapped by `runTransition`
([run-transition.ts](harness/src/turn-orchestrator/run-transition.ts)):
load record → null-check → stale-skip → handle → save. This owns the
crash-isolation contract:

- An unexpected handler throw routes the session to the `failed` terminal
  (acked so the durable queue stops retrying) and surfaces `message_complete{stop_reason:'error'}` + `agent_end` to the UI.
- A handler may throw `TransientError`
  ([errors.ts](harness/src/turn-orchestrator/errors.ts)) to opt into the
  queue's retry/backoff/DLQ instead of the terminal path.

`dispatchWithHook` in [agent-trigger.ts](harness/src/turn-orchestrator/agent-trigger.ts)
is the single chokepoint every agent-issued function call passes through.
It runs `consultBefore` before forwarding to the target function id.
`consultBefore` triggers `policy::check_permissions` directly (5 s timeout)
and maps the reply to `allow` / `deny` / `pending`. Fail-closed: policy
unreachable → deny with a `gate_unavailable` `DenialEnvelope`.

## Registered functions

- `run::start` — Persist run config and messages, seed `turn_state` to
  `provisioning`, and wake the FSM via `saveRecord`.
- `turn::provisioning` — FSM step: build system prompt + single `agent_trigger` schema, write enriched `run_request`, advance to `assistant_streaming`.
- `turn::assistant_streaming` — FSM step: stream the turn over a provider channel; on completion emit `message_complete`, persist the assistant message (dup-guarded), route to `function_execute` / `steering_check` / `stopped` (via `finishSession`).
- `turn::function_execute` — FSM step: own the full function lifecycle via `rec.work`; build batch from `rec.last_assistant`, run each call (skip already-executed and awaiting-approval ids), checkpoint per-call via `writeRecord`; if `pending` → append to `awaiting_approval` and keep dispatching the remaining calls (pending does not block siblings); park to `function_awaiting_approval` when any call awaits approval; finalize results into messages + emit `turn_end` when the batch completes → `steering_check` / `stopped` (via `finishSession`).
- `turn::function_awaiting_approval` — FSM step: on each wake, read decisions for individual `awaiting_approval[]` entries; execute each resolved call immediately (`allow` → dispatch pre-approved; `deny`/`aborted` → synthetic denial); remove resolved entries; stay parked while any remain; when none remain → `finalizeBatch` if complete else `function_execute`.
- `turn::steering_check` — FSM step: drain `steering`/`followup` inboxes, enforce `max_turns` cap (emits synthetic `max_turns` message + `turn_end` → `stopped` via `finishSession`), route to `assistant_streaming` / `stopped`.
- `turn::get_state` — One-shot reader returning a lean `TurnStateView` (from `schemas.ts:toView`) for a session. UI clients call this on reload to recover in-progress modals (e.g. `function_awaiting_approval`) without reading iii state directly. Returns `null` for unknown sessions.

## Triggers

The record-written wake is inline in `saveRecord` (no separate `on-record-written` adapter): every `saveRecord` call that transitions to a non-terminal, non-parking state enqueues `turn::{newState}` on the `turn-step` FIFO. Similarly, `turn_state_changed` events are emitted inline from `persistRecord` inside `TurnStore` — there is no separate `on-turn-state-changed` state trigger.

Paused turns (`function_awaiting_approval`) are woken when `approval::resolve` writes scope `approvals`, which fires `turn::on_approval` (registered in [function-awaiting-approval/process.ts](harness/src/turn-orchestrator/function-awaiting-approval/process.ts); see [workers/approval-gate.md](workers/approval-gate.md)).

## Turn FSM

Each state is a registered `turn::{state}` function executed via
`runTransition` and enqueued onto the `turn-step` FIFO queue from `saveRecord` when `shouldWakeStep` allows.
The 7 states from [state.ts](harness/src/turn-orchestrator/state.ts):

| State | Handler file | Role |
|---|---|---|
| `provisioning` | [provisioning/process.ts](harness/src/turn-orchestrator/provisioning/process.ts) | Fetch skills index + default-skill bodies, build system prompt, write enriched `run_request` (with `function_schemas: [agentTriggerTool()]`), → `assistant_streaming`. |
| `assistant_streaming` | [assistant-streaming/process.ts](harness/src/turn-orchestrator/assistant-streaming/process.ts) | Increment `turn_count`; create channel; trigger provider stream; relay `message_update` deltas; on completion call `finalizeAssistantTurn` which emits `message_complete`, persists the assistant message (dup-guarded), then routes → `function_execute` (has calls) / `steering_check` (no calls) / `stopped` via `finishSession` (error/aborted). |
| `function_execute` | [function-execute/process.ts](harness/src/turn-orchestrator/function-execute/process.ts) | Build batch from `rec.last_assistant` (or reuse existing `rec.work`); for each call: emit `function_execution_start`, skip if already executed or awaiting approval, dispatch via `dispatchWithHook`; if `pending` → append to `awaiting_approval` and continue other calls; park to `function_awaiting_approval` when any call awaits; otherwise commit result (silent `writeRecord` checkpoint) + emit `function_execution_end`; after batch: fold results into messages + emit `turn_end` → `steering_check` / `stopped` via `finishSession`. |
| `function_awaiting_approval` | [function-awaiting-approval/process.ts](harness/src/turn-orchestrator/function-awaiting-approval/process.ts) | On each wake: for each `awaiting_approval[]` entry with a decision, execute immediately (`allow` → pre-approved dispatch; `deny`/`aborted` → synthetic denial); remove resolved entries; stay parked while any remain; when none remain → `finalizeBatch` if complete else `function_execute`. |
| `steering_check` | [steering-check/process.ts](harness/src/turn-orchestrator/steering-check/process.ts) | Priority route: steering msg → `assistant_streaming` (unless `max_turns` reached); followup msg → `assistant_streaming` (unless `max_turns` reached); function results present → `assistant_streaming` (unless `max_turns` reached); else emit `turn_end` once → `stopped` via `finishSession`. `max_turns` path emits a synthetic `message_complete` + `turn_end`. |
| `stopped` | (no handler) | Terminal. Idempotent. Session teardown (`agent_end`) happens inline via `TurnStatePorts.finishSession` before entering this state. |
| `failed` | (set by `runTransition` on unexpected throw) | Terminal. Carries `error: {kind, message}` on the record. Emits `message_complete{stop_reason:'error'}` + `agent_end` so the UI sees the reason. A handler may throw `TransientError` to use the queue's retry/DLQ instead. |

`NON_STEPABLE_STATES` in [store.ts](harness/src/turn-orchestrator/state-runtime/store.ts) are
`stopped`, `failed`, and `function_awaiting_approval` — `saveRecord` does not
enqueue a handler for these.

`dispatchWithHook` returns `{ kind: 'result', result }` or `{ kind: 'pending' }`.
Policy denies are returned as `{ kind: 'result' }` with a denied `FunctionResult`.
`pending` triggers the `function_awaiting_approval` park. Multiple calls may
await approval concurrently; each is executed individually as its decision
arrives.

## State scopes

Session-scoped iii state uses semantic scopes from
[state.ts](harness/src/turn-orchestrator/state.ts) with
`session_id` as the key. I/O goes through
[state-runtime/store.ts](harness/src/turn-orchestrator/state-runtime/store.ts) (`TurnStore`).

| Scope | Key | Purpose |
|---|---|---|
| `turn_state` | `<session_id>` | Serialised `TurnStateRecord` (incl. `work?: TurnWork` and `error?: {kind, message}`). |
| `messages` | `<session_id>` | Active path `AgentMessage[]`; mirrored into `session-tree::*` on every save (inline in `TurnStore.saveMessages` / `appendMessages`). |
| `run_request` | `<session_id>` | The `run::start` payload enriched by `provisioning` to include `function_schemas: [agentTriggerTool()]` and the assembled `system_prompt`. Typed as `RunRequest` ([run-request.ts](harness/src/turn-orchestrator/run-request.ts)). |
| `session_tree_mirror_len` | `<session_id>` | High-water mark so the session-tree messages mirror is incremental. |
| `event_counter` | `<session_id>` | Monotonic counter for `agent::events` sequence numbers. |

Keys that no longer exist: `function_prepared`, `function_executed`,
`function_schemas` (standalone), `tool_prepared`, `tool_executed`,
`tool_schemas`, `sandbox_id`, `last_compaction_at`,
`last_compaction_consumed_at` — these were removed in the rewrite.

The `TurnStateRecord` carries `work?: TurnWork` (inline `{ prepared: PreparedCall[]; executed: Record<id, ExecutedCall> }`) in place of the former separate state keys. `PreparedCall`, `ExecutedCall`, and `TurnWork` are defined in [function-execute/types.ts](harness/src/turn-orchestrator/function-execute/types.ts).

## UI events

`turn_state_changed` is emitted inline by `TurnStore.saveRecord` on every
persist that goes through the full save path. It carries a lean
`TurnStateView` (not the full `TurnStateRecord`) as `new_value` (and
`old_value` when updating). `TurnStateView` is defined in
[schemas.ts](harness/src/turn-orchestrator/schemas.ts) and contains:
`session_id`, `state`, `turn_count`, `max_turns`, `awaiting_approval`, `error`.

`turn::get_state` also returns a `TurnStateView` (via `toView`), not the full
record, so heavy internal fields (`work`, `last_assistant`) are never sent to
consumers.

## Approval chokepoint

Unchanged from prior design: `dispatchWithHook` → `consultBefore` →
`policy::check_permissions` (5 s timeout, fail-closed). A `needs_approval`
reply returns `{ kind: 'pending' }` from `dispatchWithHook`, which parks the
session to `function_awaiting_approval`. `approval::resolve` writes the
decision to scope `approvals`, which fires `turn::on_approval` to enqueue `turn::function_awaiting_approval` on the `turn-step` queue.

## Configuration

From the top-level `turn-orchestrator` section of
[config.yaml](harness/config.yaml):

- `system_default_skills` (default `["iii://iii-directory/index"]`) —
  skill URIs the bootstrap step downloads into the session's system prompt
  context.

## Dependencies

From
[src/turn-orchestrator/iii.worker.yaml](harness/src/turn-orchestrator/iii.worker.yaml):
`session ^0.2.0`, `provider-anthropic ^0.2.0`,
`provider-openai ^0.2.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/turn-orchestrator/main.ts](harness/src/turn-orchestrator/main.ts) | Binary entry point. |
| [src/turn-orchestrator/register.ts](harness/src/turn-orchestrator/register.ts) | Composes all registered functions: `run::start`, per-state `turn::{state}` handlers, `turn::on_approval`, `turn::get_state`. |
| [src/turn-orchestrator/run-start.ts](harness/src/turn-orchestrator/run-start.ts) | `run::start` handler — persists run config and messages, seeds `turn_state` to `provisioning` via `saveRecord` (which wakes the FSM). |
| [src/turn-orchestrator/run-transition.ts](harness/src/turn-orchestrator/run-transition.ts) | Shared FSM transition runner: load → null-check → stale-skip → handle → save. Routes to `failed` on unexpected throw; re-throws `TransientError` for queue retry. |
| [src/turn-orchestrator/state-runtime/store.ts](harness/src/turn-orchestrator/state-runtime/store.ts) | `TurnStore` / `createTurnStore` — agent-scope load/save, `shouldWakeStep`, inline FIFO enqueue from `saveRecord`. |
| [src/turn-orchestrator/run-request.ts](harness/src/turn-orchestrator/run-request.ts) | `RunRequest` type and `parseRunRequest` — the typed, parsed form of scope `run_request` (includes `function_schemas`). |
| [src/turn-orchestrator/get-state.ts](harness/src/turn-orchestrator/get-state.ts) | `turn::get_state` — one-shot reader returning `TurnStateView \| null`. |
| [src/turn-orchestrator/agent-trigger.ts](harness/src/turn-orchestrator/agent-trigger.ts) | Dispatcher chokepoint: `dispatchWithHook` (consult + trigger), `triggerFunctionCall` (trigger/decode/error), `agentTriggerTool` (schema), `unwrapAgentTrigger`. |
| [src/turn-orchestrator/hook.ts](harness/src/turn-orchestrator/hook.ts) | `consultBefore` — `policy::check_permissions` (5 s, fail-closed) → `allow` / `pending` / `deny`. |
| [src/turn-orchestrator/function-awaiting-approval/process.ts](harness/src/turn-orchestrator/function-awaiting-approval/process.ts) | `turn::function_awaiting_approval` FSM step + `turn::on_approval` state trigger on scope `approvals`. |
| [src/turn-orchestrator/schemas.ts](harness/src/turn-orchestrator/schemas.ts) | All registered-function I/O schemas and types: `RunStartPayloadSchema`, `TurnStepPayloadSchema`, `TurnStateView`, `toView`, `ApprovalDecisionEventSchema`. |
| [src/turn-orchestrator/state-runtime/ports.ts](harness/src/turn-orchestrator/state-runtime/ports.ts) | `TurnStatePorts` / `createTurnStatePorts` — shared dependency ports for per-state handlers (incl. `finishSession`). |
| [src/turn-orchestrator/provisioning/process.ts](harness/src/turn-orchestrator/provisioning/process.ts) | `turn::provisioning` handler and provisioning pipeline. |
| [src/turn-orchestrator/assistant-streaming/process.ts](harness/src/turn-orchestrator/assistant-streaming/process.ts) | `turn::assistant_streaming` handler and stream orchestration. |
| [src/turn-orchestrator/function-execute/process.ts](harness/src/turn-orchestrator/function-execute/process.ts) | `turn::function_execute` handler. |
| [src/turn-orchestrator/function-awaiting-approval/process.ts](harness/src/turn-orchestrator/function-awaiting-approval/process.ts) | `turn::function_awaiting_approval` handler. |
| [src/turn-orchestrator/steering-check/process.ts](harness/src/turn-orchestrator/steering-check/process.ts) | `turn::steering_check` handler. |
| [src/turn-orchestrator/state.ts](harness/src/turn-orchestrator/state.ts) | `TurnState`, `TurnStateRecord`, `TurnWork`, `AwaitingApprovalEntry`, state-key helpers, `newRecord`, `transitionTo`. |
| [src/turn-orchestrator/errors.ts](harness/src/turn-orchestrator/errors.ts) | `TransientError` (opt into queue retry), `ContextOverflowError`, `CompactionBusyError`. |
| [src/turn-orchestrator/events.ts](harness/src/turn-orchestrator/events.ts) | `emit(iii, sid, event)` — appends a sequenced `AgentEvent` to the `agent::events` stream. |
| [src/turn-orchestrator/preflight.ts](harness/src/turn-orchestrator/preflight.ts) | `runPreflight` — context-compaction check before each provider call. |
| [src/turn-orchestrator/provider-router.ts](harness/src/turn-orchestrator/provider-router.ts) | `decide` + `targetFunctionId` — pick `provider::<name>::stream` for the run's `provider` field. |
| [src/turn-orchestrator/system-prompt.ts](harness/src/turn-orchestrator/system-prompt.ts) | `buildSystemPrompt` — assembles system prompt from request, bootstrap skills, skills index. |
| [src/turn-orchestrator/bootstrap.ts](harness/src/turn-orchestrator/bootstrap.ts) | Best-effort skill download via `directory::skills::download` at startup. |
| [src/turn-orchestrator/config.ts](harness/src/turn-orchestrator/config.ts) | Loads the worker's config slice. |
| [src/turn-orchestrator/iii.worker.yaml](harness/src/turn-orchestrator/iii.worker.yaml) | Worker manifest. |
