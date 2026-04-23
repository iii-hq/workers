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
| `iii.*` | SDK callbacks |
| `a2a::*` | this worker's own JSON-RPC entry |

### CLI flags

```
--engine-url <URL>   WebSocket URL of the iii engine (default ws://localhost:49134)
--expose-all         Ignore the a2a.expose gate (dev only). Hard floor still applies.
--tier <name>        Show only functions whose a2a.tier metadata equals <name>.
                     E.g. `--tier partner` for B2B surfaces, `--tier public` for open.
--base-url <URL>     Public origin advertised in the agent card. The card is served
                     at <base-url>/a2a. Default: http://localhost:3111
--debug              Verbose logging
```

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
