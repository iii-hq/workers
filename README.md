# iii workers

Workers for the [iii engine](https://github.com/iii-hq/iii). Each top-level
directory is a self-contained worker module: a process that connects to the
engine over WebSocket, registers functions + triggers, and does something
useful.

Workers are installed via `iii worker add <name>`, which resolves the matching
asset for the host from the workers registry API.

## Skills

Each worker ships an agent skill under `<worker>/skills/`. Install them with the
`skills` CLI (works with Claude Code, Cursor, and 30+ other agents).

```bash
# List every worker skill in this repo
npx skills add iii-hq/workers --list

# Install one worker's skill
npx skills add iii-hq/workers --skill database

# Install several
npx skills add iii-hq/workers --skill database,coder,shell

# Install all worker skills at once
npx skills add iii-hq/workers --all
```

For the iii engine's top-level skills (mental model, SDKs, config, patterns),
see [`iii-hq/iii`](https://github.com/iii-hq/iii):

```bash
npx skills add iii-hq/iii --all
```

## Modules

| Worker | Kind | Summary |
|---|---|---|
| [`acp`](acp/) | Rust | Agent Client Protocol surface — stdio JSON-RPC, exposes iii agents as ACP sessions. |
| [`approval-gate`](approval-gate/) | Rust | Human-in-the-loop approval gate — evaluates each function call (continue / deny / hold), holds pending calls for a human, and emits `approval::pending-*` events. Binds the harness `pre_trigger` hook. See [`approval-gate/architecture/`](approval-gate/architecture/). |
| [`harness`](harness/) | Node | TS port of the iii harness stack — bundles `harness` (provider registry + credentials/settings/permissions via the `configuration` worker), `turn-orchestrator`, `hook-fanout`, `models-catalog`, the `provider-*` workers, `llm-budget`, and `context-compaction` as one pnpm monorepo. Approval is delegated to the standalone `approval-gate` worker via the `pre_trigger` hook. Conversations persist in `session-manager`. See [`harness/README.md`](harness/README.md). |
| [`codex`](codex/) | Rust | OpenAI Codex as an iii worker — `codex::*` spawn the codex CLI for headless turns, mirror raw thread events onto `codex::events`, and stream AgentEvent frames onto `agent::events`. |
| [`grok`](grok/) | Rust | xAI Grok CLI as an iii worker — `grok::*` spawn the grok CLI for headless turns (`grok --print --output-format streaming-json`), mirror raw events onto `grok::events`, and stream AgentEvent frames onto `agent::events`. |
| [`devin`](devin/) | Rust | Devin as an iii worker: `devin::run` drives the local devin CLI and streams AgentEvent frames onto `agent::events`, `devin::session::*` wrap the Devin cloud session lifecycle, and `devin::api` reaches any v3 endpoint. |
| [`claude-code`](claude-code/) | Node | Claude Code as an iii worker — `claude::*` runs headless Claude Code turns, mirrors raw messages onto `claude::events`, and streams AgentEvent frames onto `agent::events`. |
| [`pi`](pi/) | Node | Pi coding agent as an iii worker — `pi::*` run headless Pi turns, mirror raw events onto `pi::events`, and stream AgentEvent frames onto `agent::events`. |
| [`hermes`](hermes/) | Python | Hermes agent as an iii worker — `hermes::run` runs headless turns with the iii runtime context, `hermes::send` delivers to 27+ messaging platforms, and inbound platform/webhook events republish via `hermes::inbound`. |
| [`opencode`](opencode/) | Node | OpenCode as an iii worker — `opencode::*` run headless OpenCode turns via `opencode run --format json`, mirror raw JSON events onto `opencode::events`, and stream AgentEvent frames (with usage + cost) onto `agent::events`. |
| [`session-manager`](session-manager/) | Rust | Durable, reactive, branching conversation store — fourteen `session::*` functions plus six trigger types; the transcript backend for `harness` and `console`. See [`session-manager/architecture/`](session-manager/architecture/). |
| [`telegram-bot`](telegram-bot/) | Rust | Telegram webhook bridge to the harness stack — live message edits, inline approval keyboards, and configurable verbosity. |
| [`slack`](slack/) | Rust | Slack Web API as `slack::*` functions plus a harness bridge — @mention-triggered turns, native `chat.*Stream` replies, Block Kit approvals. See [`slack/architecture/`](slack/architecture/). |
| [`context-manager`](context-manager/) | Rust | Model-ready context assembly — four `context::*` functions for token counting, function-result pruning, and history compaction over caller-supplied messages. Storage-agnostic; summarisation via `llm-router` when installed. |
| [`database`](database/) | Rust | PostgreSQL, MySQL, and SQLite client — query, execute, transactions, prepared statements, and change feeds. |
| [`iii-directory`](iii-directory/) | Rust | Engine introspection (functions / triggers / workers), workers-registry proxy, and filesystem-backed skill + prompt reader. |
| [`lsp`](lsp/) | Rust | Language Server for iii function ids, trigger configs, and worker discovery. Autocomplete / hover across JS/TS, Python, Rust. |
| [`lsp-vscode`](lsp-vscode/) | Node | VS Code extension package `iii-lsp`, embedding the `lsp` server. |
| [`image-resize`](image-resize/) | Rust | Image resize via channel I/O — JPEG/PNG/WebP with EXIF auto-orient, scale-to-fit / crop-to-fit. |
| [`llm-router`](llm-router/) | Rust | One front door + provider protocol in front of every LLM provider — `router::chat`/`router::complete`, provider registry + credentials, model catalog, and routing. See [`llm-router/README.md`](llm-router/README.md). |
| [`mcp`](mcp/) | Rust | MCP 2025-06-18 Streamable HTTP bridge — exposes iii functions tagged `mcp.expose` as MCP tools. |
| [`rbac-proxy`](rbac-proxy/) | Rust | RBAC boundary proxy for the iii worker protocol — opens its own port and reverse-proxies functions + channels to a trusted engine listener, authenticating each connection, gating every invocation and trigger binding, namespacing registrations, running middleware + registration hooks, and filtering the `engine::*` discovery results to the caller's boundaries. The [`console`](console/) reverse-proxy with the engine's RBAC vendored in front, out of process. |
| [`provider-anthropic`](provider-anthropic/) | Rust | Anthropic Messages API provider behind `llm-router` — `provider::anthropic::stream` with prompt caching, thinking, and live model discovery. |
| [`provider-llamacpp`](provider-llamacpp/) | Rust | llama.cpp server (`llama-server`) Chat Completions provider behind `llm-router` — `provider::llamacpp::stream` with optional (no-`--api-key`) auth, real json_schema-constrained output, and live model discovery via `/v1/models` + `/props`. |
| [`provider-openai`](provider-openai/) | Rust | OpenAI Chat Completions provider behind `llm-router` — `provider::openai::stream` with reasoning support and live chat-model discovery. |
| [`provider-xai`](provider-xai/) | Rust | xAI (Grok) Chat Completions provider behind `llm-router` — `provider::xai::stream` with grok reasoning support and live model discovery against `api.x.ai`. |
| [`provider-zai`](provider-zai/) | Rust | Z.AI (GLM) Chat Completions provider behind `llm-router` — `provider::zai::stream` with GLM thinking/effort support and a curated catalog against `api.z.ai` (no upstream model listing). |
| [`shell`](shell/) | Rust | Unix shell + filesystem worker — `shell::exec` with denylist/timeout/output caps and background jobs; `fs::ls`/`stat`/`mkdir`/`rm`/`chmod`/`mv`/`grep`/`sed`/`read`/`write` with host jail, denylist, and size caps. |
| [`storage`](storage/) | Rust | S3-compatible object storage across AWS S3, GCS, Cloudflare R2, and a managed local rustfs backend. Streamed uploads, presigned URLs, and object change triggers. |
| [`scrapling`](scrapling/) | Python | [Scrapling](https://github.com/D4Vinci/Scrapling) as an iii worker — `scrapling::*` map three fetch tiers (HTTP / Camoufox stealth / Playwright), screenshots, and CSS/XPath/regex/adaptive extraction over the bus. |
| [`browser`](browser/) | Rust | Interactive Chromium sessions over CDP with console/network capture, a11y-tree snapshots with actionable refs, viewable screenshots, and DevTools element picking for the console UI. |
| [`worktree`](worktree/) | Rust | Git worktree lifecycle for parallel agents — `worktree::*` mint, claim, and track isolated worktrees per repo, emit six lifecycle trigger types, and land branches back through a per-repo FIFO queue (rebase, test gate, ff-only merge). |

## SDK

Workers target the `iii-sdk` package on each ecosystem:

- Rust — [crates.io/crates/iii-sdk](https://crates.io/crates/iii-sdk)
- Node — [npmjs.com/package/iii-sdk](https://www.npmjs.com/package/iii-sdk)
- Python — [pypi.org/project/iii-sdk](https://pypi.org/project/iii-sdk/)

Each module pins its exact version in its own manifest (`Cargo.toml`,
`package.json`, or `pyproject.toml`).

## Build

Rust workers:

```bash
cd <worker>
cargo build --release
```

Node/Python workers follow the standard `npm install` / `pip install -e .`
flow — see each module's README for specifics. `harness` is a pnpm
monorepo (`pnpm install && pnpm build`).

## Binary releases

Rust workers ship as standalone binaries — see the modules table above —
and are released via GitHub Actions:

1. Trigger the **Create Tag** workflow (Actions tab) — pick a worker, bump
   type (`patch`/`minor`/`major`), and a registry tag (`latest` / `next`).
2. A tag of the form `<worker>/v<X.Y.Z>` is pushed to `main`, with the
   registry tag embedded in the tag's annotated message.
3. The unified **Release** workflow fires on the tag, cross-compiles
   binaries for up to 9 targets (Linux gnu/musl, macOS x86_64 + aarch64,
   Windows x86_64/i686/aarch64, armv7), uploads them to a GitHub Release
   with SHA-256 checksums, and calls `POST /publish` on the workers
   registry API.

Targets per build (Windows targets are skipped on POSIX-only workers such
as `shell`):

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

Workers are discovered through the workers registry API at
`https://api.workers.iii.dev`. Each release publishes a manifest entry
declaring the worker kind (`binary` / container image), supported targets,
download URLs, and the worker's collected function + trigger interface.
`iii worker add <name>` queries this API to locate the right asset for the
host.

## Add a new worker

Start with [`docs/sops/new-worker.md`](docs/sops/new-worker.md) — the
cross-cutting checklist (naming, required files, CI gates, release wiring).
For the inside of a Rust `deploy: binary` worker, continue with
[`docs/sops/binary-worker.md`](docs/sops/binary-worker.md). Each worker ships
a consumer `README.md` per the [`worker-readme.md`](worker-readme.md)
contract (install via `iii worker add`, quickstart, configuration).
[`docs/README.md`](docs/README.md) indexes all shared docs.

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

Full reference (discovery buckets, interface boot smoke, e2e workflows):
[`docs/architecture/testing-and-ci.md`](docs/architecture/testing-and-ci.md).

## CD

Releases are cut manually via the **Create Tag** workflow
([`.github/workflows/create-tag.yml`](.github/workflows/create-tag.yml)) —
pick a worker, a bump type, and a registry tag (`latest` / `next`). The
resulting `<worker>/v<X.Y.Z>` tag drives a single dispatcher
([`.github/workflows/release.yml`](.github/workflows/release.yml)) that:

1. Routes on `deploy` from `iii.worker.yaml`:
   - `binary` → cross-compile via
     [`_rust-binary.yml`](.github/workflows/_rust-binary.yml).
   - `image` → multi-arch image to `ghcr.io/<owner>/<worker>` via
     [`_container.yml`](.github/workflows/_container.yml).
   - `bundle` → single-file archive via
     [`_bundle.yml`](.github/workflows/_bundle.yml).
2. Calls `POST /publish` against the workers registry API via
   [`_publish-registry.yml`](.github/workflows/_publish-registry.yml).

Step-by-step (variants, troubleshooting, rollback):
[`docs/sops/release.md`](docs/sops/release.md).

## License

Apache 2.0 — see [`LICENSE`](LICENSE).
