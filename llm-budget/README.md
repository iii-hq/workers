# llm-budget

Workspace + agent LLM spend caps on the iii bus: `budget::*` functions for budget CRUD, spend checks/recording, alerts, usage rollups, forecast, enforcement, exemptions, and pause/resume — with optional companion skills nested under `llm-budget/*`.

## Install

```bash
iii worker add llm-budget
```

`iii worker add` fetches the binary, writes a config block into `~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

Register `budget::*` through the iii SDK:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "budget::create".into(),
            payload: json!({
                "workspace_id": "ws-demo",
                "name": "default",
                "ceiling_usd": 500.0,
                "period": "month",
            }),
            action: None,
            timeout_ms: Some(5000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Further calls use the returned `budget_id` with `"budget_id": "<uuid>"`, e.g. `budget::check`, `budget::record`, `budget::usage`. Exact payloads match the bundled skills under `/skills/` in this crate.

## Configuration

Committed defaults are loaded with `--config ./config.yaml` (the binary default):

```yaml
skills_trigger_timeout_ms: 5000           # iii trigger timeout when registering skills::* 
skills_handshake_deadline_secs: 180       # retry skills::register until this deadline
```

Additional keys live in [`src/config.rs`](src/config.rs).

State is persisted via built-in iii `state::*` helpers (scope `budgets`); survives restarts when the engine backend is durable.
