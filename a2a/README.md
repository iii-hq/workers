# iii-a2a

A2A (Agent-to-Agent) JSON-RPC surface for the iii engine. Registers two
HTTP triggers on the engine:

- `GET /.well-known/agent-card.json` — discovery / capability card
- `POST /a2a` — JSON-RPC dispatch (`message/send`, `tasks/get`,
  `tasks/cancel`, `tasks/list`, `message/stream`, `tasks/resubscribe`)

## Access control

Function exposure is governed by **iii-sdk RBAC** at `iii-worker-manager`. This worker is a protocol transport — the engine decides who can see and call what.

Configure in `config.yaml`:

```yaml
workers:
  - name: iii-worker-manager
    config:
      rbac:
        auth_function_id: myproject::auth
        expose_functions:
          - match("api::*")
          - match("*::public")
          - metadata:
              public: true
  - name: iii-mcp    # or iii-a2a
```

See https://iii.dev/docs/how-to/worker-rbac.md for the RBAC surface.

### Breaking change (v0.4)

- `--expose-all` and `--tier` removed. Port policy to `auth_function_id`.
- `mcp.expose` / `a2a.expose` metadata flags no longer consulted.
- See `examples/default-secure-auth.rs` for a drop-in replacement.

### Engine introspection

For the use case the old `engine::*` hard floor hid, use the `introspect` worker (`iii worker add introspect`). It exposes `introspect::functions`, `introspect::topology`, `introspect::health`, etc. — agents call those through MCP/A2A without leaking raw engine internals.

### CLI flags

```text
--engine-url <URL>   WebSocket URL of the iii engine (default ws://localhost:49134)
--base-url <URL>     Public origin advertised in the agent card. The card is served
                     at <base-url>/a2a. Default: http://localhost:3111
--rbac-tag <TAG>     Forward an `x-iii-rbac-tag` header on the worker WebSocket
                     upgrade. iii-worker-manager's `auth_function_id` reads
                     this tag to apply policy.
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

### 2. Register a test function

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
# Agent card — skills are governed by iii-worker-manager RBAC.
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

### 5. Validate with any A2A client

Any AI agent runtime that speaks A2A JSON-RPC 0.3 works. Point it at
`http://<your-host>:3111/.well-known/agent-card.json` (or the
`--base-url` you configured) and exercise `message/send`.

## Function resolution inside `message/send`

1. **Data part** with `{ "function_id": "foo::bar", "payload": {...} }` —
   direct invocation.
2. **Text part** beginning with `foo::bar <json>` — first token is the
   function id; rest is parsed as the payload.
3. Anything else — task fails with "No function_id found".

Before dispatch, the resolved `function_id` is rejected if it is a
protocol entry point (`mcp::*` / `a2a::*`). Everything else is delegated
to iii-worker-manager RBAC.

## Task model

Tasks are persisted in engine state (`a2a:tasks` scope). Terminal states
(`Completed`, `Canceled`, `Failed`, `Rejected`) are idempotent — repeated
`message/send` on the same `task_id` returns the stored result rather
than re-invoking the function. Mid-flight cancel is honoured: if a
`tasks/cancel` lands while a function call is in progress, the result is
discarded and the task keeps its `Canceled` state.

## Streaming

`message/stream` and `tasks/resubscribe` emit Server-Sent Events
(`text/event-stream`) on the same `POST /a2a` endpoint. Each event is a
[`TaskStatusUpdateEvent`](https://a2aproject.dev/spec/v0.3) or
`TaskArtifactUpdateEvent` framed as
`id: <n>\nevent: <kind>\ndata: <json>\n\n`.

`message/stream` walks the task through `submitted → working → artifact
→ completed` (or `→ failed`). The `submitted` frame is wire-only — it
matches the A2A spec sequence, but the persisted task transitions
straight to `working`.

`tasks/resubscribe` latches onto an in-flight task, replays one frame
with the current state, and forwards subsequent broadcasts until the
task reaches a terminal state. Resubscribing to a terminal task emits a
single `final: true` frame and closes.

Cross-method propagation: a sync `message/send` or `tasks/cancel`
broadcasts through the same registry, so concurrent stream subscribers
see live transitions instead of needing to poll `tasks/get`.

```bash
# message/stream — start a task and watch SSE frames
curl --no-buffer -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{
    "jsonrpc":"2.0","id":"s1","method":"message/stream",
    "params":{"message":{
      "messageId":"m1","role":"user",
      "parts":[{"data":{"function_id":"pricing::quote","payload":{}}}]
    }}
  }'

# tasks/resubscribe — latch onto an existing in-flight task
curl --no-buffer -sX POST http://127.0.0.1:3111/a2a \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":"r1","method":"tasks/resubscribe",
       "params":{"id":"<task-id>"}}'
```

Internally the worker constructs a `ChannelWriter` via
`ChannelWriter::new(iii.address(), &channel_ref)` (iii-sdk 0.11.3 API —
no `connect()` step, the WebSocket opens lazily on first send/write).

## Not implemented

- Push notifications

Returns JSON-RPC error `-32003`.
