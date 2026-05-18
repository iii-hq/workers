# turn-orchestrator

Durable `run::start` state machine that drives each agent turn through
provisioning, assistant, function-execute, steering, and tearing-down.

## Purpose

This is the heart of the bundle. `run::start` opens a session and returns
immediately; the rest of the work happens inside the durable `turn::step`
state machine, woken once per state transition by a publish to the
`turn::step_requested` topic. The FSM provisions the sandbox, streams the
assistant turn from a provider, executes any returned function calls
through the `agent::call` chokepoint (which the approval gate intercepts),
emits `agent::events` for the harness fanout, and persists everything to
iii state so the run survives restarts.

`agent::call` is the single dispatcher every agent-issued tool call passes
through. It runs `consultBefore` (publishes `agent::before_function_call`
and waits for the approval gate's reply) before forwarding to the target
function id. Fail-closed: a missing/erroring gate denies the call with a
`gate_unavailable` `DenialEnvelope`.

## Registered functions

- `run::start` — Start a durable agent session and return immediately.
- `run::start_and_wait` — Start a durable agent session and block until terminal (test/dev convenience).
- `turn::step` — Run one durable state machine transition for a session.
- `agent::call` — LLM-facing dispatcher: dispatches an iii function and returns a FunctionResult.

## Triggers

- Durable subscriber on `turn::step_requested` → `turn::step`. Registered in [src/turn-orchestrator/subscriber.ts](harness-node/src/turn-orchestrator/subscriber.ts). Each `step` loads the `TurnStateRecord`, runs one transition, saves it back, and re-publishes `turn::step_requested` unless the run is terminal.

## Turn FSM

The full FSM, transitions, and dispatch table lives in
[src/turn-orchestrator/transitions.ts](harness-node/src/turn-orchestrator/transitions.ts).
The 10 states from
[src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts):

| State | Handler | Role |
|---|---|---|
| `provisioning` | [states/provisioning.ts](harness-node/src/turn-orchestrator/states/provisioning.ts) | Boot the sandbox, prime the system prompt, fetch function schemas. |
| `awaiting_assistant` | [states/assistant.ts](harness-node/src/turn-orchestrator/states/assistant.ts) | Request an assistant turn via `provider::<name>::stream`. |
| `assistant_streaming` | same | Drain the channel; relay events. |
| `assistant_finished` | same | Persist the final `AssistantMessage`; pick next state. |
| `function_prepare` | [states/functions.ts](harness-node/src/turn-orchestrator/states/functions.ts) | Snapshot the pending function calls. |
| `function_execute` | same | Run each call via `dispatchWithHook` → `agent::call`. |
| `function_finalize` | same | Persist results; emit `function_call_end` events. |
| `steering_check` | [states/steering.ts](harness-node/src/turn-orchestrator/states/steering.ts) | Decide whether to continue, stop, or hit `max_turns`. |
| `tearing_down` | [states/tearing-down.ts](harness-node/src/turn-orchestrator/states/tearing-down.ts) | Emit `turn_end` once, free the sandbox if any. |
| `stopped` | (no-op) | Terminal. Idempotent. |

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
| `session/<sid>/function_prepared` | Snapshot of pending function calls for the current turn. |
| `session/<sid>/function_executed` | Results of the current turn's function calls. |
| `session/<sid>/tool_prepared`, `session/<sid>/tool_executed` | Legacy aliases of the two above. |

## Configuration

From the top-level `turn-orchestrator` section of
[config.yaml](harness-node/config.yaml):

- `sync_default_timeout_ms` (default `120000`) — `run::start_and_wait` poll
  budget.
- `sync_poll_interval_ms` (default `50`) — poll interval used by
  `run::start_and_wait`.
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
| [src/turn-orchestrator/register.ts](harness-node/src/turn-orchestrator/register.ts) | Composes `run::start*`, `agent::call`, `turn::step` and kicks off the bootstrap. |
| [src/turn-orchestrator/run-start.ts](harness-node/src/turn-orchestrator/run-start.ts) | `run::start` + `run::start_and_wait` handlers and the `publishStep` helper. |
| [src/turn-orchestrator/agent-call.ts](harness-node/src/turn-orchestrator/agent-call.ts) | The dispatcher chokepoint; `dispatchWithHook` runs `consultBefore` before triggering the function. |
| [src/turn-orchestrator/hook.ts](harness-node/src/turn-orchestrator/hook.ts) | `consultBefore` — publishes `agent::before_function_call` and decodes the gate reply. |
| [src/turn-orchestrator/subscriber.ts](harness-node/src/turn-orchestrator/subscriber.ts) | `turn::step` durable subscriber. |
| [src/turn-orchestrator/transitions.ts](harness-node/src/turn-orchestrator/transitions.ts) | State → handler dispatch table. |
| [src/turn-orchestrator/states/*.ts](harness-node/src/turn-orchestrator/states/) | One file per FSM state. |
| [src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts) | `TurnState`, `TurnStateRecord`, state-key helpers. |
| [src/turn-orchestrator/persistence.ts](harness-node/src/turn-orchestrator/persistence.ts) | Load/save helpers + the `session-tree::*` messages mirror. |
| [src/turn-orchestrator/events.ts](harness-node/src/turn-orchestrator/events.ts) | `emit(iii, sid, event)` — appends a sequenced `AgentEvent` to the `agent::events` stream. |
| [src/turn-orchestrator/provider-router.ts](harness-node/src/turn-orchestrator/provider-router.ts) | Picks `provider::<name>::stream` for the run's `provider` field. |
| [src/turn-orchestrator/system-prompt.ts](harness-node/src/turn-orchestrator/system-prompt.ts) | Builds the system prompt from `run_request.system_prompt` + bootstrap skills. |
| [src/turn-orchestrator/bootstrap.ts](harness-node/src/turn-orchestrator/bootstrap.ts) | Best-effort skill download via `directory::skills::download`. |
| [src/turn-orchestrator/config.ts](harness-node/src/turn-orchestrator/config.ts) | Loads the worker's config slice. |
| [src/turn-orchestrator/iii.worker.yaml](harness-node/src/turn-orchestrator/iii.worker.yaml) | Worker manifest. |
