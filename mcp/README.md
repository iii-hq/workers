# iii-mcp

Model Context Protocol surface for the iii engine. Speaks MCP JSON-RPC on
two transports:

- **stdio** (default): for Claude Desktop, Cursor, MCP Inspector launching
  the binary directly.
- **Streamable HTTP**: `POST /mcp` on the engine's HTTP trigger port.
  Enable with `--no-stdio`.

## Exposure model

Functions registered on the engine are **invisible to MCP clients by
default**. Authors opt in per-function.

### Opt-in metadata

```rust
// Rust worker
iii.register_function_with(
    RegisterFunctionMessage {
        id: "eval::metrics".into(),
        description: Some("P50/P95/P99 latency + error rate".into()),
        metadata: Some(json!({
            "mcp.expose": true,
            "mcp.tier": "agent",  // optional — free-form string
        })),
        ..Default::default()
    },
    handler,
);
```

```ts
// Node worker (iii-sdk 0.11.3+)
iii.registerFunction("eval::metrics", handler, {
  metadata: { "mcp.expose": true, "mcp.tier": "agent" },
})
```

```python
# Python worker
iii.register_function(
    "eval::metrics",
    handler,
    metadata={"mcp.expose": True, "mcp.tier": "agent"},
)
```

### Hard floor (never exposed)

Even with `--expose-all`, these namespaces are **always** filtered out:

| Prefix | Why |
|---|---|
| `engine::*` | iii engine internals |
| `state::*` | KV plumbing, not an agent tool |
| `stream::*` | channel plumbing |
| `iii.*` / `iii::*` | SDK internals — callback functions and namespaced SDK APIs |
| `mcp::*` | this worker's own entry point |
| `a2a::*` | sibling protocol worker's entry point |

### CLI flags

```text
--engine-url <URL>   WebSocket URL of the iii engine (default ws://localhost:49134)
--no-stdio           Skip stdio, run HTTP-only
--expose-all         Ignore the mcp.expose gate (dev only). Hard floor still applies.
--no-builtins        Hide the 6 built-in management tools on stdio. Also hidden over HTTP by default.
--http-builtins      Opt in to exposing built-ins on HTTP (default: hidden).
                     --no-builtins always wins and hides them everywhere.
--tier <name>        Show only functions whose mcp.tier metadata equals <name>.
                     E.g. `--tier user` for end-user Claude Desktop config,
                     `--tier agent` for an agent client, `--tier ops` for dashboards.
--debug              Verbose logging
```

## Built-in management tools

`iii-mcp` emits 6 built-in tools that let an MCP client drive an iii engine:

- `iii_worker_register` — spawn a Node/Python worker from source
- `iii_worker_stop` — kill a spawned worker
- `iii_trigger_register` / `iii_trigger_unregister` — attach or detach
  HTTP/cron/queue triggers
- `iii_trigger_void` — fire-and-forget invocation
- `iii_trigger_enqueue` — enqueue to a named queue

**Stdio transport keeps these visible by default.** HTTP transport hides
them by default because worker spawning requires the stdio-attached
process — the tools would error on invocation anyway. Opt in per-deploy
with `--http-builtins` to advertise them over HTTP as well (they will
still error on `iii_worker_*` and `iii_trigger_register*` because those
paths need the attached process). `--no-builtins` always wins and hides
them on both transports.

## Invocation path

`tools/call` with name `foo__bar` →
1. Reverse-mangle to `foo::bar`.
2. Hard-floor check: reject if prefix matches `ALWAYS_HIDDEN_PREFIXES`.
3. Re-check the function actually has `mcp.expose: true` (and optional
   tier match) — `tools/list` snapshots can go stale.
4. `iii.trigger(function_id, payload)` with the arguments object.

Result is wrapped as `{ content: [{ type: "text", text: ... }] }` per MCP
spec. Errors surface as `isError: true`.

## Example: minimal agent-facing config

One worker that exposes two agent-facing functions and one ops-only function:

```rust
// user-facing
iii.register_function_with(
    RegisterFunctionMessage {
        id: "reports::weekly".into(),
        metadata: Some(json!({ "mcp.expose": true, "mcp.tier": "user" })),
        ..Default::default()
    },
    weekly_handler,
);

// agent-only planning tool
iii.register_function_with(
    RegisterFunctionMessage {
        id: "reports::plan".into(),
        metadata: Some(json!({ "mcp.expose": true, "mcp.tier": "agent" })),
        ..Default::default()
    },
    plan_handler,
);

// ops dashboards — not exposed to user/agent tiers
iii.register_function_with(
    RegisterFunctionMessage {
        id: "reports::rebuild_cache".into(),
        metadata: Some(json!({ "mcp.expose": true, "mcp.tier": "ops" })),
        ..Default::default()
    },
    rebuild_handler,
);
```

Claude Desktop user config:

```json
{ "mcpServers": { "iii": { "command": "iii-mcp", "args": ["--tier", "user", "--no-builtins"] } } }
```

Agent client hits the engine over HTTP with `--tier agent`. Ops dashboard
uses `--tier ops`. Same worker, three audiences.
