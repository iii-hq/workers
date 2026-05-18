# harness-node

Node/TypeScript port of the iii harness stack. One package, one folder per
worker, one feature per file. Each worker is independently runnable as
`pnpm dev:<worker>` (development) or `iii-<worker>` (production binary).

The Rust workers `shell`, `iii-directory`, and the engine's `state::*`/
`stream::*`/`iii::durable::*` primitives are NOT ported — they run
alongside `harness-node` over the iii bus.

## Workers

| Folder | Bus surface | Role |
|---|---|---|
| `src/harness/` | `harness::status`, `ui::subscribe`/`unsubscribe`, `harness::fs::read_inline`, `policy::check_permissions` | Meta-worker; loads `iii-permissions.yaml`; spins up `ui::*` fanout pumps. |
| `src/approval-gate/` | `approval::resolve`, `approval::list_pending`, `policy::approval_gate` (subscriber) | Consults policy + pause-and-wait approval flow. |
| `src/turn-orchestrator/` | `run::start`, `run::start_and_wait`, `agent::call`, `turn::step` | Durable FSM driving each agent turn; chokepoint dispatcher. |
| `src/session/` | `session-tree::*` (11 fns), `session-inbox::*` (3 fns) | Branching session storage + per-session inbox queues. |
| `src/llm-budget/` | `budget::*` (14 fns) | Workspace + agent LLM spend caps. |
| `src/hook-fanout/` | `hook-fanout::publish_collect` | Generic publish-and-collect over a stream topic. |
| `src/auth-credentials/` | `auth::*` (file-backed) | Provider credential store. |
| `src/models-catalog/` | `models::list`, `models::get`, `models::supports` | Static model metadata. |
| `src/provider-anthropic/` | `provider::anthropic::stream`, `provider::anthropic::complete` | Anthropic SSE → channel writer. |
| `src/provider-openai/` | `provider::openai::stream`, `provider::openai::complete` | OpenAI SSE → channel writer. |
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
node dist/auth-credentials/main.js      --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/models-catalog/main.js        --url ws://127.0.0.1:49134
node dist/provider-anthropic/main.js    --url ws://127.0.0.1:49134 --config ./config.yaml
node dist/provider-openai/main.js       --url ws://127.0.0.1:49134 --config ./config.yaml
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
