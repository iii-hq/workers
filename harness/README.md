# harness

Meta-worker that composes fifteen modular workers into a runnable iii chat surface, exposes a browser-facing HTTP bridge (`bridge::trigger`, `bridge::events`), and ships a Vite/React UI that talks to the bus through it. The harness does not own chat, agent, or provider logic; it registers a small set of bus functions and expects peers such as `turn-orchestrator`, `provider-router`, shell tools, and related workers to be installed alongside it. Deeper layout and streams behavior are documented in [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Install

```bash
iii worker add harness
```

`iii worker add` fetches the binary, writes a config block into `~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

To register the harness skill bundle metadata with the bus (the worker does this automatically at boot when `skills` is available), ensure the [skills](../skills) worker is part of your stack:

```bash
iii worker add skills
```

## Quickstart

After `iii start`, probe the bundle and list expected runtime workers:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "harness::status".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Forward an arbitrary bus call through the HTTP-oriented bridge (same shape as `bridge::trigger` on the engine):

```rust
// function_id / payload match iii.trigger(...)
let result = iii
    .trigger(TriggerRequest {
        function_id: "bridge::trigger".into(),
        payload: json!({
            "function_id": "models::list",
            "payload": {},
        }),
        action: None,
        timeout_ms: Some(240_000),
    })
    .await?;
```

Registered functions (use `::` ids on the bus):

| Function | Role |
|---|---|
| `harness::status` | Bundle name, version, and expected worker list (cheap liveness probe). |
| `bridge::trigger` | Forwards `{ function_id, payload }` to `iii.trigger`. HTTP: `POST` `bridge/trigger`. |
| `bridge::events` | SSE-style tail of `agent::events` for a session. HTTP: `GET` `bridge/events`. |

`bridge::trigger` is not meant as an LLM tool — it is the browser’s call-anything escape hatch.

## Configuration

```yaml
# Default engine WebSocket URL when III_URL / --url are unset
engine_url: "ws://127.0.0.1:49134"
```

Other runtime flags:

- `--config` — path to this file (default `./config.yaml`; override with `III_HARNESS_CONFIG`).
- `--url` or `III_URL` — engine WebSocket URL; wins over `engine_url` in the file.

Registry-facing defaults also appear in `iii-harness --manifest` under `default_config`.

## Expected workers

`EXPECTED_WORKERS` (in [`src/lib.rs`](src/lib.rs)) is generated at build time
from the `dependencies:` block of [`iii.worker.yaml`](iii.worker.yaml) by
[`build.rs`](build.rs). Add or remove a worker by editing `iii.worker.yaml`
only — the Rust constant rebuilds automatically.

## Local demo stack

From a checkout, the [`Makefile`](Makefile) drives the engine.
`harness/config.yaml` and `harness/iii.lock` are generated (and
gitignored) — `make config` runs `iii worker add .`, which reads
[`iii.worker.yaml`](iii.worker.yaml) `dependencies:`, resolves transitives,
writes the lock, and appends the harness entry with its `worker_path:`.
The engine then spawns each worker via that worker's own
`iii.worker.yaml` `scripts.start` (`cargo run` for local Rust crates —
builds on demand). Per-worker overrides (e.g.,
[`iii-directory/config.yaml`](../iii-directory/config.yaml)) live in
each worker's own directory.

```bash
make all      # config + engine + verify
make config   # (re)generate harness/config.yaml + iii.lock via `iii worker add .`
make engine   # start iii in background, reading harness/config.yaml
make verify   # call harness::status + models::list
make web      # background Vite dev server on :5173 (no tmux)
make stop     # kill engine + web
make restart  # stop + engine + verify (does not regenerate config.yaml)
make logs W=engine   # tail engine log (W=web for vite)
make clean    # remove pids/logs/data + generated config.yaml
```

PIDs and logs default under `~/iii-harness-demo`. The engine runs
with `harness/` as its working directory, so `iii-state`, `iii-stream`,
and `iii-directory` write their stores under `harness/data/`.

Contributor commands (fmt, clippy, tests) for this crate live in [`binary-worker.md`](../binary-worker.md) §11; source layout notes are in [`ARCHITECTURE.md`](ARCHITECTURE.md).
