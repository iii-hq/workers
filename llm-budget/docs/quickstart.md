```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "budget::create".into(),
            payload: json!({
                "workspace_id": "ws-demo",
                "name": "default",
                "ceiling_usd": 500.0,
                "period": "month",
            }),
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
  function_id: 'budget::create',
  payload: {
    workspace_id: 'ws-demo',
    name: 'default',
    ceiling_usd: 500.0,
    period: 'month',
  },
})

console.log(result)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

result = worker.trigger({
    "function_id": "budget::create",
    "payload": {
        "workspace_id": "ws-demo",
        "name": "default",
        "ceiling_usd": 500.0,
        "period": "month",
    },
})

print(result)
```

`budget::create` returns a `budget_id`. Subsequent calls (`budget::check`, `budget::record`, `budget::usage`, …) take that id as their primary handle.
