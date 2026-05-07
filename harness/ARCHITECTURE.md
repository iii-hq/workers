# Harness Architecture

The `harness` is a meta-worker for the [iii](https://github.com/iii-experimental/harness) bus. It does not implement chat, agents, or providers itself — it **composes** 14 specialized workers into a runnable chat surface, exposes a small browser-facing HTTP bridge, and ships a Vite/React UI that talks to the bus through that bridge.

```
                         ┌────────────────────────────┐
        browser  ───────►│ harness/web (React + Vite) │
        :5173            └──────────────┬─────────────┘
                                        │ POST /bridge/trigger
                                        │ {function_id, payload}
                              vite proxy │ → http://127.0.0.1:3111
                                        ▼
                         ┌────────────────────────────┐
                         │  iii engine HTTP trigger   │
                         │  registered by harness     │
                         └──────────────┬─────────────┘
                                        │ iii.trigger(function_id, payload)
                                        ▼
            ┌────────────────────────────────────────────────────┐
            │                  iii bus  (ws://:49134)            │
            │                                                    │
            │  harness::status     bridge::trigger               │◄── iii-harness
            │  run::start          turn::*                       │◄── turn-orchestrator
            │  agent::call         provider::*                  │◄── turn-orchestrator, …
            │  session::* state::*                               │◄── session-tree
            │  shell::filesystem::* shell::bash::*               │◄── shell-* workers
            │  agent::before_tool_call (topic)                   │◄── policy-denylist
            │  auth::* skills::register …                        │◄── auth-credentials, skills
            └────────────────────────────────────────────────────┘
```

## Components

### 1. `iii-harness` worker (`src/`)

Single-binary Rust worker (`src/main.rs` → `src/lib.rs`) that connects to a running iii engine and registers exactly two functions plus one HTTP trigger.

| Function | Source | Purpose |
|---|---|---|
| `harness::status` | `lib.rs:71` | Returns `{ok, name, version, expected_workers[]}`. Used as the cheapest "is the bundle alive" probe. |
| `bridge::trigger` | `lib.rs:87` | Forwards `{function_id, payload}` from HTTP onto `iii.trigger(...)`. Backed by an HTTP trigger at `POST /bridge/trigger` (`lib.rs:123`). |

Boot sequence (`register_with_iii`, `lib.rs:70`):

1. Register `harness::status`.
2. Register `bridge::trigger` and bind it to `POST /bridge/trigger` with a 4-minute timeout (`BRIDGE_TIMEOUT_MS`, `lib.rs:16`) — long enough for multi-turn tool-calling to complete before the engine's default 30s 504.
3. Best-effort fire `skills::register` with the harness's skill descriptor (`build_skills_register_payload`, `lib.rs:45`). A missing `skills` worker does not block boot.

The harness exposes two dispatchers:

- `bridge::trigger` (this worker) — HTTP-bound, browser path. The browser
  POSTs `{function_id, payload}`; this forwards to `iii.trigger(...)`. Not
  advertised to the LLM.
- `agent::call` (turn-orchestrator) — LLM-facing. The provider sees one
  tool, `agent_call`, with `{function, payload}` arguments. Validates
  payload against the target's `request_format`, lazy-provisions sandboxes
  on the first `shell::*` call, and dispatches via `iii.trigger(...)`.
  Both appear in `engine::functions::list`.

`bridge::trigger` is **intentionally not advertised as an LLM tool** (`tools: []` in the skill payload). It exists only as the browser's call-anything escape hatch — exposing it to a model would let the model call itself recursively.

### 2. `harness/web` — React UI (`web/`)

Vite + React 18 single-page app. All bus calls go through one helper:

- `web/src/bridge.ts` — `bridge<T>(functionId, payload)` does `POST /bridge/trigger`, returns the result, surfaces engine errors as `BridgeError`.
- `vite.config.ts:9` — dev server proxies `/bridge` → `http://127.0.0.1:3111` (the engine's HTTP trigger port). Same path works in production behind any reverse proxy that forwards `/bridge`.

The UI does not ship tool schemas: `turn-orchestrator` provisions a single
`agent_call` tool (see `agent_call_tool`) and builds the system prompt
server-side. The model passes `function` (a bus id such as
`shell::filesystem::ls`) and `payload` (arguments). Permission is enforced by
`policy-denylist` on `agent::before_tool_call` (see Trust boundary below).

### 3. The 14 expected workers

`EXPECTED_WORKERS` (`lib.rs:18`) is the source of truth for what the harness assumes is on the bus. Grouped by role:

| Group | Workers | Role |
|---|---|---|
| Orchestration | `turn-orchestrator`, `provider-router` | Runs a turn end-to-end: fan a request to a provider and dispatch tool calls. |
| Sessions / state | `session-tree`, `session-inbox` | Persisted message trees and a steering/follow-up inbox queue. |
| Catalog | `models-catalog` | Model metadata. |
| Auth | `auth-credentials` | Provider credentials store. |
| Policy / safety | `policy-denylist`, `llm-budget` | Hook subscriber on `agent::before_tool_call` and budget tracking. |
| Hooks | `hook-fanout` | Generic publish-and-collect primitive. |
| Tools | `shell-bash`, `shell-filesystem`, `subagent` | LLM-callable tool implementations. |
| Providers | `provider-anthropic`, `provider-openai` | Concrete LLM transport workers behind `provider-router`. |

The harness owns *no* logic from any of these — it only knows their names. Each worker is a separate crate in `workers/<name>/` with its own `iii.worker.yaml`, lifecycle, and tests.

The integration test `tests/integration.rs:7` enforces that `EXPECTED_WORKERS` and `iii.worker.yaml` dependencies stay in sync. (Note: `iii.worker.yaml` is currently absent from `harness/` — the test will fail until the manifest is restored.)

### 4. `scripts/demo.sh` — local orchestration

The harness ships without a registry entry, so `iii worker add harness` does not work yet. `scripts/demo.sh` is the supported way to bring up the full stack from this checkout:

```
demo.sh build    # cargo build --release for harness + 14 workers
demo.sh engine   # start `iii --use-default-config` in background
demo.sh start    # spawn all 14 workers + harness as nohup processes
demo.sh verify   # call harness::status, models::list, provider::cli::list_models
demo.sh web      # npm install + vite in a tmux session
demo.sh stop     # kill every PID in $DEMO_DIR/pids/ + engine + tmux
demo.sh all      # build + engine + start + verify
```

PIDs and logs live under `$DEMO_DIR` (default `~/iii-harness-demo`). One PID file per worker, one log file per worker — no shared logger, no daemon supervisor.

`scripts/real-usage.sh` exercises the running stack end-to-end: `auth::set_token` → `run::start_and_wait` → `state::get` for both messages and turn record → `state::list` to enumerate sessions.

## Runtime data flow

A user message from the browser:

```
1. UI calls bridge("run::start_and_wait", {session_id, provider, model, messages})
2. POST /bridge/trigger hits the engine's HTTP trigger
3. bridge::trigger handler unwraps {body} → {function_id, payload}
4. iii.trigger("run::start_and_wait", payload, timeout=240s)
5. turn-orchestrator picks it up, runs the agent loop:
     - emits `agent::before_tool_call` (subscribers: policy-denylist, llm-budget)
     - routes each tool execution through `agent_call::dispatch` (schema check, lazy sandbox, then `iii.trigger` to the inner function)
     - calls provider-router → provider-anthropic / provider-openai
     - persists transcript via session-tree / state
6. turn-orchestrator returns full transcript
7. bridge::trigger wraps it as {status_code, headers, body} and returns to the browser
```

Sessions are persisted in two stores that don't merge automatically:

- `run::start_and_wait` writes under `scope="agent"`, key `session/<id>/messages` and `session/<id>/turn_state` (engine state).
- `session::tree` / `session::messages` read `session-tree`'s own store, populated only by explicit `session::create` + `session::append`.

`scripts/real-usage.sh:5-11` documents this — anything reading "all sessions" today should `state::list scope=agent prefix="session/"` and filter for objects carrying `session_id`+`state`.

## Trust boundary

The harness assumes a layered model and does not enforce policy itself:

1. **SDK wrapper (chat client side)** — workspace allowlist on path arguments before the bus call is dispatched.
2. **`policy-denylist` (engine side)** — subscriber on `agent::before_tool_call` that blocks by tool name. Configured via `POLICY_DENIED_TOOLS` env var, e.g. `shell::filesystem::rm,shell::filesystem::sed,shell::filesystem::edit,shell::filesystem::chmod,shell::filesystem::mv`.
3. **`<ApprovalRow>` (chat UI)** — per-call user approval surfaced inline before any write reaches disk.

`bridge::trigger` is the one bus surface reachable from the browser. It has **no** allowlist — any function id is callable — so the deployment must keep `:3111` private and rely on the three layers above. There is no per-user auth on `bridge::trigger`; the harness assumes a single-tenant local install.

## Versioning and skill registration

On boot the harness publishes a skill descriptor via `skills::register`:

```json
{
  "id": "harness",
  "skill_version": "<Cargo.toml version>",
  "min_console_version": "0.1.0",
  "body": "Harness meta-worker. Composes the modular workers …",
  "expected_workers": [ … 14 … ],
  "tools": []
}
```

Empty `tools` is deliberate — the harness has no LLM-callable functions of its own. `harness::status` is for operators; `bridge::trigger` is for the browser.

## What lives where

```
harness/
├── Cargo.toml                # iii-harness crate, depends on iii-sdk = =0.11.3
├── src/
│   ├── lib.rs                # register_with_iii, EXPECTED_WORKERS, build_skills_register_payload
│   └── main.rs               # connect to III_URL (default ws://127.0.0.1:49134), register, wait for ctrl-c
├── tests/
│   └── integration.rs        # asserts EXPECTED_WORKERS ↔ iii.worker.yaml stay aligned
├── scripts/
│   ├── demo.sh               # build / engine / start / verify / logs / web / stop / all
│   └── real-usage.sh         # end-to-end auth → run → state read
└── web/
    ├── package.json          # React 18 + Vite 5
    ├── vite.config.ts        # /bridge → 127.0.0.1:3111 proxy
    └── src/
        ├── bridge.ts         # bridge<T>(functionId, payload)
        ├── App.tsx           # session UI, composer, panels (no client-side tool catalog)
        ├── components/       # AuthPanel, Composer, ContextMeter, ControlsBar,
        │                     # CostPanel, FilesystemPanel, SessionList,
        │                     # SessionView, StatusPill
        └── types.ts          # AgentMessage, AuthStatus, ModelInfo, SessionRow
```

## Design choices worth knowing

- **Meta-worker, not framework.** Adding a capability means adding a worker crate and listing it in `EXPECTED_WORKERS` + `iii.worker.yaml`. The harness binary itself stays small.
- **Single browser endpoint.** One HTTP trigger (`/bridge/trigger`) reaches the entire bus, so the UI never needs new HTTP routes when the bus grows. Cost: `bridge::trigger` is a powerful primitive that must not leak past the local trust boundary.
- **No registry coupling.** `demo.sh` runs every worker straight from `target/release/iii-<name>`, so the bundle works against an unmodified upstream `iii` engine without any registry index entries.
- **Drift detection via test.** `tests/integration.rs` is the only thing keeping `EXPECTED_WORKERS` honest against the YAML manifest — keep both in sync when adding/removing workers.
- **Long bridge timeout.** 4 minutes is high on purpose: a multi-tool turn with Opus + filesystem ops routinely exceeds 30s, and a 504 mid-turn would orphan the orchestrator's bookkeeping.
