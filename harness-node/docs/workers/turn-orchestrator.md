# turn-orchestrator

Durable `run::start` state machine that drives each agent turn through
provisioning, assistant, function-execute, steering, and tearing-down.

## Purpose

This is the heart of the bundle. `run::start` opens a session and returns
immediately; the rest of the work happens inside the durable `turn::step`
state machine, woken once per state transition by a publish to the
`turn::step_requested` topic. The FSM provisions the sandbox, streams the
assistant turn from a provider, executes any returned function calls
through the `agent::call` chokepoint, emits `agent::events` for the
harness fanout, and persists everything to iii state so the run survives
restarts.

`agent::call` is the single dispatcher every agent-issued tool call passes
through. It runs `consultBefore` before forwarding to the target function
id. `consultBefore` triggers `policy::check_permissions` directly (5 s
timeout) and maps the reply to allow / deny / pending. Fail-closed: policy
unreachable → deny with a `gate_unavailable` `DenialEnvelope`, unless the
function id appears in the per-run `approval_required` list (legacy
fallback → pending, so the UI can still resolve it).

## Registered functions

- `run::start` — Start a durable agent session and return immediately.
- `run::start_and_wait` — Start a durable agent session and block until terminal (test/dev convenience).
- `turn::step` — Run one durable state machine transition for a session.
- `agent::call` — LLM-facing dispatcher: dispatches an iii function and returns a FunctionResult.
- `turn::is_abort_signal_set` — Condition function bound to the agent-scope state trigger; matches `state:created`/`state:updated` writes that set `session/<id>/abort_signal` to `true`.
- `turn::on_abort_signal` — State trigger adapter: publishes `turn::step_requested` when the abort signal is set so the FSM advances on the next safe boundary.
- `turn::is_terminal_state_write` — Condition function bound to the terminal state trigger; matches writes to `session/<id>/turn_state` whose `new_value.state === 'stopped'`.
- `turn::on_terminal_state` — State trigger adapter: resolves the in-process waiter installed by `run::start_and_wait` for that session id.
- `turn::is_stepable_record_write` — Condition function bound to the record-written state trigger; matches `turn_state` writes whose `new_value.state` is non-terminal and non-parking (i.e. excludes `stopped` and `function_awaiting_approval`).
- `turn::on_record_written` — State trigger adapter: directly triggers `turn::step` for the affected session, so saving the record is itself the wake-up event.
- `turn::is_turn_state_write` — Condition function bound to the turn-state-changed trigger; matches every `state:created` / `state:updated` write to `session/<sid>/turn_state` regardless of FSM state.
- `turn::on_turn_state_changed` — State trigger adapter: emits a `turn_state_changed` agent event carrying the full new (and prior) `TurnStateRecord` so the UI can derive pending approvals from state.

## Triggers

- **Durable subscriber** on `turn::step_requested` → `turn::step`. Registered in [src/turn-orchestrator/subscriber.ts](harness-node/src/turn-orchestrator/subscriber.ts). Each `step` loads the `TurnStateRecord`, runs one transition, saves it back, and re-publishes `turn::step_requested` unless the run is terminal **or** paused on approvals (`function_awaiting_approval`). The approval-gate worker wakes those paused turns through its own state trigger on the `approvals` scope.
- **State trigger** on `scope: agent` gated by `condition_function_id: turn::is_abort_signal_set` → `turn::on_abort_signal`. Registered in [src/turn-orchestrator/on-abort-signal.ts](harness-node/src/turn-orchestrator/on-abort-signal.ts). Publishes `turn::step_requested` the moment `session/<id>/abort_signal` is set to `true`, so the FSM advances to `steering_check` (and observes the abort) on the next safe boundary without waiting for the current step to time out.
- **State trigger** on `scope: agent` gated by `condition_function_id: turn::is_terminal_state_write` → `turn::on_terminal_state`. Registered in [src/turn-orchestrator/on-terminal.ts](harness-node/src/turn-orchestrator/on-terminal.ts). Fires on the `session/<id>/turn_state` write that lands `stopped`; the handler resolves the per-session waiter installed by `run::start_and_wait` so the sync wrapper returns without polling.
- **State trigger** on `scope: agent` gated by `condition_function_id: turn::is_stepable_record_write` → `turn::on_record_written`. Registered in [src/turn-orchestrator/on-record-written.ts](harness-node/src/turn-orchestrator/on-record-written.ts). Directly triggers `turn::step` for the affected session on every non-terminal, non-parking `session/<sid>/turn_state` write. Replaces the imperative `publishStep` self-publish — saving the record is now the wake.
- **State trigger** on `scope: agent` gated by `condition_function_id: turn::is_turn_state_write` → `turn::on_turn_state_changed`. Registered in [src/turn-orchestrator/on-turn-state-changed.ts](harness-node/src/turn-orchestrator/on-turn-state-changed.ts). Fires on every `session/<sid>/turn_state` write (created or updated) and emits a `turn_state_changed` event to `agent::events` carrying the full new (and prior) record so the UI can derive pending approvals from state rather than from a signal event.

## Turn FSM

The full FSM, transitions, and dispatch table lives in
[src/turn-orchestrator/transitions.ts](harness-node/src/turn-orchestrator/transitions.ts).
The 11 states from
[src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts):

| State | Handler | Role |
|---|---|---|
| `provisioning` | [states/provisioning.ts](harness-node/src/turn-orchestrator/states/provisioning.ts) | Boot the sandbox, prime the system prompt, fetch function schemas. |
| `awaiting_assistant` | [states/assistant.ts](harness-node/src/turn-orchestrator/states/assistant.ts) | Request an assistant turn via `provider::<name>::stream`. |
| `assistant_streaming` | same | Drain the channel; relay events. |
| `assistant_finished` | same | Persist the final `AssistantMessage`; pick next state. |
| `function_prepare` | [states/functions.ts](harness-node/src/turn-orchestrator/states/functions.ts) | Snapshot the pending function calls. |
| `function_execute` | same | Run each call via `dispatchWithHook` → `agent::call`. If the gate returns `pending`, append the call to `awaiting_approval` and transition to `function_awaiting_approval` (the rest of the batch is left for the resumed step). |
| `function_awaiting_approval` | same (`handleAwaitingApproval`) | Read `approvals/<sid>/<cid>` for every entry in `awaiting_approval`. While any decision is still missing, return without stepping (the approval-gate's state trigger will wake us). When all decisions are present, fold them into the prepared snapshot — `allow` → `pre_approved: true`, `deny`/`aborted` → `blocked` with a denial result — clear `awaiting_approval`, and transition back to `function_execute`. |
| `function_finalize` | same | Persist results; emit `function_call_end` + `turn_end` events. |
| `steering_check` | [states/steering.ts](harness-node/src/turn-orchestrator/states/steering.ts) | Decide whether to continue, stop, or hit `max_turns`. |
| `tearing_down` | [states/tearing-down.ts](harness-node/src/turn-orchestrator/states/tearing-down.ts) | Emit `agent_end` once, free the sandbox if any. |
| `stopped` | (no-op) | Terminal. Idempotent. |

`dispatchWithHook` in [agent-call.ts](harness-node/src/turn-orchestrator/agent-call.ts)
now returns one of three shapes: `{ kind: 'result' }`, `{ kind: 'deny' }`,
or `{ kind: 'pending' }`. Pending is what triggers the
`function_awaiting_approval` park.

## State keys

All keys live under iii state scope `agent`. From
[src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts):

| Key shape | Purpose |
|---|---|
| `session/<sid>/turn_state` | Serialised `TurnStateRecord`. |
| `session/<sid>/messages` | Active path `AgentMessage[]`; mirrored into `session-tree::*` on every save. |
| `session/<sid>/run_request` | The original `run::start` payload (provider, model, system_prompt, mode, image, idle_timeout_secs, cwd, cwd_hash). |
| `session/<sid>/cwd` | Working directory for the sandbox. |
| `harness/cwd/<hash>/last_session_id` | Reverse index from `cwd_hash` to the last session that ran there. |
| `session/<sid>/sandbox_id` | Active sandbox handle. |
| `session/<sid>/function_schemas` | Cached tool schemas exposed to the model. |
| `session/<sid>/tool_schemas` | Legacy alias of `function_schemas`. |
| `session/<sid>/session_tree_mirror_len` | High-water mark so the messages mirror is incremental. |
| `session/<sid>/last_compaction_at` | Last entry id the compactor wrote. |
| `session/<sid>/last_compaction_consumed_at` | Last compaction the loader applied. |
| `session/<sid>/event_counter` | Monotonic counter for `agent::events` sequence numbers. |
| `session/<sid>/abort_signal` | Set by `router::abort` to interrupt a streaming turn. |
| `session/<sid>/function_prepared` | Snapshot of pending function calls for the current turn. Each entry carries `pre_approved` / `blocked` flags so resumed approvals can short-circuit re-dispatch. |
| `session/<sid>/function_executed` | Results of the current turn's function calls. |
| `session/<sid>/tool_prepared`, `session/<sid>/tool_executed` | Legacy aliases of the two above. |

The `TurnStateRecord` also carries an optional `awaiting_approval:
AwaitingApprovalEntry[]` field — populated when `function_execute` is
parked, drained when `function_awaiting_approval` folds the resolved
decisions back into the prepared snapshot.

## Configuration

From the top-level `turn-orchestrator` section of
[config.yaml](harness-node/config.yaml):

- `sync_default_timeout_ms` (default `120000`) — wall-clock cap on a
  `run::start_and_wait` call; if the terminal state trigger doesn't
  resolve the waiter within this many ms, the wrapper throws.
- `system_default_skills` (default `["iii://iii-directory/index"]`) —
  skills the bootstrap step downloads into the session's system prompt
  context.

## Dependencies

From
[src/turn-orchestrator/iii.worker.yaml](harness-node/src/turn-orchestrator/iii.worker.yaml):
`session ^0.2.0`, `hook-fanout ^0.2.0`, `provider-anthropic ^0.2.0`,
`provider-openai ^0.2.0`.

## Source layout

| File | Purpose |
|---|---|
| [src/turn-orchestrator/main.ts](harness-node/src/turn-orchestrator/main.ts) | Binary entry point. |
| [src/turn-orchestrator/register.ts](harness-node/src/turn-orchestrator/register.ts) | Composes `run::start*`, `agent::call`, `turn::step`, the abort-signal and terminal-state state triggers, and kicks off the bootstrap. |
| [src/turn-orchestrator/run-start.ts](harness-node/src/turn-orchestrator/run-start.ts) | `run::start` + `run::start_and_wait` handlers and the `publishStep` helper. `executeSync` installs a terminal-state waiter, kicks the run, then races the waiter against `sync_default_timeout_ms` — no polling. |
| [src/turn-orchestrator/on-terminal.ts](harness-node/src/turn-orchestrator/on-terminal.ts) | State trigger adapter — `turn::is_terminal_state_write` (condition) + `turn::on_terminal_state` (handler) — plus the in-process `installTerminalWaiter` / `clearTerminalWaiter` API used by `executeSync` to await a terminal `turn_state` write reactively. |
| [src/turn-orchestrator/agent-call.ts](harness-node/src/turn-orchestrator/agent-call.ts) | The dispatcher chokepoint; `dispatchWithHook` runs `consultBefore` before triggering the function and returns `result` / `deny` / `pending`. |
| [src/turn-orchestrator/hook.ts](harness-node/src/turn-orchestrator/hook.ts) | `consultBefore` — calls `policy::check_permissions` directly (5 s timeout) and maps the reply to `allow` / `pending` / `deny`; fails closed with a `gate_unavailable` envelope. `publishAfter` still routes through `hook-fanout::publish_collect` for the after-hook fanout path. |
| [src/turn-orchestrator/abort.ts](harness-node/src/turn-orchestrator/abort.ts) | `performAbortSideEffects` — writes `session/<sid>/abort_signal = true` and, for turns paused on approvals, one `{decision: 'aborted'}` record per pending call so the approvals state trigger wakes the turn. |
| [src/turn-orchestrator/on-abort-signal.ts](harness-node/src/turn-orchestrator/on-abort-signal.ts) | State trigger adapter — `turn::is_abort_signal_set` (condition) + `turn::on_abort_signal` (handler) — publishes `turn::step_requested` whenever `session/<id>/abort_signal` is set to `true`. |
| [src/turn-orchestrator/subscriber.ts](harness-node/src/turn-orchestrator/subscriber.ts) | `turn::step` durable subscriber. Skips the auto re-publish of `turn::step_requested` while the record is in `function_awaiting_approval` (the approvals state trigger owns that wake). |
| [src/turn-orchestrator/transitions.ts](harness-node/src/turn-orchestrator/transitions.ts) | State → handler dispatch table. |
| [src/turn-orchestrator/states/*.ts](harness-node/src/turn-orchestrator/states/) | One file per FSM state; `states/functions.ts` owns `function_prepare`, `function_execute`, `function_awaiting_approval`, and `function_finalize`. |
| [src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts) | `TurnState`, `TurnStateRecord` (now with `awaiting_approval?: AwaitingApprovalEntry[]`), state-key helpers. |
| [src/turn-orchestrator/persistence.ts](harness-node/src/turn-orchestrator/persistence.ts) | Load/save helpers + the `session-tree::*` messages mirror. `PreparedEntry` now carries `pre_approved` so resumed turns can dispatch the call without re-asking the gate. |
| [src/turn-orchestrator/events.ts](harness-node/src/turn-orchestrator/events.ts) | `emit(iii, sid, event)` — appends a sequenced `AgentEvent` to the `agent::events` stream. |
| [src/turn-orchestrator/on-record-written.ts](harness-node/src/turn-orchestrator/on-record-written.ts) | State-trigger adapter — `turn::is_stepable_record_write` (condition) + `turn::on_record_written` (handler) — directly triggers `turn::step` on every non-terminal, non-parking `turn_state` write. Replaces the imperative `publishStep` self-publish so saving the record is itself the wake. |
| [src/turn-orchestrator/on-turn-state-changed.ts](harness-node/src/turn-orchestrator/on-turn-state-changed.ts) | State-trigger adapter — `turn::is_turn_state_write` (condition) + `turn::on_turn_state_changed` (handler) — emits `turn_state_changed` to `agent::events` on every `turn_state` write (created or updated). Carries the full new (and prior) `TurnStateRecord` so the console can derive pending approvals from state rather than from a signal event. |
| [src/turn-orchestrator/provider-router.ts](harness-node/src/turn-orchestrator/provider-router.ts) | Picks `provider::<name>::stream` for the run's `provider` field. |
| [src/turn-orchestrator/system-prompt.ts](harness-node/src/turn-orchestrator/system-prompt.ts) | Builds the system prompt from `run_request.system_prompt` + bootstrap skills. |
| [src/turn-orchestrator/bootstrap.ts](harness-node/src/turn-orchestrator/bootstrap.ts) | Best-effort skill download via `directory::skills::download`. |
| [src/turn-orchestrator/config.ts](harness-node/src/turn-orchestrator/config.ts) | Loads the worker's config slice. |
| [src/turn-orchestrator/iii.worker.yaml](harness-node/src/turn-orchestrator/iii.worker.yaml) | Worker manifest. |
