```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "shell::exec".into(),
            payload: json!({
                "command": "echo",
                "args": ["hello"],
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
  function_id: 'shell::exec',
  payload: { command: 'echo', args: ['hello'] },
})

console.log(result)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

result = worker.trigger({
    "function_id": "shell::exec",
    "payload": {"command": "echo", "args": ["hello"]},
})

print(result)
```

The example calls `shell::exec` on the host. The same payload retargets at a microVM with `target: { "kind": "sandbox", "sandbox_id": "<uuid>" }`. Other entry points: `shell::exec_bg`, `shell::status`, `shell::kill`, `shell::list`, plus the `shell::fs::*` family (`ls`, `stat`, `read`, `write`, `grep`, `sed`, `mkdir`, `rm`, `chmod`, `mv`).
