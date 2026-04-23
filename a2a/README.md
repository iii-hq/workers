# iii-a2a

A2A (Agent-to-Agent) JSON-RPC surface for the iii engine. Registers two
HTTP triggers on the engine:

- `GET /.well-known/agent-card.json` — discovery / capability card
- `POST /a2a` — JSON-RPC dispatch (`message/send`, `tasks/get`,
  `tasks/cancel`, `tasks/list`)

## Exposure model

Functions are **invisible to remote agents by default**. Authors opt in
per-function. Mirrors the iii-mcp design — same metadata keys, same hard
floor, same optional tier filter.

### Opt-in metadata

```rust
// Rust worker
iii.register_function_with(
    RegisterFunctionMessage {
        id: "pricing::quote".into(),
        description: Some("Quote a price for SKU + quantity".into()),
        metadata: Some(json!({
            "a2a.expose": true,
            "a2a.tier": "partner",  // optional — free-form string
        })),
        ..Default::default()
    },
    handler,
);
```

```ts
// Node worker
iii.registerFunction("pricing::quote", handler, {
  metadata: { "a2a.expose": true, "a2a.tier": "partner" },
})
```

```python
# Python worker
iii.register_function(
    "pricing::quote",
    handler,
    metadata={"a2a.expose": True, "a2a.tier": "partner"},
)
```

### Hard floor (never exposed)

Even with `--expose-all`, these namespaces never appear as agent-card
skills and cannot be invoked via `message/send`:

| Prefix | Why |
|---|---|
| `engine::*` | iii engine internals |
| `state::*` | KV plumbing |
| `stream::*` | channel plumbing |
| `iii.*` / `iii::*` | SDK callbacks and namespaced SDK APIs |
| `mcp::*` | sibling protocol worker's entry point |
| `a2a::*` | this worker's own JSON-RPC entry |

### CLI flags

```text
--engine-url <URL>   WebSocket URL of the iii engine (default ws://localhost:49134)
--expose-all         Ignore the a2a.expose gate (dev only). Hard floor still applies.
--tier <name>        Show only functions whose a2a.tier metadata equals <name>.
                     E.g. `--tier partner` for B2B surfaces, `--tier public` for open.
--base-url <URL>     Public origin advertised in the agent card. The card is served
                     at <base-url>/a2a. Default: http://localhost:3111
--debug              Verbose logging
```

## Local testing

A2A has no standard client inspector like MCP does. Use curl. Full smoke
path from a clean machine, assuming `iii-sdk` v0.11.3 engine installed:

### 1. Start the engine

Minimal `config.yaml`:

```yaml
workers:
  - name: iii-worker-manager
  - name: iii-http
    config:
      host: 127.0.0.1
      port: 3111
  - name: iii-state
```

```bash
iii --no-update-check
```

### 2. Register a test function tagged `a2a.expose: true`

```rust
use iii_sdk::{register_worker, InitOptions, RegisterFunctionMessage};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "pricing::quote".into(),
            description: Some("Quote a price".into()),
            metadata: Some(json!({ "a2a.expose": true })),
            ..Default::default()
        },
        |_input: Value| async move { Ok(json!({"price": 42})) },
    );

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

### 3. Start iii-a2a

```bash
cargo run --release -p iii-a2a
# or: ./target/release/iii-a2a --base-url http://127.0.0.1:3111
```

Registers `GET /.well-known/agent-card.json` and `POST /a2a`.

### 4. Smoke path

```bash
# Agent card — exposed skills only, hard floor filters engine::/state::/etc.
curl -s http://127.0.0.1:3111/.well-known/agent-card.json \
  | jq '{name, skills: [.skills[] | {id, description}]}'

# message/send with data part (direct invocation)
curl -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc":"2.0","id":"t1","method":"message/send",
    "params":{"message":{
      "messageId":"m1","role":"user",
      "parts":[{"data":{"function_id":"pricing::quote","payload":{}}}]
    }}
  }' | jq '.result.task | {state: .status.state, artifact: .artifacts[0].parts[0].text}'

# message/send with text shorthand ("function_id <json>")
curl -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc":"2.0","id":"t2","method":"message/send",
    "params":{"message":{
      "messageId":"m2","role":"user",
      "parts":[{"text":"pricing::quote {}"}]
    }}
  }' | jq '.result.task.status.state'

# tasks/get — retrieve the stored task
curl -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":"t3","method":"tasks/get","params":{"id":"<paste task.id from t1>"}}' \
  | jq '.result.task.status.state'
```

### 5. Verify each gate

```bash
# Hidden function (no a2a.expose) → state:"failed", distinct rejection
curl -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc":"2.0","id":"g1","method":"message/send",
    "params":{"message":{
      "messageId":"x","role":"user",
      "parts":[{"data":{"function_id":"demo::hidden","payload":{}}}]
    }}
  }' | jq '.result.task.status.message.parts[0].text'

# Infra prefix → "in the iii-engine internal namespace" message
curl -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc":"2.0","id":"g2","method":"message/send",
    "params":{"message":{
      "messageId":"x","role":"user",
      "parts":[{"data":{"function_id":"state::set","payload":{}}}]
    }}
  }' | jq '.result.task.status.message.parts[0].text'

# Tier filter: restart iii-a2a with --tier partner, function has a2a.tier="partner"
./target/release/iii-a2a --tier partner &
curl -s http://127.0.0.1:3111/.well-known/agent-card.json | jq '.skills[].id'
```

### 6. Validate with any A2A client

Any AI agent runtime that speaks A2A JSON-RPC 0.3 works. Point it at
`http://<your-host>:3111/.well-known/agent-card.json` (or the
`--base-url` you configured) and exercise `message/send`.

## Function resolution inside `message/send`

1. **Data part** with `{ "function_id": "foo::bar", "payload": {...} }` —
   direct invocation.
2. **Text part** beginning with `foo::bar <json>` — first token is the
   function id; rest is parsed as the payload.
3. Anything else — task fails with "No function_id found".

Before dispatch, the resolved `function_id` is re-checked against the
same exposure gate as `tools/list`. Hard-floor namespaces reject
unconditionally.

## Task model

Tasks are persisted in engine state (`a2a:tasks` scope). Terminal states
(`Completed`, `Canceled`, `Failed`, `Rejected`) are idempotent — repeated
`message/send` on the same `task_id` returns the stored result rather
than re-invoking the function. Mid-flight cancel is honoured: if a
`tasks/cancel` lands while a function call is in progress, the result is
discarded and the task keeps its `Canceled` state.

## Not implemented

- `message/stream`, `tasks/resubscribe` (streaming)
- Push notifications

Returns JSON-RPC errors `-32004` and `-32003` respectively.

## Example: multi-tier partner API

One worker, three audiences.

```rust
iii.register_function_with(
    RegisterFunctionMessage {
        id: "pricing::public_quote".into(),
        metadata: Some(json!({ "a2a.expose": true, "a2a.tier": "public" })),
        ..Default::default()
    },
    public_quote_handler,
);

iii.register_function_with(
    RegisterFunctionMessage {
        id: "pricing::partner_quote".into(),
        metadata: Some(json!({ "a2a.expose": true, "a2a.tier": "partner" })),
        ..Default::default()
    },
    partner_quote_handler,
);

iii.register_function_with(
    RegisterFunctionMessage {
        id: "pricing::internal_cost".into(),
        metadata: Some(json!({ "a2a.expose": true, "a2a.tier": "ops" })),
        ..Default::default()
    },
    internal_cost_handler,
);
```

Three a2a workers behind three different auth proxies, each with a
different `--tier`, advertise three different agent cards.
