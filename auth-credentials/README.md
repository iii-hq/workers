# auth-credentials

Provider credential vault on the iii bus. It stores API keys and OAuth tokens so
providers and agents read credentials through `auth::*` without handling raw
secrets in every caller.

## Install

```bash
iii worker add auth-credentials
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "auth::get_token".into(),
            payload: json!({ "provider": "anthropic" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Other entry points: `auth::set_token`, `auth::delete_token`, `auth::list_providers`, and `auth::status`.

## Configuration

```yaml
engine_url: "ws://127.0.0.1:49134"   # override with --url or III_URL
store: iii_state                     # iii_state (durable) or memory (ephemeral)
```

`store` defaults to durable storage via iii-state. Use `memory` for tests or
ephemeral runs. You can still override with `AUTH_CREDENTIALS_STORE=memory` or
`iii_state` for backward compatibility.

Other defaults live in [`src/config.rs`](src/config.rs).

## Companion workers

To register markdown skills with the engine, the [skills](../skills) worker
should be running so this worker’s `skills::register` handshake succeeds.

```bash
iii worker add skills
```
