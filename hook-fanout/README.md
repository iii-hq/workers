# hook-fanout

Reusable publish-collect primitive on the iii bus. Publishes a hook topic,
collects replies from subscribers within a deadline, then merges them by a
caller-selected `merge_rule` and returns the merged value.

## Install

```bash
iii worker add hook-fanout
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
            function_id: "hook-fanout::publish_collect".into(),
            payload: json!({
                "topic": "agent::before_tool_call",
                "payload": { "tool_call": { "id": "t1" } },
                "merge_rule": "first_block_wins",
                "timeout_ms": 5000,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Response shape: `{ "event_id", "replies", "merged" }` (see source for merge rules).

## Configuration

```yaml
default_timeout_ms: 10000 # used when the caller omits timeout_ms on the payload
min_timeout_ms: 50      # floor applied to the effective wait window
poll_interval_ms: 25    # how often we poll the hook reply stream
```

Other defaults and behavior live in [`src/config.rs`](src/config.rs).

## Migration notes

Function id was `hooks::publish_collect`; it is now `hook-fanout::publish_collect`
(same payload and semantics).
