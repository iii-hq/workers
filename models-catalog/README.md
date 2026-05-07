# models-catalog

Model capabilities knowledge base on the iii bus. Models live under
`models:<provider>:<id>` in scope `models`; the embedded `data/models.json` seeds
state once when empty. Callers use `models::list`, `models::get`,
`models::supports`, and `models::register`.

## Install

```bash
iii worker add models-catalog
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "models::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

`models::get` takes `{ "provider": "…", "model_id": "…" }`. `models::supports`
takes `{ "provider", "model_id", "capability" }` (e.g. `"tools"`,
`"thinking:xhigh"`). `models::register` accepts a full model object.

## Configuration

```yaml
engine_url: "ws://127.0.0.1:49134" # overridden by CLI --url or env III_URL when set
state_request_timeout_ms: 5000 # state:: / internal bus triggers
skills_register_timeout_ms: 5000
skills_unregister_timeout_ms: 2000
```

Other defaults and parsing live in [`src/config.rs`](src/config.rs).
