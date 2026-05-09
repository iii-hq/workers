<!-- partial-info: Body of the rendered README's `## Quickstart` section. README only — not used in skill.md. Aim for ≤30 lines of code; show one to three functions chosen for introductory value. -->

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "textstats::analyze".into(),
            payload: json!({ "text": "hello world\nlooks small" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

The example calls `textstats::analyze`. Other entry points: `textstats::diff` and `textstats::summarize`.
