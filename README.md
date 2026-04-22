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
| [`a2a`](a2a/) | Rust | A2A (Agent-to-Agent) JSON-RPC protocol worker. Publishes an agent card, routes `message/send` to exposed functions gated by `a2a.expose` metadata. |
| [`agent`](agent/) | Rust | Chat + planning agent that discovers other workers' functions at runtime and drives them via `iii.trigger()`. |
| [`coding`](coding/) | Rust | Code-generation / refactor assistant. |
| [`eval`](eval/) | Rust | OTel span ingestion, latency percentiles, baseline + drift detection, system health score. |
| [`experiment`](experiment/) | Rust | A/B experiment tracking with weighted variant sampling and outcome aggregation. |
| [`guardrails`](guardrails/) | Rust | Input/output guardrails — PII detection, prompt-injection filters, length enforcement. |
| [`iii-lsp`](iii-lsp/) | Rust | Language Server for iii function ids, trigger configs, and worker discovery. Autocomplete/hover across JS/TS, Python, Rust. |
| [`iii-lsp-vscode`](iii-lsp-vscode/) | Node | VS Code extension that embeds `iii-lsp`. |
| [`image-resize`](image-resize/) | Rust | Image resize via channel I/O. JPEG/PNG/WebP with EXIF auto-orient, scale-to-fit / crop-to-fit. |
| [`introspect`](introspect/) | Rust | Live topology, trace walks, Mermaid diagrams, and per-function explain output. |
| [`llm-router`](llm-router/) | Rust | Unopinionated LLM routing brain — policies, classifiers, A/B tests, health + budget aware. Sits in front of any gateway. |
| [`mcp`](mcp/) | Rust | Model Context Protocol surface — stdio + HTTP JSON-RPC, exposes iii functions tagged `mcp.expose` as MCP tools. |
| [`proof`](proof/) | Node | AI-driven browser testing — diffs changes, generates test plans, drives Playwright. |
| [`shell`](shell/) | Rust | Sandboxed-ish Unix shell execution. Allowlist + denylist, timeout + output caps, background jobs. |
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

Rust workers that ship as standalone binaries (`a2a`, `agent`, `coding`,
`eval`, `experiment`, `guardrails`, `iii-lsp`, `image-resize`, `introspect`,
`llm-router`, `mcp`, `shell`) are released via GitHub Actions:

1. Trigger the **Create Tag** workflow (Actions tab) — pick a worker, bump
   type (`patch`/`minor`/`major`), and optional prerelease label.
2. A tag of the form `<worker>/v<X.Y.Z>` is pushed to `main`.
3. The matching **Release** workflow fires on the tag, builds cross-compiled
   binaries for 9 targets (Linux gnu/musl, macOS x86_64 + aarch64, Windows
   x86_64/i686/aarch64, armv7), and uploads them to a GitHub Release with
   SHA-256 checksums.

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

## CI

Pull requests trigger per-worker `cargo fmt --check`, `cargo clippy
--all-targets --all-features -- -D warnings`, and `cargo test --all-features`
for Rust modules that changed (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

## License

Apache 2.0 — see [`LICENSE`](LICENSE).
