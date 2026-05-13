```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let created = worker
        .trigger(TriggerRequest {
            function_id: "session-tree::create".into(),
            payload: json!({ "display_name": "demo" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;
    let session_id = created["session_id"].as_str().unwrap();

    let appended = worker
        .trigger(TriggerRequest {
            function_id: "session-tree::append".into(),
            payload: json!({
                "session_id": session_id,
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "hello" }],
                    "timestamp": 0,
                },
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("entry_id = {:?}", appended["entry_id"]);
    Ok(())
}
```

```typescript
import { registerWorker } from 'iii-sdk'

const worker = registerWorker('ws://localhost:49134')

const { session_id } = await worker.trigger({
  function_id: 'session-tree::create',
  payload: { display_name: 'demo' },
})

const appended = await worker.trigger({
  function_id: 'session-tree::append',
  payload: {
    session_id,
    message: {
      role: 'user',
      content: [{ type: 'text', text: 'hello' }],
      timestamp: 0,
    },
  },
})

console.log(appended.entry_id)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

created = worker.trigger({
    "function_id": "session-tree::create",
    "payload": {"display_name": "demo"},
})
session_id = created["session_id"]

appended = worker.trigger({
    "function_id": "session-tree::append",
    "payload": {
        "session_id": session_id,
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": "hello"}],
            "timestamp": 0,
        },
    },
})

print(appended["entry_id"])
```

The example creates a session and appends a single user message. Other entry points: `session-tree::messages`, `session-tree::tree`, `session-tree::fork`, `session-tree::clone`, `session-tree::compact`, and `session-tree::export_html`.
