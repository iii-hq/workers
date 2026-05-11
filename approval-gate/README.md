# approval-gate

Hooks `agent::before_function_call`, pauses matching tool calls when `approval_required` mentions their `function_id`, surfaces `approval_requested` on `agent::events/<session_id>`, blocks until the workspace operator calls `approval::resolve`, then streams `approval_resolved` (or auto-denies after `default_timeout_ms`).

## Install

```bash
iii worker add approval-gate
```

`iii worker add` fetches the binary, writes `config.yaml` defaults into `~/.iii/config.yaml`, and the engine starts the worker with the next `iii start`.

Install the [`skills`](../skills/README.md) worker alongside it if you rely on the bundled AI skill docs emitted at boot.

## Quickstart

Approval traffic is normally published by `provider-router`/`turn-orchestrator`. To confirm the gate is connected, target the registered handler (dev-only — production traffic comes from the orchestrated hook envelope):

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let envelope = serde_json::json!({
        "event_id": "evt-check",
        "reply_stream": "rs-check",
        "payload": {
            "session_id": "sess-dev",
            "function_call": {
                "id": "tc-dev",
                "function_id": "shell::filesystem::write",
                "arguments": {},
            },
            "approval_required": ["shell::filesystem::write"],
        }
    });

    let result = iii
        .trigger(TriggerRequest {
            function_id: "policy::approval_gate".into(),
            payload: envelope,
            action: None,
            timeout_ms: Some(240_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Release the latch with:

```rust
approval_gate::FN_RESOLVE // "approval::resolve"
```

using JSON `{ "session_id": "sess-dev", "function_call_id": "tc-dev", "decision": "allow" }`.

## Configuration

Committed defaults mirror the registry manifest (`iii.worker.yaml`):

```yaml
topic: agent::before_function_call   # subscribe trigger topic
approval_state_scope: approvals      # state:: scope for pending rows
default_timeout_ms: 300000           # auto deny after 5 minutes
```

Engines may nest the block under `config:` exactly like [`policy-denylist`](../policy-denylist/README.md). Overrides:

| Env | Meaning |
|---|---|
| `APPROVAL_GATE_TOPIC` | Overrides `topic` |
| `APPROVAL_GATE_STATE_SCOPE` | Overrides `approval_state_scope` |
| `APPROVAL_GATE_TIMEOUT_MS` | Overrides `default_timeout_ms` |

Other defaults and serde aliases live in [`src/config.rs`](src/config.rs).

## Registered surfaces

| Function | Role |
|---|---|
| `policy::approval_gate` | Subscriber body + `durable:subscriber` trigger on `topic`. |
| `approval::resolve` | Operator decision (`allow` / `deny`) for one pending `(session_id, tool_call)`. |
| `approval::list_pending` | Hydrate UI lists after reload (`{ "pending": [...] }`). |

## Runtime expectations

`state::*`, `stream::set`, and the configured hook publisher must exist on the bus. Without `provider-router` emitting `agent::before_function_call`, nothing reaches the subscriber even though registrations succeed.
