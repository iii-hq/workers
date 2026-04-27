# iii-mcp

Model Context Protocol surface for the iii engine. Speaks MCP JSON-RPC on
two transports:

- **stdio** (default): for Claude Desktop, Cursor, MCP Inspector launching
  the binary directly.
- **Streamable HTTP**: `POST /mcp` on the engine's HTTP trigger port.
  Enable with `--no-stdio`.

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
--no-stdio           Skip stdio, run HTTP-only
--no-builtins        Hide the 6 built-in management tools on stdio. Also hidden over HTTP by default.
--http-builtins      Opt in to exposing built-ins on HTTP (default: hidden).
                     --no-builtins always wins and hides them everywhere.
--rbac-tag <TAG>     Forward an `x-iii-rbac-tag` header on the worker WebSocket
                     upgrade. iii-worker-manager's `auth_function_id` reads
                     this tag to apply policy.
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

Any worker registering functions through the engine works. Quick
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
# tools/list — visible functions are governed by iii-worker-manager RBAC.
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

- **Tools** → List Tools → shows the functions iii-worker-manager RBAC
  exposes for this connection.
- **Resources** → 4 URIs (`iii://functions`, `iii://workers`,
  `iii://triggers`, `iii://context`).
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

Add `"--rbac-tag", "claude-desktop"` and `"--no-builtins"` for a clean
user-facing surface — the rbac-tag is forwarded as `x-iii-rbac-tag` and
your `auth_function_id` can scope policy on it.

## Invocation path

`tools/call` with name `foo__bar` →
1. Reverse-mangle to `foo::bar`.
2. Structural guard: reject `mcp::*` / `a2a::*` (protocol entry points).
3. `iii.trigger(function_id, payload)` with the arguments object.
   iii-worker-manager applies its RBAC policy on the way through.

Result is wrapped as `{ content: [{ type: "text", text: ... }] }` per MCP
spec. Errors surface as `isError: true`.

## MCP 2025-06-18 spec coverage

Beyond the core `tools/list` + `tools/call` path, this worker speaks:

- **Pagination** on `tools/list`, `resources/list`, `prompts/list` —
  opaque `cursor` round-trip per spec.
- **`completion/complete`** — argument autocomplete for prompt args
  (e.g. language enum on `register-function`).
- **`logging/setLevel`** + **`notifications/message`** — per-session
  level bridged into `tracing`.
- **`resources/subscribe`** + **`resources/unsubscribe`** +
  **`notifications/resources/updated`** — change notifications when
  `iii.list_functions()` / `list_workers()` / `list_triggers()` shift
  for a subscribed URI.
- **`resources/templates/list`** — templates for `iii://functions/{id}`,
  `iii://workers/{id}`, `iii://triggers/{id}`.
- **Tool annotations + outputSchema + structured content** — tools
  surface `title`, `annotations` (`readOnlyHint`, `destructiveHint`,
  `idempotentHint`, `openWorldHint`), and `outputSchema` from
  `FunctionInfo.metadata.mcp.*`.
- **Progress + cancellation** — `_meta.progressToken` on `tools/call`
  emits `notifications/progress`; `notifications/cancelled` flips a
  per-request cancel channel and short-circuits the in-flight trigger.

Server-to-client requests (`sampling/createMessage`,
`elicitation/create`) and Streamable HTTP SSE remain on the Phase 2b/2c
roadmap.
