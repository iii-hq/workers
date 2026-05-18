# iii workers

Workers for the [iii engine](https://github.com/iii-hq/iii). Each top-level
directory is a self-contained worker module: a process that connects to the
engine over WebSocket, registers functions + triggers, and does something
useful.

Workers are installed via `iii worker add <name>`, which resolves the matching
asset for the host from the workers registry API.

## Modules

| Worker | Kind | Summary |
|---|---|---|
| [`acp`](acp/) | Rust | Agent Client Protocol surface — stdio JSON-RPC, exposes iii agents as ACP sessions. |
| [`harness-node`](harness-node/) | Node | TS port of the iii harness stack — bundles `harness`, `turn-orchestrator`, `approval-gate`, `session`, `hook-fanout`, `auth-credentials`, `models-catalog`, `provider-anthropic`, `provider-openai`, `llm-budget`, and `context-compaction` as one pnpm monorepo. See [`harness-node/README.md`](harness-node/README.md). |
| [`iii-database`](iii-database/) | Rust | PostgreSQL, MySQL, and SQLite client — query, execute, transactions, prepared statements, and change feeds. |
| [`iii-directory`](iii-directory/) | Rust | Engine introspection (functions / triggers / workers), workers-registry proxy, and filesystem-backed skill + prompt reader. |
| [`iii-lsp`](iii-lsp/) | Rust | Language Server for iii function ids, trigger configs, and worker discovery. Autocomplete / hover across JS/TS, Python, Rust. |
| [`iii-lsp-vscode`](iii-lsp-vscode/) | Node | VS Code extension that embeds `iii-lsp`. |
| [`image-resize`](image-resize/) | Rust | Image resize via channel I/O — JPEG/PNG/WebP with EXIF auto-orient, scale-to-fit / crop-to-fit. |
| [`llm-budget`](llm-budget/) | Rust | Workspace + agent LLM spend caps with alerts, forecast, and period rollover under `budget::*`. |
| [`mcp`](mcp/) | Rust | MCP 2025-06-18 Streamable HTTP bridge — exposes iii functions tagged `mcp.expose` as MCP tools. |
| [`shell`](shell/) | Rust | Unix shell + filesystem worker — `shell::exec` with allowlist/denylist/timeout/output caps and background jobs; `fs::ls`/`stat`/`mkdir`/`rm`/`chmod`/`mv`/`grep`/`sed`/`read`/`write` with host jail, denylist, and size caps. |
| [`storage`](storage/) | Rust | S3-compatible object storage across AWS S3, GCS, Cloudflare R2, and a managed local rustfs backend. Streamed uploads, presigned URLs, and object change triggers. |
| [`todo-worker`](todo-worker/) | Node | Quickstart CRUD todo worker using the Node iii SDK. |
| [`todo-worker-python`](todo-worker-python/) | Python | Quickstart CRUD todo worker using the Python iii SDK. |

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
flow — see each module's README for specifics. `harness-node` is a pnpm
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

[`binary-worker.md`](binary-worker.md) is the deep dive for the Rust binary
scaffold (manifest, config, layout, function contract).
[`worker-readme.md`](worker-readme.md) covers the README contract every
worker shares (install via `iii worker add`, run via `iii start`, how-to
over reference).

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
   - `binary` → cross-compile via
     [`_rust-binary.yml`](.github/workflows/_rust-binary.yml).
   - `image` → multi-arch image to `ghcr.io/<owner>/<worker>` via
     [`_container.yml`](.github/workflows/_container.yml).
2. Calls `POST /publish` against the workers registry API via
   [`_publish-registry.yml`](.github/workflows/_publish-registry.yml).

## License

Apache 2.0 — see [`LICENSE`](LICENSE).
