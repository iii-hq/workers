# harness

Node/TypeScript port of the iii harness stack. One package, one folder per
worker, one feature per file. Each worker is independently runnable as
`pnpm dev:<worker>` (development) or `iii-<worker>` (production binary).

The Rust workers `shell`, `iii-directory`, and the engine's `state::*`/
`stream::*`/`iii::durable::*` primitives are NOT ported — they run
alongside `harness` over the iii bus.

## Workers

| Folder | Bus surface | Role |
|---|---|---|
| `src/harness/` | `ui::subscribe`/`unsubscribe`, `harness::fs::read_inline`, `policy::check_permissions`, `harness::provider::{register,resolve,list}` | Meta-worker; loads `iii-permissions.yaml`; spins up `ui::*` fanout pumps; owns the provider registry + the `harness` entry in the `configuration` worker (api keys, per-provider settings, permissions). |
| `src/approval-gate/` | `approval::resolve` | Persists operator decisions to scope `approvals` (turn-orchestrator reacts via `turn::on_approval`); default mode seeded from `harness` config `permissions.default_mode`. |
| `src/turn-orchestrator/` | `run::start`, `turn::{state}`, `turn::get_state` | Durable FSM driving each agent turn; `dispatchWithHook` approval chokepoint. |
| `src/session/` | `session-tree::*` (11 fns), `session-inbox::*` (3 fns) | Branching session storage + per-session inbox queues. |
| `src/llm-budget/` | `budget::*` (14 fns) | Workspace + agent LLM spend caps. |
| `src/hook-fanout/` | `hook-fanout::publish_collect` | Generic publish-and-collect over a stream topic. |
| `src/models-catalog/` | `models::list`, `models::get`, `models::supports`, `models::register` | Model catalog populated exclusively by provider discovery (`provider::<name>::refresh_models` -> `models::register`); no embedded seed. |
| `src/provider-anthropic/` | `provider::anthropic::{stream,complete,refresh_models}` | Anthropic SSE → channel writer; self-declares to the harness registry; pulls `/v1/models` into the catalog. |
| `src/provider-openai/` | `provider::openai::{stream,complete,refresh_models}` | OpenAI SSE → channel writer; self-declares + pulls `/v1/models`. |
| `src/provider-kimi/` | `provider::kimi::{stream,complete,refresh_models}` | Kimi (Moonshot) SSE → channel writer; self-declares + pulls `/v1/models`. |
| `src/provider-lmstudio/` | `provider::lmstudio::{stream,complete,refresh_models,load_model,unload_model}` | LM Studio (localhost); self-declares + discovers loaded models. |
| `src/provider-llamacpp/` | `provider::llamacpp::{stream,complete,refresh_models}` | llama-server (localhost); self-declares + discovers the loaded model. |
| `src/context-compaction/` | (none — pure side-car on `agent::events`) | Optional out-of-band session-history compactor. |

## Quickstart

```bash
pnpm install
pnpm build              # compile to dist/
# In separate terminals (or via your process manager):
node dist/harness/main.js               --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/turn-orchestrator/main.js     --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/approval-gate/main.js         --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/session/main.js               --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/hook-fanout/main.js           --url ws://127.0.0.1:49134
node dist/models-catalog/main.js        --url ws://127.0.0.1:49134
node dist/provider-anthropic/main.js    --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/provider-openai/main.js       --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/provider-kimi/main.js         --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/provider-lmstudio/main.js     --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/provider-llamacpp/main.js     --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/llm-budget/main.js            --url ws://127.0.0.1:49134
# Optional side-car:
node dist/context-compaction/main.js    --url ws://127.0.0.1:49134
```

For development, replace `node dist/<worker>/main.js` with `pnpm dev:<worker>`.

## Configuration

All workers honour `--url` / `III_URL` for the engine WebSocket and
`--config` for the YAML config file (default `./config.yaml`).

The harness worker watches `iii-permissions.yaml` (default
`./iii-permissions.yaml`) and reloads it on change. The shipped default
file at the workspace root is symlinked into this folder.

## Layout

- `docs/` — architecture documentation: [`docs/architecture.md`](docs/architecture.md) is the system overview; one file per worker lives under [`docs/workers/`](docs/workers/).
- `src/types/` — wire types (mirrors `harness/crates/harness-types`).
- `src/runtime/` — cross-worker SDK helpers (worker bootstrap, state/stream wrappers, OTel stub).
- `src/<worker>/` — one folder per worker. Each `register.ts` composes the worker's bus surface from per-feature files; each `main.ts` is the binary entry-point.
- `tests/` — vitest suites per worker.
