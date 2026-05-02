# iii workers

Workers for the [iii engine](https://github.com/iii-hq/iii). Each directory is a
self-contained worker module: a process that connects to the engine over
WebSocket, registers functions + triggers, and does something useful.

Workers are discoverable through the [registry](registry/index.json) and — for
binary-shipped workers — installed via `iii worker add <name>`, which pulls the
matching GitHub Release asset for the host's target triple.

## Modules

| Worker | Kind | Summary |
|---|---|---|
| [`audit-log`](audit-log/) | Rust | Append-only JSON-lines audit log of every tool call + result on `agent::after_tool_call`. |
| [`auth-credentials`](auth-credentials/) | Rust | Provider credential vault under `auth::*` — API keys and OAuth tokens. |
| [`auth-rbac`](auth-rbac/) | Rust | HMAC API keys and workspace roles (owner/admin/member/viewer) under `auth::rbac::*`. |
| [`context-compaction`](context-compaction/) | Rust | Subscriber that triggers session compaction once context-window thresholds are reached. |
| [`dlp-scrubber`](dlp-scrubber/) | Rust | Hook subscriber on `agent::after_tool_call` that redacts common secret shapes in tool result text. |
| [`document-extract`](document-extract/) | Rust | PDF/Word text extraction under `document::extract` for agent context ingestion. |
| [`durable-queue`](durable-queue/) | Rust | Per-session durable queues under `queue::*` (push, drain, peek). |
| [`guardrails`](guardrails/) | Rust | Local heuristics for PII, leaked API keys, jailbreak keywords, and toxicity under `guardrails::*`. |
| [`harness-runtime`](harness-runtime/) | Rust | `agent::stream_assistant` provider router plus `agent::abort` and `agent::push_steering` / `push_followup` helpers. |
| [`hook-fanout`](hook-fanout/) | Rust | Reusable publish-collect primitive under `hooks::publish_collect` — fans an event to subscribers and merges replies. |
| [`iii-lsp`](iii-lsp/) | Rust | Language Server for iii function ids, trigger configs, and worker discovery. Autocomplete/hover across JS/TS, Python, Rust. |
| [`iii-lsp-vscode`](iii-lsp-vscode/) | Node | VS Code extension that embeds `iii-lsp`. |
| [`image-resize`](image-resize/) | Rust | Image resize via channel I/O. JPEG/PNG/WebP with EXIF auto-orient, scale-to-fit / crop-to-fit. |
| [`llm-budget`](llm-budget/) | Rust | Workspace + agent LLM spend caps with alerts, forecast, and period rollover under `budget::*`. |
| [`mcp`](mcp/) | Rust | Model Context Protocol surface — stdio + HTTP JSON-RPC, exposes iii functions tagged `mcp.expose` as MCP tools. |
| [`models-catalog`](models-catalog/) | Rust | Model capabilities knowledge base under `models::*` (list/get/supports/register). |
| [`policy-denylist`](policy-denylist/) | Rust | Hook subscriber on `agent::before_tool_call` that blocks calls whose name is on a configured denylist. |
| [`proof`](proof/) | Node | AI-driven browser testing — diffs changes, generates test plans, drives Playwright. |
| [`provider-anthropic`](provider-anthropic/) | Rust | Native Anthropic Messages API streaming provider under `provider::anthropic::*`. |
| [`provider-azure-openai`](provider-azure-openai/) | Rust | Azure OpenAI Responses provider under `provider::azure-openai::*`. |
| [`provider-bedrock`](provider-bedrock/) | Rust | AWS Bedrock provider under `provider::bedrock::*`. (Stub today; emits a not-implemented error.) |
| [`provider-cerebras`](provider-cerebras/) | Rust | OpenAI-compatible Cerebras provider under `provider::cerebras::*`. |
| [`provider-cli`](provider-cli/) | Rust | Wraps installed coding CLIs (claude, codex, opencode, openclaw, hermes, pi, gemini, cursor-agent) under `provider::cli::*`. |
| [`provider-deepseek`](provider-deepseek/) | Rust | OpenAI-compatible DeepSeek provider under `provider::deepseek::*`. |
| [`provider-fireworks`](provider-fireworks/) | Rust | OpenAI-compatible Fireworks provider under `provider::fireworks::*`. |
| [`provider-google`](provider-google/) | Rust | Google Gemini provider under `provider::google::*`. |
| [`provider-google-vertex`](provider-google-vertex/) | Rust | Vertex AI Gemini provider under `provider::google-vertex::*`. |
| [`provider-groq`](provider-groq/) | Rust | OpenAI-compatible Groq provider under `provider::groq::*`. |
| [`provider-huggingface`](provider-huggingface/) | Rust | OpenAI-compatible Hugging Face Inference provider under `provider::huggingface::*`. |
| [`provider-kimi-coding`](provider-kimi-coding/) | Rust | OpenAI-compatible Moonshot Kimi coding provider under `provider::kimi-coding::*`. |
| [`provider-minimax`](provider-minimax/) | Rust | OpenAI-compatible MiniMax provider under `provider::minimax::*`. |
| [`provider-mistral`](provider-mistral/) | Rust | OpenAI-compatible Mistral La Plateforme provider under `provider::mistral::*`. |
| [`provider-openai`](provider-openai/) | Rust | OpenAI Chat Completions provider under `provider::openai::*`. |
| [`provider-openai-responses`](provider-openai-responses/) | Rust | OpenAI Responses API provider under `provider::openai-responses::*`. |
| [`provider-opencode-go`](provider-opencode-go/) | Rust | OpenAI-compatible opencode Go endpoint under `provider::opencode-go::*`. |
| [`provider-opencode-zen`](provider-opencode-zen/) | Rust | OpenAI-compatible opencode Zen endpoint under `provider::opencode-zen::*`. |
| [`provider-openrouter`](provider-openrouter/) | Rust | OpenAI-compatible OpenRouter routing layer under `provider::openrouter::*`. |
| [`provider-vercel-ai-gateway`](provider-vercel-ai-gateway/) | Rust | OpenAI-compatible Vercel AI Gateway provider under `provider::vercel-ai-gateway::*`. |
| [`provider-xai`](provider-xai/) | Rust | OpenAI-compatible xAI Grok provider under `provider::xai::*`. |
| [`provider-zai`](provider-zai/) | Rust | OpenAI-compatible Z.ai provider under `provider::zai::*`. |
| [`session-corpus`](session-corpus/) | Rust | Dataset publishing pipeline for completed sessions under `corpus::*` — secret scan, redact, review, publish. |
| [`session-tree`](session-tree/) | Rust | Session storage as a parent-id tree of typed entries under `session::*`. |
| [`state-flag`](state-flag/) | Rust | Per-session boolean flags under `flag::set`, `flag::clear`, `flag::is_set`. |
| [`shell-bash`](shell-bash/) | Rust | Sandboxed shell execution under `shell::bash::*` — wraps the engine `sandbox::exec` primitive. |
| [`shell-filesystem`](shell-filesystem/) | Rust | Sandboxed filesystem operations under `shell::fs::*` — read, write, list, stat, glob. |
| [`shell-subagent`](shell-subagent/) | Rust | Spawn child agent sessions under `shell::subagent::*` via `run::start_and_wait`. |
| [`turn-orchestrator`](turn-orchestrator/) | Rust | Durable `run::start` state machine driving each agent turn through provisioning, assistant, tools, steering, and tearing-down. |
| [`todo-worker`](todo-worker/) | Node | Quickstart CRUD todo worker using the Node iii SDK. |
| [`todo-worker-python`](todo-worker-python/) | Python | Quickstart CRUD todo worker using the Python iii SDK. |

## SDK

All workers target [`iii-sdk`](https://crates.io/crates/iii-sdk) v0.11.3
(Rust), `iii-sdk@0.11.3` on npm, or `iii-sdk==0.11.3` on PyPI.

## Build

Rust workers:

```bash
cd <worker>
cargo build --release
```

Node/Python workers follow the standard `npm install` / `pip install -e .`
flow — see each module's README for specifics.

## Binary releases

All Rust workers ship as standalone binaries — see the modules table above
— and are released via GitHub Actions:

1. Trigger the **Create Tag** workflow (Actions tab) — pick a worker, bump
   type (`patch`/`minor`/`major`), and a registry tag (`latest` / `next`).
2. A tag of the form `<worker>/v<X.Y.Z>` is pushed to `main`, with the
   registry tag embedded in the tag's annotated message.
3. The unified **Release** workflow fires on the tag, builds cross-compiled
   binaries for 9 targets (Linux gnu/musl, macOS x86_64 + aarch64, Windows
   x86_64/i686/aarch64, armv7), uploads them to a GitHub Release with
   SHA-256 checksums, and calls `POST /publish` on the registry API.

Targets per build:

```text
aarch64-apple-darwin
x86_64-apple-darwin
x86_64-pc-windows-msvc
i686-pc-windows-msvc
aarch64-pc-windows-msvc
x86_64-unknown-linux-gnu
x86_64-unknown-linux-musl
aarch64-unknown-linux-gnu
armv7-unknown-linux-gnueabihf
```

## Registry

[`registry/index.json`](registry/index.json) is the discovery manifest. Each
entry declares the worker kind (`binary` / container image), the release tag
prefix, supported targets, and a default config. `iii worker add <name>`
reads this file to locate the right asset for the host.

## Add a new worker

See [`AGENTS-NEW-WORKER.md`](AGENTS-NEW-WORKER.md) for the full checklist
covering folder layout, `iii.worker.yaml`, lint, tests, deploy type
(`binary` vs. `image`), and the release flow.

## CI

Pull requests trigger per-worker lint + tests for the changed worker(s).
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) discovers changes by
reading each worker's `iii.worker.yaml`, then routes:

- Rust → `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --all-features`
- Node → `biome ci` against [`biome.json`](biome.json) and `npm test`
- Python → `ruff check` + `ruff format --check` against [`ruff.toml`](ruff.toml) and `pytest`

The `pr-checks` job additionally enforces, per changed worker: `README.md`
present, `iii.worker.yaml` valid, `tests/` non-empty, and the manifest
version is greater than the version on the PR's base branch.

## CD

Releases are cut manually via the **Create Tag** workflow
([`.github/workflows/create-tag.yml`](.github/workflows/create-tag.yml)) —
pick a worker, a bump type, and a registry tag (`latest` / `next`). The
resulting `<worker>/v<X.Y.Z>` tag drives a single dispatcher
([`.github/workflows/release.yml`](.github/workflows/release.yml)) that:

1. Routes on `deploy` from `iii.worker.yaml`:
   - `binary` → cross-compile to 9 targets via
     [`_rust-binary.yml`](.github/workflows/_rust-binary.yml).
   - `image` → multi-arch image to `ghcr.io/<owner>/<worker>` via
     [`_container.yml`](.github/workflows/_container.yml).
2. Calls `POST /publish` against the workers registry API
   ([`openapi.yaml`](openapi.yaml)) via
   [`_publish-registry.yml`](.github/workflows/_publish-registry.yml).

## License

Apache 2.0 — see [`LICENSE`](LICENSE).
