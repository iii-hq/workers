# harness

Meta-worker that composes the modular workers behind a runnable iii chat
surface and exposes the browser-facing HTTP bridge (`bridge::trigger`)
the bundled Vite/React UI talks to. The harness does
not own chat, agent, or provider logic — it registers a small set of
bus functions and expects peers such as
[`turn-orchestrator`](../turn-orchestrator),
[`provider-router`](../provider-router), shell tools, and related
workers to be installed alongside it. `iii worker add harness` pulls
the whole bundle in transitively.

## Install

```bash
iii worker add harness
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, resolves every transitive worker from
`iii.worker.yaml` `dependencies:`, and the engine starts the bundle on
the next `iii start`.

To back chat history with durable SQL storage instead of the bundled
in-memory `iii-state`, add the [`iii-database`](../iii-database) worker:

```bash
iii worker add iii-database
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

Forward an arbitrary bus call through the HTTP-oriented bridge (same
shape as `bridge::trigger` on the engine):

```rust
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

Registered functions:

| Function | Role |
|---|---|
| `harness::status` | Bundle name, version, and expected worker list (cheap liveness probe). |
| `bridge::trigger` | Forwards `{ function_id, payload }` to `iii.trigger`. HTTP: `POST bridge/trigger`. |

`bridge::trigger` is the browser's call-anything escape hatch — not
meant as an LLM tool.

## Configuration

```yaml
engine_url: "ws://127.0.0.1:49134"   # WebSocket URL when III_URL / --url are unset
```

Runtime flags:

- `--config` — path to this file (default `./config.yaml`; override with `III_HARNESS_CONFIG`).
- `--url` / `III_URL` — engine WebSocket URL; wins over `engine_url` in the file.
