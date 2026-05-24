# turn-orchestrator

Durable `run::start` state machine that drives each agent turn through
provisioning, assistant, function-execute, steering, and tearing-down.

## Purpose

This is the heart of the bundle. `run::start` opens a session and returns
immediately; the rest of the work happens inside per-state durable functions
(`turn::provisioning`, `turn::assistant_streaming`, …), each enqueued onto
the `turn-step` FIFO queue via `wakeState` ([wake.ts](harness/src/turn-orchestrator/wake.ts)).
Saving the record with a new non-terminal, non-parking state automatically
enqueues the next handler (`saveRecord` in
[persistence.ts](harness/src/turn-orchestrator/persistence.ts) calls `shouldWakeStep` then `wakeState`).

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
- `turn::assistant_streaming` — FSM step: stream the turn over a provider channel; on completion emit `message_complete`, persist the assistant message (dup-guarded), route to `function_execute` / `steering_check` / `tearing_down`.
- `turn::function_execute` — FSM step: own the full function lifecycle via `rec.work`; build batch from `rec.last_assistant`, run each call, checkpoint per-call via `writeRecord`, park to `function_awaiting_approval` on a `pending` gate reply, finalize results into messages + emit `turn_end`, route to `steering_check` / `tearing_down`.
- `turn::function_awaiting_approval` — FSM step: read decisions for `awaiting_approval[]`; fold them into `rec.work.batch` (`allow` → `pre_approved`, `deny`/`aborted` → `blocked`); clear `awaiting_approval`, advance to `function_execute`.
- `turn::steering_check` — FSM step: check abort signal, drain `steering`/`followup` inboxes, enforce `max_turns` cap (emits synthetic `max_turns` message + `turn_end` → `tearing_down`), route to `assistant_streaming` / `tearing_down`.
- `turn::tearing_down` — FSM step: emit `agent_end`, advance to `stopped`.
- `turn::get_state` — One-shot reader returning a lean `TurnStateView` (from `schemas.ts:toView`) for a session. UI clients call this on reload to recover in-progress modals (e.g. `function_awaiting_approval`) without reading iii state directly. Returns `null` for unknown sessions.
- `turn::is_abort_signal_set` — Condition function bound to the agent-scope state trigger; matches `state:created`/`state:updated` writes that set `session/<id>/abort_signal` to `true`.
- `turn::on_abort_signal` — State trigger adapter: enqueues `turn::{current_state}` (via `wakeFromRecord`) when the abort signal is set so the FSM observes the abort on the next safe boundary.

## Triggers

- **State trigger** on `scope: agent` gated by `condition_function_id: turn::is_abort_signal_set` → `turn::on_abort_signal`. Registered in [on-abort-signal.ts](harness/src/turn-orchestrator/on-abort-signal.ts). Enqueues the handler for the session's current persisted state the moment `session/<id>/abort_signal` is set to `true`, so the FSM advances to `steering_check` without waiting for the current step to time out.

The record-written wake is now inline in `saveRecord` (no separate `on-record-written` adapter): every `saveRecord` call that transitions to a non-terminal, non-parking state calls `wakeState` directly. Similarly, `turn_state_changed` events are emitted inline from `persistRecord` via `emitTurnStateChanged` ([turn-state-write.ts](harness/src/turn-orchestrator/turn-state-write.ts)) — there is no separate `on-turn-state-changed` state trigger.

Paused turns (`function_awaiting_approval`) are woken when `approval::resolve` or abort triggers each per-call `turn::approval_resume` function (see [approval-resume.ts](harness/src/turn-orchestrator/approval-resume.ts) and [workers/approval-gate.md](workers/approval-gate.md)). `recoverPendingApprovals` re-registers these resume functions at worker startup for sessions that were parked before a restart.

## Turn FSM

Each state is a registered `turn::{state}` function executed via
`runTransition` and enqueued onto the `turn-step` FIFO queue by `wakeState`.
The 8 states from [state.ts](harness/src/turn-orchestrator/state.ts):

| State | Handler file | Role |
|---|---|---|
| `provisioning` | [states/provisioning.ts](harness/src/turn-orchestrator/states/provisioning.ts) | Fetch skills index + default-skill bodies, build system prompt, write enriched `run_request` (with `function_schemas: [agentTriggerTool()]`), → `assistant_streaming`. |
| `assistant_streaming` | [states/assistant-streaming.ts](harness/src/turn-orchestrator/states/assistant-streaming.ts) | Increment `turn_count`; create channel; trigger provider stream; relay `message_update` deltas; on completion call `finalizeAssistant` which emits `message_complete`, persists the assistant message (dup-guarded), then routes → `function_execute` (has calls) / `steering_check` (no calls) / `tearing_down` (error/aborted). |
| `function_execute` | [states/function-execute.ts](harness/src/turn-orchestrator/states/function-execute.ts) | Build batch from `rec.last_assistant` (or reuse existing `rec.work`); for each call: emit `function_execution_start`, skip if already executed, dispatch via `dispatchWithHook`; if `pending` → append to `awaiting_approval`, register `turn::approval_resume`, → `function_awaiting_approval`; otherwise commit result (silent `writeRecord` checkpoint) + emit `function_execution_end`; after batch: fold results into messages + emit `turn_end` → `steering_check` / `tearing_down`. |
| `function_awaiting_approval` | [states/function-awaiting-approval.ts](harness/src/turn-orchestrator/states/function-awaiting-approval.ts) | Read decision for each `awaiting_approval[]` entry; if any is still missing → return (park); when all present, fold into `rec.work.batch` (`allow` → `pre_approved: true`; `deny`/`aborted` → `blocked` with denial result); clear `awaiting_approval` → `function_execute`. |
| `steering_check` | [states/steering-check.ts](harness/src/turn-orchestrator/states/steering-check.ts) | Priority route: abort → `tearing_down`; steering msg → `assistant_streaming` (unless `max_turns` reached); followup msg → `assistant_streaming` (unless `max_turns` reached); function results present → `assistant_streaming` (unless `max_turns` reached); else emit `turn_end` once → `tearing_down`. `max_turns` path emits a synthetic `message_complete` + `turn_end`. |
| `tearing_down` | [states/tearing-down.ts](harness/src/turn-orchestrator/states/tearing-down.ts) | Emit `agent_end` → `stopped`. |
| `stopped` | (no handler) | Terminal. Idempotent. |
| `failed` | (set by `runTransition` on unexpected throw) | Terminal. Carries `error: {kind, message}` on the record. Emits `message_complete{stop_reason:'error'}` + `agent_end` so the UI sees the reason. A handler may throw `TransientError` to use the queue's retry/DLQ instead. |

`NON_STEPABLE_STATES` in [wake.ts](harness/src/turn-orchestrator/wake.ts) are
`stopped`, `failed`, and `function_awaiting_approval` — `saveRecord` does not
enqueue a handler for these.

`dispatchWithHook` returns one of three shapes: `{ kind: 'result' }`,
`{ kind: 'deny' }`, or `{ kind: 'pending' }`. `pending` triggers the
`function_awaiting_approval` park.

## State keys

All keys live under iii state scope `agent`. Key helpers are defined in
[state.ts](harness/src/turn-orchestrator/state.ts); persistence helpers in
[persistence.ts](harness/src/turn-orchestrator/persistence.ts).

| Key shape | Purpose |
|---|---|
| `session/<sid>/turn_state` | Serialised `TurnStateRecord` (incl. `work?: TurnWork` and `error?: {kind, message}`). |
| `session/<sid>/messages` | Active path `AgentMessage[]`; mirrored into `session-tree::*` on every save (inline in `persistence.saveMessages`). |
| `session/<sid>/run_request` | The `run::start` payload enriched by `provisioning` to include `function_schemas: [agentTriggerTool()]` and the assembled `system_prompt`. Typed as `RunRequest` ([run-request.ts](harness/src/turn-orchestrator/run-request.ts)). |
| `session/<sid>/session_tree_mirror_len` | High-water mark so the session-tree messages mirror is incremental. The session-tree mirror is still inline in `persistence.saveMessages` — its relocation to a reactive subscriber is tracked as a follow-up, not done. |
| `session/<sid>/event_counter` | Monotonic counter for `agent::events` sequence numbers. |
| `session/<sid>/abort_signal` | Set by `router::abort` via `performAbortSideEffects` to interrupt a streaming turn. |

Keys that no longer exist: `function_prepared`, `function_executed`,
`function_schemas` (standalone), `tool_prepared`, `tool_executed`,
`tool_schemas`, `sandbox_id`, `last_compaction_at`,
`last_compaction_consumed_at` — these were removed in the rewrite.

The `TurnStateRecord` carries `work?: TurnWork` (inline `{batch: PreparedEntry[]; results: ExecutedEntry[]}`) in place of the former separate state keys. `PreparedEntry`, `ExecutedEntry`, and `TurnWork` are all defined in [state.ts](harness/src/turn-orchestrator/state.ts).

## UI events

`turn_state_changed` is emitted inline by `persistRecord` (via
[turn-state-write.ts](harness/src/turn-orchestrator/turn-state-write.ts))
on every `saveRecord` / `persistRecord` call. It carries a lean
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
session to `function_awaiting_approval` and registers a per-call
`turn::approval_resume` function. `approval::resolve` (or abort via
`performAbortSideEffects`) triggers that resume function, which persists the
decision to scope `approvals` and calls `wakeFromRecord` to re-enqueue the
session's current state handler.

## Configuration

From the top-level `turn-orchestrator` section of
[config.yaml](harness/config.yaml):

- `system_default_skills` (default `["iii://iii-directory/index"]`) —
  skill URIs the bootstrap step downloads into the session's system prompt
  context.

## Dependencies

From
[src/turn-orchestrator/iii.worker.yaml](harness/src/turn-orchestrator/iii.worker.yaml):
`session ^0.2.0`, `hook-fanout ^0.2.0`, `provider-anthropic ^0.2.0`,
`provider-openai ^0.2.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/turn-orchestrator/main.ts](harness/src/turn-orchestrator/main.ts) | Binary entry point. |
| [src/turn-orchestrator/register.ts](harness/src/turn-orchestrator/register.ts) | Composes all registered functions: `run::start`, per-state `turn::{state}` handlers, abort-signal trigger, approval-resume recovery, `turn::get_state`. |
| [src/turn-orchestrator/run-start.ts](harness/src/turn-orchestrator/run-start.ts) | `run::start` handler — persists run config and messages, seeds `turn_state` to `provisioning` via `saveRecord` (which wakes the FSM). |
| [src/turn-orchestrator/run-transition.ts](harness/src/turn-orchestrator/run-transition.ts) | Shared FSM transition runner: load → null-check → stale-skip → handle → save. Routes to `failed` on unexpected throw; re-throws `TransientError` for queue retry. |
| [src/turn-orchestrator/wake.ts](harness/src/turn-orchestrator/wake.ts) | `wakeState` / `wakeFromRecord` — enqueue `turn::{state}` onto the `turn-step` FIFO queue; `shouldWakeStep` gates non-stepable states. |
| [src/turn-orchestrator/schemas.ts](harness/src/turn-orchestrator/schemas.ts) | All registered-function I/O schemas and types: `RunStartPayloadSchema`, `TurnStepPayloadSchema`, `TurnStateView`, `toView`, `AbortSignalWriteEventSchema`. |
| [src/turn-orchestrator/run-request.ts](harness/src/turn-orchestrator/run-request.ts) | `RunRequest` type and `parseRunRequest` — the typed, parsed form of `session/<sid>/run_request` (includes `function_schemas`). |
| [src/turn-orchestrator/get-state.ts](harness/src/turn-orchestrator/get-state.ts) | `turn::get_state` — one-shot reader returning `TurnStateView \| null`. |
| [src/turn-orchestrator/agent-trigger.ts](harness/src/turn-orchestrator/agent-trigger.ts) | Dispatcher chokepoint: `dispatchWithHook` (consult + trigger), `triggerFunctionCall` (trigger/decode/error), `agentTriggerTool` (schema), `unwrapAgentTrigger`. |
| [src/turn-orchestrator/hook.ts](harness/src/turn-orchestrator/hook.ts) | `consultBefore` — `policy::check_permissions` (5 s, fail-closed) → `allow` / `pending` / `deny`. `publishAfter` — `hook-fanout::publish_collect` for after-hook fanout. |
| [src/turn-orchestrator/approval-resume.ts](harness/src/turn-orchestrator/approval-resume.ts) | Per-call `turn::approval_resume` registration and handler (persist decision + `wakeFromRecord`); `recoverPendingApprovals` re-registers at startup. |
| [src/turn-orchestrator/abort.ts](harness/src/turn-orchestrator/abort.ts) | `performAbortSideEffects` — writes `session/<sid>/abort_signal = true` and triggers each `turn::approval_resume` with `{decision: 'aborted'}` for parked sessions. |
| [src/turn-orchestrator/on-abort-signal.ts](harness/src/turn-orchestrator/on-abort-signal.ts) | State trigger adapter — `turn::is_abort_signal_set` (condition) + `turn::on_abort_signal` (handler, calls `wakeFromRecord`). |
| [src/turn-orchestrator/turn-state-write.ts](harness/src/turn-orchestrator/turn-state-write.ts) | `emitTurnStateChanged` — inline UI notification emitting `turn_state_changed` with lean `TurnStateView`. Called from `persistRecord`. |
| [src/turn-orchestrator/states/provisioning.ts](harness/src/turn-orchestrator/states/provisioning.ts) | `turn::provisioning` handler. |
| [src/turn-orchestrator/states/assistant-streaming.ts](harness/src/turn-orchestrator/states/assistant-streaming.ts) | `turn::assistant_streaming` handler. |
| [src/turn-orchestrator/states/function-execute.ts](harness/src/turn-orchestrator/states/function-execute.ts) | `turn::function_execute` handler. |
| [src/turn-orchestrator/states/function-awaiting-approval.ts](harness/src/turn-orchestrator/states/function-awaiting-approval.ts) | `turn::function_awaiting_approval` handler. |
| [src/turn-orchestrator/states/steering-check.ts](harness/src/turn-orchestrator/states/steering-check.ts) | `turn::steering_check` handler. |
| [src/turn-orchestrator/states/tearing-down.ts](harness/src/turn-orchestrator/states/tearing-down.ts) | `turn::tearing_down` handler. |
| [src/turn-orchestrator/states/index.ts](harness/src/turn-orchestrator/states/index.ts) | Re-exports per-state `register` functions. |
| [src/turn-orchestrator/state.ts](harness/src/turn-orchestrator/state.ts) | `TurnState`, `TurnStateRecord`, `TurnWork`, `PreparedEntry`, `ExecutedEntry`, `AwaitingApprovalEntry`, state-key helpers, `newRecord`, `transitionTo`, `isTerminal`. |
| [src/turn-orchestrator/persistence.ts](harness/src/turn-orchestrator/persistence.ts) | Load/save helpers: `loadRecord` (with legacy `assistant_finished` migration), `saveRecord` (persist + wake), `persistRecord` (persist + UI event, no wake), `writeRecord` (silent checkpoint), `saveMessages` (+ session-tree mirror). |
| [src/turn-orchestrator/errors.ts](harness/src/turn-orchestrator/errors.ts) | `TransientError` (opt into queue retry), `ContextOverflowError`, `CompactionBusyError`. |
| [src/turn-orchestrator/events.ts](harness/src/turn-orchestrator/events.ts) | `emit(iii, sid, event)` — appends a sequenced `AgentEvent` to the `agent::events` stream. |
| [src/turn-orchestrator/preflight.ts](harness/src/turn-orchestrator/preflight.ts) | `runPreflight` — context-compaction check before each provider call. |
| [src/turn-orchestrator/provider-router.ts](harness/src/turn-orchestrator/provider-router.ts) | `decide` + `targetFunctionId` — pick `provider::<name>::stream` for the run's `provider` field. |
| [src/turn-orchestrator/system-prompt.ts](harness/src/turn-orchestrator/system-prompt.ts) | `buildSystemPrompt` — assembles system prompt from request, bootstrap skills, skills index. |
| [src/turn-orchestrator/bootstrap.ts](harness/src/turn-orchestrator/bootstrap.ts) | Best-effort skill download via `directory::skills::download` at startup. |
| [src/turn-orchestrator/config.ts](harness/src/turn-orchestrator/config.ts) | Loads the worker's config slice. |
| [src/turn-orchestrator/iii.worker.yaml](harness/src/turn-orchestrator/iii.worker.yaml) | Worker manifest. |
