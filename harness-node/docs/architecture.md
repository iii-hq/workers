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
| harness | [src/harness/](harness-node/src/harness/) | Meta-worker; loads `iii-permissions.yaml`, exposes `harness::*` / `policy::check_permissions` / `ui::*`, spins up `agent::events` fan-out. | [workers/harness.md](harness-node/docs/workers/harness.md) |
| turn-orchestrator | [src/turn-orchestrator/](harness-node/src/turn-orchestrator/) | Durable FSM driving each agent turn; chokepoint dispatcher for `agent::call`. | [workers/turn-orchestrator.md](harness-node/docs/workers/turn-orchestrator.md) |
| approval-gate | [src/approval-gate/](harness-node/src/approval-gate/) | Hook subscriber on `agent::before_function_call`; consults policy and pauses for user resolution. | [workers/approval-gate.md](harness-node/docs/workers/approval-gate.md) |
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

  client -- "run::start" --> turnOrch
  client -- "ui::subscribe / harness::status" --> harness

  turnOrch -- "provider::*::stream" --> provAnth
  turnOrch -- "provider::*::stream" --> provOAI
  turnOrch -- "agent::call (chokepoint)" --> approval
  turnOrch -- "session-tree::* mirror" --> session
  turnOrch -- "state::* persistence" --> state

  approval -- "policy::check_permissions" --> harness
  approval -- "subscribes agent::before_function_call" --> state

  provAnth -- "auth::get_token" --> auth
  provOAI -- "auth::get_token" --> auth

  state -- "agent::events stream" --> harness
  state -- "agent::events stream" --> compact
  harness -- "ui::session::event::<browser_id>" --> client
  compact -- "session-tree::compact" --> session

  approval -- "hook-fanout::publish_collect" -.optional.-> hook
```

## Turn FSM

[src/turn-orchestrator/state.ts](harness-node/src/turn-orchestrator/state.ts)
defines a 10-state durable FSM. Every transition is driven by the
`turn::step` durable subscriber, which is woken by a publish to the
`turn::step_requested` topic.

```mermaid
stateDiagram-v2
  [*] --> provisioning
  provisioning --> awaiting_assistant
  awaiting_assistant --> assistant_streaming
  assistant_streaming --> assistant_finished
  assistant_finished --> function_prepare: has function calls
  assistant_finished --> steering_check: no function calls
  function_prepare --> function_execute
  function_execute --> function_finalize
  function_finalize --> steering_check
  steering_check --> awaiting_assistant: continue
  steering_check --> tearing_down: stop or max turns
  tearing_down --> stopped
  stopped --> [*]
```

## Approval flow

```mermaid
sequenceDiagram
  participant Turn as turn-orchestrator
  participant Bus as iii bus (state::* + stream::*)
  participant Gate as approval-gate (policy::approval_gate)
  participant Harness as harness (policy::check_permissions)
  participant User

  Turn->>Bus: publish agent::before_function_call (via hook-fanout-style envelope)
  Bus-->>Gate: durable:subscriber wakes
  Gate->>Harness: policy::check_permissions(function_id, args)
  alt rule.action == allow
    Harness-->>Gate: allow
    Gate-->>Bus: reply allow
  else rule.action == deny
    Harness-->>Gate: deny + DenialEnvelope
    Gate-->>Bus: reply deny
  else no rule (needs_approval)
    Harness-->>Gate: needs_approval
    Gate->>Bus: state::set approvals/<sid>/<call_id> = pending
    User->>Bus: approval::resolve(decision)
    Gate-->>Bus: reply allow or deny (poll-loop)
  end
  Bus-->>Turn: hook reply
```

## Kernel deny list

[iii-permissions.yaml](iii-permissions.yaml) at the workspace root is the
single source of truth for what an in-run agent can call. The harness loads
it at boot and watches for changes via `chokidar`. Rules are scanned
top-to-bottom; first match wins. Kernel rules below are shipped by default
and protect the gate / state surface.

| Rule id | Function | Action | Why |
|---|---|---|---|
| `kernel/no-self-approve` | `approval::resolve` | deny | An agent must not flip its own pending approval. |
| `kernel/no-self-policy` | `policy::check_permissions` | deny | Don't let the model trace or precompute its own gate decision. |
| `kernel/no-self-policy-gate` | `policy::approval_gate` | deny | Same as above, for the subscriber. |
| `kernel/no-self-hook` | `hook-fanout::publish_collect` | deny | The hook primitive is operator-only. |
| `kernel/no-state-set` | `state::set` | deny | Use a worker function; never raw state writes. |
| `kernel/no-state-update` | `state::update` | deny | Same. |
| `kernel/no-state-delete` | `state::delete` | deny | Same. |
| `kernel/no-stream-set` | `stream::set` | deny | Bypasses the agent loop. |
| `kernel/no-durable-publish` | `iii::durable::publish` | deny | Bypasses the durable hook plane. |
| `kernel/no-auth-set` | `auth::set_token` | deny | Token plumbing is operator-only. |
| `kernel/no-auth-delete` | `auth::delete_token` | deny | Same. |
| `kernel/no-oauth-anthropic-login` | `oauth::anthropic::login` | deny | Same. |
| `kernel/no-oauth-openai-login` | `oauth::openai-codex::login` | deny | Same. |
| `kernel/no-self-run-start` | `run::start` | deny | No re-entrant runs. |
| `kernel/no-self-run-start-and-wait` | `run::start_and_wait` | deny | Same. |
| `kernel/no-router-stream-assistant` | `router::stream_assistant` | deny | Routing internal. |
| `kernel/no-router-abort` | `router::abort` | deny | Routing internal. |

Bare-string allow rules: `harness::status`, `state::get`, `state::list`,
`models::list`, `models::get`, `models::supports`, `auth::get_token`,
`auth::list_providers`, `auth::status`, `oauth::anthropic::status`,
`oauth::openai-codex::status`, the `directory::engine::*` introspection
surface, the `directory::skills::*` and `directory::prompts::*` lookups, and
`approval::list_pending`.

## Shared modules

| Folder | Purpose |
|---|---|
| [src/runtime/](harness-node/src/runtime/) | Cross-worker SDK helpers. `worker.ts` parses CLI flags and bootstraps an SDK connection; `config.ts` loads `config.yaml`; `state.ts` / `stream.ts` wrap the iii engine's state/stream surface; `ids.ts` mints UUID-like ids; `otel.ts` provides a pino logger and OTel stub; `handler.ts` exposes `unwrapBody` / `requireString`. |
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

## Configuration

All workers honour `--url` / `III_URL` (engine WebSocket) and `--config`
(YAML config file; defaults to `./config.yaml`). The shipped
[config.yaml](harness-node/config.yaml) contains the per-worker sub-sections;
each worker reads only the fields it cares about and ignores the rest. The
harness worker additionally watches the path in `permissions_path` (default
`./iii-permissions.yaml`, symlinked to the workspace-root file) and reloads
the policy on change.
