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
| [`iii-lsp`](iii-lsp/) | Rust | Language Server for iii function ids, trigger configs, and worker discovery. Autocomplete/hover across JS/TS, Python, Rust. |
| [`iii-lsp-vscode`](iii-lsp-vscode/) | Node | VS Code extension that embeds `iii-lsp`. |
| [`image-resize`](image-resize/) | Rust | Image resize via channel I/O. JPEG/PNG/WebP with EXIF auto-orient, scale-to-fit / crop-to-fit. |
| [`mcp`](mcp/) | Rust | Model Context Protocol surface — stdio + HTTP JSON-RPC, exposes iii functions tagged `mcp.expose` as MCP tools. |
| [`proof`](proof/) | Node | AI-driven browser testing — diffs changes, generates test plans, drives Playwright. |
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

Rust workers that ship as standalone binaries (`iii-lsp`, `image-resize`,
`mcp`) are released via GitHub Actions:

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
