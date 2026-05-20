# harness-node architecture

`harness-node` is the Node/TypeScript port of the iii harness stack. It ships
as one pnpm package containing 11 workers (one folder per worker, one feature
per file) plus a shared `runtime/` SDK helper layer and a `types/` wire-type
mirror of `harness/crates/harness-types`. Each worker is independently runnable
as `pnpm dev:<worker>` (development) or `iii-<worker>` (production binary);
[src/index.ts](harness-node/src/index.ts) is the composite entry-point that
spins every worker up in a single process by reusing each worker's
`register()` callback unchanged.

The Rust workers `shell`, `iii-directory`, and the engine's `state::*` /
`stream::*` / `iii::durable::*` primitives are NOT ported. `harness-node`
talks to them over the iii bus exactly the same way it talks to its own
workers.

## Worker catalogue

| Worker | Folder | Role | Doc |
|---|---|---|---|
| harness | [src/harness/](harness-node/src/harness/) | Meta-worker; loads `iii-permissions.yaml`, exposes `harness::trigger` (WS ingestion bridge — see [Telemetry & trace correlation](#telemetry--trace-correlation)) / `policy::check_permissions` / `ui::*`, spins up `agent::events` fan-out. | [workers/harness.md](harness-node/docs/workers/harness.md) |
| turn-orchestrator | [src/turn-orchestrator/](harness-node/src/turn-orchestrator/) | Durable FSM driving each agent turn; chokepoint dispatcher for `agent::trigger`. | [workers/turn-orchestrator.md](harness-node/docs/workers/turn-orchestrator.md) |
| approval-gate | [src/approval-gate/](harness-node/src/approval-gate/) | Registers `approval::resolve` and shared approval wire schemas; routes decisions to per-call `turn::approval_resume` fns owned by the turn-orchestrator. | [workers/approval-gate.md](harness-node/docs/workers/approval-gate.md) |
| session | [src/session/](harness-node/src/session/) | Branching session storage (`session-tree::*`) plus per-session inbox queues (`session-inbox::*`). | [workers/session.md](harness-node/docs/workers/session.md) |
| llm-budget | [src/llm-budget/](harness-node/src/llm-budget/) | Workspace + agent LLM spend caps with alerts, forecast, period rollover. | [workers/llm-budget.md](harness-node/docs/workers/llm-budget.md) |
| hook-fanout | [src/hook-fanout/](harness-node/src/hook-fanout/) | Generic publish-and-collect primitive over a stream topic. | [workers/hook-fanout.md](harness-node/docs/workers/hook-fanout.md) |
| auth-credentials | [src/auth-credentials/](harness-node/src/auth-credentials/) | File-backed multi-provider credential store. | [workers/auth-credentials.md](harness-node/docs/workers/auth-credentials.md) |
| models-catalog | [src/models-catalog/](harness-node/src/models-catalog/) | Static model-capability catalogue (state-first, embedded fallback). | [workers/models-catalog.md](harness-node/docs/workers/models-catalog.md) |
| provider-anthropic | [src/provider-anthropic/](harness-node/src/provider-anthropic/) | Anthropic Messages API SSE → channel writer. | [workers/provider-anthropic.md](harness-node/docs/workers/provider-anthropic.md) |
| provider-openai | [src/provider-openai/](harness-node/src/provider-openai/) | OpenAI Chat Completions SSE → channel writer. | [workers/provider-openai.md](harness-node/docs/workers/provider-openai.md) |
| context-compaction | [src/context-compaction/](harness-node/src/context-compaction/) | Optional `agent::events` side-car that compacts session history when running token count crosses a threshold. | [workers/context-compaction.md](harness-node/docs/workers/context-compaction.md) |

## System diagram

```mermaid
flowchart LR
  client[Browser or CLI client]

  subgraph harnessNode [harness-node workers]
    harness[harness]
    turnOrch[turn-orchestrator]
    approval[approval-gate]
    session[session]
    budget[llm-budget]
    hook[hook-fanout]
    auth[auth-credentials]
    models[models-catalog]
    provAnth[provider-anthropic]
    provOAI[provider-openai]
    compact[context-compaction]
  end

  subgraph external [External Rust workers + engine]
    shell[shell]
    directory[iii-directory]
    state["iii engine state::* / stream::* / iii::durable::*"]
  end

  client -- "harness::trigger(run::start, ...)" --> harness
  harness -- "iii.trigger run::start" --> turnOrch
  client -- "ui::subscribe" --> harness

  turnOrch -- "provider::*::stream" --> provAnth
  turnOrch -- "provider::*::stream" --> provOAI
  turnOrch -- "consultBefore: policy::check_permissions" --> harness
  turnOrch -- "agent::trigger → hook-fanout::publish_collect (after-hook)" --> hook
  turnOrch -- "session-tree::* mirror" --> session
  turnOrch -- "state::* persistence" --> state

  client -- "approval::resolve" --> approval
  approval -- "trigger turn::approval_resume::<sid>/<cid>" --> turnOrch
  turnOrch -- "state::set approvals/<sid>/<cid>" --> state
  turnOrch -- "iii.trigger turn::step" --> turnOrch

  provAnth -- "auth::get_token" --> auth
  provOAI -- "auth::get_token" --> auth

  state -- "agent::events stream" --> harness
  state -- "agent::events stream" --> compact
  state -- "state trigger (scope=agent, abort_signal)" --> turnOrch
  state -- "state trigger (scope=agent, turn_state created)" --> harness
  harness -- "ui::session::event::<browser_id>" --> client
  compact -- "session-tree::compact" --> session
```

## Turn FSM

[src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts)
defines an 11-state durable FSM. Every transition is driven by the
`turn::step` durable subscriber, which is woken by a publish to the
`turn::step_requested` topic — either by the orchestrator itself
(re-publish at the end of a step), by a per-call
`turn::approval_resume` handler (when a human decision or abort lands), or by
the orchestrator's own `abort_signal` state trigger.

```mermaid
stateDiagram-v2
  [*] --> provisioning
  provisioning --> awaiting_assistant
  awaiting_assistant --> assistant_streaming
  assistant_streaming --> assistant_finished
  assistant_finished --> function_prepare: has function calls
  assistant_finished --> steering_check: no function calls
  function_prepare --> function_execute
  function_execute --> function_finalize: all calls resolved (allow/deny)
  function_execute --> function_awaiting_approval: any call needs_approval
  function_awaiting_approval --> function_awaiting_approval: decision(s) still missing
  function_awaiting_approval --> function_execute: all decisions written
  function_finalize --> steering_check
  steering_check --> awaiting_assistant: continue
  steering_check --> tearing_down: stop or max turns
  tearing_down --> stopped
  stopped --> [*]
```

## Approval flow

The orchestrator consults `policy::check_permissions` directly inside
`consultBefore` — `allow`, `deny`, or `pending`. There is no hook fanout on
the before path. The orchestrator parks the turn in `function_awaiting_approval`,
registers a `turn::approval_resume` function per pending call, and waits until
`approval::resolve` (or abort) triggers that function, which persists the
decision and invokes `turn::step`.

```mermaid
sequenceDiagram
  participant Turn as turn-orchestrator (FSM)
  participant Bus as iii bus (state::* + stream::*)
  participant Harness as harness (policy::check_permissions)
  participant Gate as approval-gate
  participant User

  Turn->>Harness: policy::check_permissions(function_id, args) [5s timeout]
  alt rule.action == allow
    Harness-->>Turn: allow → dispatch the call
  else rule.action == deny
    Harness-->>Turn: deny + DenialEnvelope → DenialResult
  else no rule (needs_approval)
    Harness-->>Turn: needs_approval → park in function_awaiting_approval
    Note over Turn,Bus: Orchestrator stops re-publishing turn::step_requested.<br/>The TurnStateRecord.awaiting_approval list pins the open calls.
    User->>Gate: approval::resolve(decision, reason)
    Gate->>Turn: trigger turn::approval_resume::<sid>/<cid>
    Turn->>Bus: state::set approvals/<sid>/<cid> = {decision, reason}
    Turn->>Turn: turn::step → function_awaiting_approval reads<br/>approvals/<sid>/<cid> for each pending entry
    Turn->>Turn: fold decisions into prepared snapshot,<br/>transition back to function_execute
  end
```

Fail-closed: policy unreachable (transport error or 5 s timeout) →
`consultBefore` denies the call with a `gate_unavailable` envelope.

Abort: `router::abort` writes `session/<sid>/abort_signal = true` (waking
the orchestrator through its own `agent`-scope state trigger) and, if the
turn is paused on approvals, triggers each registered
`turn::approval_resume` function with `{decision: 'aborted'}`.

## Kernel deny list

[iii-permissions.yaml](iii-permissions.yaml) at the workspace root is the
single source of truth for what an in-run agent can call. The harness loads
it at boot and watches for changes via `chokidar`. Rules are scanned
top-to-bottom; first match wins. Kernel rules below are shipped by default
and protect the gate / state surface.

Deny shorthands (`!function_id` in the YAML): `approval::resolve`,
`policy::check_permissions`, `hook-fanout::publish_collect`, `state::set`,
`state::update`, `state::delete`, `stream::set`, `iii::durable::publish`,
`auth::set_token`, `auth::delete_token`, `oauth::anthropic::login`,
`oauth::openai-codex::login`, `run::start`, `run::start_and_wait`,
`router::stream_assistant`, `router::abort`.

Bare-string allow rules: `state::get`, `state::list`,
`models::list`, `models::get`, `models::supports`, `auth::get_token`,
`auth::list_providers`, `auth::status`, `oauth::anthropic::status`,
`oauth::openai-codex::status`, the `directory::engine::*` introspection
surface, and the `directory::skills::*` and `directory::prompts::*`
lookups.

## Shared modules

| Folder | Purpose |
|---|---|
| [src/runtime/](harness-node/src/runtime/) | Cross-worker SDK helpers. `worker.ts` parses CLI flags, bootstraps an SDK connection, and wraps it in a `Proxy` so every `registerFunction` is auto-instrumented (see [Telemetry & trace correlation](#telemetry--trace-correlation)); `config.ts` loads `config.yaml`; `state.ts` / `stream.ts` wrap the iii engine's state/stream surface; `ids.ts` mints UUID-like ids; `otel.ts` wires `iii-sdk` OTel via `initHarnessOtel`, exposes `instrumentHandler` (per-call span + baggage propagation), and bridges every pino log to an OTel log auto-correlated to the active span; `handler.ts` exposes `unwrapBody` / `requireString`. |
| [src/types/](harness-node/src/types/) | Wire types that mirror `harness/crates/harness-types/src/*.rs`. `agent-event.ts`, `agent-message.ts`, `content.ts`, `function.ts`, `stream-event.ts`, `thinking.ts`, `provider.ts`, plus `wire.ts` for envelope helpers. |

## Boot ordering & dependencies

The composite entry-point [src/index.ts](harness-node/src/index.ts) spins
workers up in this order so each dependency is already on the bus before its
dependants register:

```mermaid
flowchart TD
  harness --> turnOrch
  harness --> approval
  models[models-catalog] --> turnOrch
  session --> turnOrch
  auth[auth-credentials] --> provAnth[provider-anthropic]
  auth --> provOAI[provider-openai]
  provAnth --> turnOrch
  provOAI --> turnOrch
  hook[hook-fanout] --> approval
  session --> compact[context-compaction]
  provAnth --> compact
  provOAI --> compact
  budget[llm-budget]
```

Edges are extracted from each worker's `iii.worker.yaml` `dependencies:`
block.

## Telemetry & trace correlation

Every harness-node function is automatically instrumented with an OTel span
tagged with `iii.session.id` / `iii.message.id` / `iii.function.id`. This is
what the engine's `engine::traces::group_by` reads to populate "Group by
Session" / "Group by Message" / "Group by Function" in the traces UI; without
the IDs on every span, those groupings return empty.

**The single chokepoint.** [src/runtime/worker.ts](harness-node/src/runtime/worker.ts)
calls `initHarnessOtel(serviceName, engineWsUrl)` before opening the SDK,
then wraps the `ISdk` in a `Proxy` that intercepts `registerFunction(id,
handler, opts)`. The local function handler is replaced with
`instrumentHandler(id, handler)` before being passed through to the real
SDK. HTTP invocation configs (objects, not functions) pass through
unchanged because there is no local handler to wrap. No per-worker
`register.ts` knows or needs to know that this is happening, so no future
worker can accidentally skip the wrap.

**What `instrumentHandler` does per call.** See
[src/runtime/otel.ts](harness-node/src/runtime/otel.ts). On every
invocation it opens a `harness.<function_id>` SERVER span, extracts
`session_id` / `message_id` from the input body (with baggage fallback
for nested calls so a downstream `iii.trigger` inherits the IDs of its
parent), stamps the three `iii.*` attributes onto the span, and runs the
handler inside an OTel context that pushes those IDs as baggage. It also
records three lifecycle events that populate the EVENTS tab in the
traces UI:

- `iii.invocation.input` — redacted + truncated input JSON.
- `iii.invocation.output` — redacted + truncated result JSON (or
  `{error: ...}` on failure).
- `exception` — standard OTel event, only on throw, so the ERRORS tab
  picks the failure up.

Payload capture is gated on the `III_DISABLE_TRACE_PAYLOADS` env var
(matches the Rust `iii-sdk` semantics — set to `1` or `true` to suppress
payloads while keeping spans).

**Why baggage matters.** The engine relies on per-span attributes for
grouping, but only the outermost handler call sees the
`session_id`/`message_id` in its body. Pushing them as baggage means
every nested `iii.trigger` span (e.g. `harness.run::start` →
`harness.provider::anthropic::stream`) inherits the IDs automatically
via the iii-sdk's `BaggageSpanProcessor`. No manual plumbing per
call-site.

**Log bridge.** Pino keeps writing to stderr (devs still see logs in their
terminal during `pnpm dev:<worker>`). On top of that, every
`logger.info/warn/error` also emits an OTel log via `iii-sdk/telemetry`'s
`getLogger()`, which auto-correlates the log to the currently active
span. This is what populates the LOGS tab in the traces UI for every
harness-node span. When `initHarnessOtel` failed to boot (e.g.
`OTEL_ENABLED=false`), the OTel side is a quiet no-op and pino continues
to write to stderr unchanged.

**`harness::trigger` as the WS ingestion bridge.** Browser-originated
requests hit `harness::trigger` (see
[src/harness/trigger.ts](harness-node/src/harness/trigger.ts)), NOT
`run::start` directly. The wrapping `instrumentHandler` reads
`session_id`/`message_id` from the outer body and seeds baggage; the
handler then forwards to `iii.trigger` with the inner `function_id` /
`payload`. This is the symmetric counterpart of the Rust harness bridge
(`workers/harness/src/lib.rs:103-159`; legacy bus id `harness::call`) and
means the span tree looks the same regardless of whether the request
landed on a Rust or Node deployment.

```mermaid
sequenceDiagram
  participant Web as console/web
  participant Bridge as harness::trigger
  participant Wrap as instrumentHandler
  participant Inner as run::start (turn-orchestrator)
  participant Trace as engine traces UI

  Web->>Bridge: {function_id:"run::start", session_id, message_id, payload}
  Wrap->>Wrap: open span "harness.harness::trigger", stamp ids, push baggage
  Bridge->>Inner: iii.trigger(run::start, payload) -- baggage propagated
  Wrap->>Wrap: open span "harness.run::start", inherit ids from baggage
  Inner-->>Bridge: result
  Bridge-->>Web: {status_code:200, body:result}
  Wrap->>Trace: spans + iii.invocation.{input,output} events + correlated logs
```

## Configuration

All workers honour `--url` / `III_URL` (engine WebSocket) and `--config`
(YAML config file; defaults to `./config.yaml`). The shipped
[config.yaml](harness-node/config.yaml) contains the per-worker sub-sections;
each worker reads only the fields it cares about and ignores the rest. The
harness worker additionally watches the path in `permissions_path` (default
`./iii-permissions.yaml`, symlinked to the workspace-root file) and reloads
the policy on change.
