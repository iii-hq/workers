```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "directory::skills::download".into(),
            payload: json!({
                "worker": "hello-worker",
                "tag": "latest",
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

```typescript
import { registerWorker } from 'iii-sdk'

const worker = registerWorker('ws://localhost:49134')

const result = await worker.trigger({
  functionId: 'directory::skills::list',
  payload: {},
})

console.log(result)
```
