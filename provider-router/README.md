# provider-router

The `router::*` surfaces on the iii bus for assistant streaming and session
helpers: routing to `provider::<name>::complete` (with optional `router::decide`
from `llm-router` when installed), abort via `state::set`, and steering /
follow-up messages through `session-inbox::push`. The turn loop itself lives in
`turn-orchestrator`; this worker only registers handlers and HTTP triggers.

`iii worker add` pulls declared worker dependencies (`session-inbox`,
`llm-budget`). Function ids use `::` throughout (see
[`src/register.rs`](src/register.rs)).

## Install

```bash
iii worker add provider-router
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
            function_id: "router::abort".into(),
            payload: json!({ "session_id": "my-session" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Streaming and budget checks use `router::stream_assistant` with the payload
shape your orchestrator sends; see in-repo callers for full examples.

## Configuration

Committed default for local runs (`--config ./config.yaml`):

```yaml
engine_url: ws://127.0.0.1:49134   # iii engine WebSocket URL
```

CLI overrides: `--url` / `III_URL` wins when set; otherwise `engine_url` from
config is used. Other defaults live in [`src/config.rs`](src/config.rs).

## Custom trigger types

This worker registers HTTP triggers (POST) for `router::push_steering`,
`router::abort`, and `router::push_followup` at `agent/{session_id}/steer`,
`agent/{session_id}/abort`, and `agent/{session_id}/follow_up` respectively,
bound to those same function ids.
