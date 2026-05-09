```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let started = worker
        .trigger(TriggerRequest {
            function_id: "run::start".into(),
            payload: json!({
                "session_id": "my-session-01",
                "provider": "anthropic",
                "model": "claude-sonnet-4-5",
                "system_prompt": "You are a helpful assistant.",
                "messages": [
                    { "role": "user", "content": [{ "type": "text", "text": "What is 2 + 2?" }] }
                ],
                "max_turns": 5,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("session started: {}", started["session_id"]);
    Ok(())
}
```

```typescript
import { registerWorker } from 'iii-sdk'

const worker = registerWorker('ws://localhost:49134')

const started = await worker.trigger({
  function_id: 'run::start',
  payload: {
    session_id: 'my-session-01',
    provider: 'anthropic',
    model: 'claude-sonnet-4-5',
    system_prompt: 'You are a helpful assistant.',
    messages: [
      { role: 'user', content: [{ type: 'text', text: 'What is 2 + 2?' }] },
    ],
    max_turns: 5,
  },
})

console.log('session started:', started.session_id)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

started = worker.trigger({
    "function_id": "run::start",
    "payload": {
        "session_id": "my-session-01",
        "provider": "anthropic",
        "model": "claude-sonnet-4-5",
        "system_prompt": "You are a helpful assistant.",
        "messages": [
            {"role": "user", "content": [{"type": "text", "text": "What is 2 + 2?"}]}
        ],
        "max_turns": 5,
    },
})

print("session started:", started["session_id"])
```

The example calls `run::start`, which returns immediately with the session id while the orchestrator drives the run asynchronously. For tests and sub-agent flows, `run::start_and_wait` accepts the same payload plus a `timeout_ms` and blocks until the session terminates.
