# session-inbox

Per-session inbox on the iii bus. Producers push items keyed by
`(session_id, name)`; consumers drain them atomically at session boundaries
(typically between agent turns). Items live in iii state under
`session/<id>/<name>` so they survive worker restart.

This is **not** a job queue (compare the engine’s queue builtin with async
consumers, retries, and DLQ). `session-inbox` is a pull-mode list: producers
fire-and-forget, consumers drain when they decide.

## Install

```bash
iii worker add session-inbox
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
            function_id: "session-inbox::push".into(),
            payload: json!({
                "session_id": "s1",
                "name": "steering",
                "item": { "role": "user", "content": "hello" },
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

`session-inbox::drain` accepts `{ "session_id", "name" }` and returns
`{ "items": [...] }` (atomic read+clear). `session-inbox::peek` reads without
mutating.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134 # used when --url / III_URL is unset
state_scope: agent                # iii state scope for inbox keys
```

Other defaults live in [`src/config.rs`](src/config.rs).

## Migration notes

Function ids were previously registered as `inbox::push`, `inbox::drain`, and
`inbox::peek`. They are now `session-inbox::push`, `session-inbox::drain`, and
`session-inbox::peek`. Payloads are unchanged.
