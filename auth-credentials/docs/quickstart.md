```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let worker = register_worker("ws://localhost:49134", InitOptions::default());

    let result = worker
        .trigger(TriggerRequest {
            function_id: "auth::get_token".into(),
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
  function_id: 'auth::get_token',
  payload: { provider: 'anthropic' },
})

console.log(result)
```

```python
from iii import register_worker

worker = register_worker("ws://localhost:49134")

result = worker.trigger({
    "function_id": "auth::get_token",
    "payload": {"provider": "anthropic"},
})

print(result)
```

The example calls `auth::get_token`. Other entry points: `auth::set_token`, `auth::delete_token`, `auth::list_providers`, and `auth::status`.

## Testing Against a Real Engine

`store: iii_state` requires the engine to expose `state::get`, `state::set`,
`state::delete`, and `state::list`. For a minimal local engine, start only the
state worker:

```bash
iii --config auth-credentials/tests/e2e/engine-state.yaml --no-update-check
```

Then run the live worker tests from another shell:

```bash
IIITEST_ENGINE_URL=ws://127.0.0.1:49134 \
  cargo test --manifest-path auth-credentials/Cargo.toml --all-features

IIITEST_ENGINE_URL=ws://127.0.0.1:49134 \
IIITEST_WORKER_BIN="$PWD/auth-credentials/target/debug/auth-credentials" \
  cargo test --manifest-path auth-credentials/Cargo.toml --test restart_e2e -- --ignored
```
