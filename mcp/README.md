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

## Local testing

Full smoke path from a clean machine. Assumes `iii-sdk` v0.11.3 engine
installed (`which iii`).

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

Run it:

```bash
iii --no-update-check
```

Engine now listening on `ws://127.0.0.1:49134` (worker bus) and
`http://127.0.0.1:3111` (HTTP triggers).

### 2. Register test functions

Any worker that tags functions with `mcp.expose: true` works. Quick
standalone Rust worker — save as `testworker/src/main.rs` and
`cargo run --release`:

```rust
use iii_sdk::{register_worker, InitOptions, RegisterFunctionMessage};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "demo::hello".into(),
            description: Some("Say hello".into()),
            metadata: Some(json!({ "mcp.expose": true })),
            ..Default::default()
        },
        |_input: Value| async move { Ok(json!({"greeting": "hi"})) },
    );

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

### 3a. Start iii-mcp over Streamable HTTP

```bash
cargo run --release -p iii-mcp -- --no-stdio
# or (release binary): ./target/release/iii-mcp --no-stdio
```

Registers `POST /mcp` on the engine's HTTP port.

### 3b. Sanity check with curl

```bash
# tools/list — should show demo__hello, NO builtins (HTTP default hides them),
# NO state::*/engine::*/iii.*/iii::* (hard floor), NO demo__hidden (no mcp.expose).
curl -sX POST http://127.0.0.1:3111/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | jq '.result.tools[].name'

# tools/call
curl -sX POST http://127.0.0.1:3111/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"demo__hello","arguments":{}}}' \
  | jq '.result.content[0].text'
```

### 3c. MCP Inspector (GUI)

Launch the inspector UI:

```bash
DANGEROUSLY_OMIT_AUTH=true npx -y @modelcontextprotocol/inspector
# → MCP Inspector is up at http://127.0.0.1:6274
```

In the browser UI:

| Field | Value |
|---|---|
| Transport Type | Streamable HTTP |
| URL | `http://127.0.0.1:3111/mcp` |

Click **Connect** → green dot. Then try each tab:

- **Tools** → List Tools → should show the functions tagged
  `mcp.expose: true`. Click one → fill the args form (text input, not a
  dropdown — Inspector decorates it with a chevron but the MCP spec's
  `PromptArgument`/tool input schema doesn't carry enum constraints,
  so type the value) → Run.
- **Resources** → 4 URIs (`iii://functions`, `iii://workers`,
  `iii://triggers`, `iii://context`). Click `iii://functions` → filtered
  list (only exposed functions, no infra).
- **Prompts** → 4 canned prompts. Pick `register-function`, fill
  `language=python`, `function_id=orders::place`, Get Prompt.
- **Ping** → `{}` response.

### 3d. Inspector CLI (non-interactive smoke)

```bash
DANGEROUSLY_OMIT_AUTH=true npx -y @modelcontextprotocol/inspector --cli \
  --transport http http://127.0.0.1:3111/mcp --method tools/list

DANGEROUSLY_OMIT_AUTH=true npx -y @modelcontextprotocol/inspector --cli \
  --transport http http://127.0.0.1:3111/mcp \
  --method tools/call --tool-name demo__hello
```

### 3e. Stdio transport (Claude Desktop / Cursor path)

```bash
DANGEROUSLY_OMIT_AUTH=true npx -y @modelcontextprotocol/inspector --cli \
  ./target/release/iii-mcp \
  --method tools/list
```

Claude Desktop config that talks to a running engine:

```json
{
  "mcpServers": {
    "iii": {
      "command": "/absolute/path/to/iii-mcp",
      "args": ["--engine-url", "ws://127.0.0.1:49134"]
    }
  }
}
```

Add `"--tier", "user"` and `"--no-builtins"` for a clean user-facing
surface. Add `"--expose-all"` for dev exploration (hard floor still
applies).

### 4. Verify each gate

From the curl / inspector shell above:

```bash
# Hidden function (no mcp.expose) → isError, "not exposed via mcp.expose metadata"
curl -sX POST http://127.0.0.1:3111/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"demo__hidden","arguments":{}}}' \
  | jq '.result.content[0].text'

# Infra prefix → isError, "in the iii-engine internal namespace"
curl -sX POST http://127.0.0.1:3111/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"state__set","arguments":{}}}' \
  | jq '.result.content[0].text'

# Builtin with --no-builtins → "disabled on this server"
# (restart iii-mcp with --no-builtins first)
curl -sX POST http://127.0.0.1:3111/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"iii_trigger_void","arguments":{"function_id":"demo::hello","payload":{}}}}' \
  | jq '.result.content[0].text'
```

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
