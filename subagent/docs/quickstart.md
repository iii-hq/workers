```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "subagent::start".into(),
            payload: json!({
                "prompt": "Summarise this in one line.",
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

```typescript
import { registerWorker } from 'iii-sdk'

const worker = registerWorker('ws://localhost:49134')

const result = await worker.trigger({
  function_id: 'subagent::start',
  payload: {
    prompt: 'Summarise this in one line.',
    provider: 'openai',
    model: 'gpt-4o-mini',
  },
})

console.log(result)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

result = worker.trigger({
    "function_id": "subagent::start",
    "payload": {
        "prompt": "Summarise this in one line.",
        "provider": "openai",
        "model": "gpt-4o-mini",
    },
})

print(result)
```

The example calls `subagent::start` with the minimum payload. Optional fields: `system_prompt`, `parent_session_id`, and `max_subagent_depth`.
