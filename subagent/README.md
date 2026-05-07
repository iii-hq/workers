# subagent

Spawn child agent sessions on the iii bus under `subagent::start`. Wraps
`run::start_and_wait` so a parent agent can run a sub-task in an isolated
session and consume the result inline. Install also pulls `turn-orchestrator`
(see `dependencies` in `iii.worker.yaml`) so `run::*` is available.

## Install

```bash
iii worker add subagent
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

Call `subagent::start` with `prompt`, `provider`, and `model`. Optional fields:
`system_prompt`, `parent_session_id`, `max_subagent_depth`.

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "subagent::start".into(),
            payload: json!({
                "prompt": "Summarize this in one line.",
                "provider": "openai",
                "model": "gpt-4o-mini",
            }),
            action: None,
            timeout_ms: Some(600_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Other keys (and their defaults) live in [`src/config.rs`](src/config.rs).

## Configuration

Operator defaults ship in committed `config.yaml` and `--config`:

```yaml
default_system_prompt: "You are a focused sub-agent. Answer the parent's subtask concisely and stop."
trigger_timeout_ms: 600000          # forwarded to run::start_and_wait
default_max_subagent_depth: 3       # nesting cap when caller omits max_subagent_depth
```
