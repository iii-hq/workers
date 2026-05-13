```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "models::list".into(),
            payload: json!({ "provider": "anthropic" }),
            action: None,
            timeout_ms: Some(5_000),
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
  function_id: 'models::list',
  payload: { provider: 'anthropic' },
})

console.log(result)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

result = worker.trigger({
    "function_id": "models::list",
    "payload": {"provider": "anthropic"},
})

print(result)
```

The example calls `models::list` with a provider filter. Other entry points: `models::get`, `models::supports`, and `models::register`.
