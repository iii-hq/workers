# mcp

Model Context Protocol (MCP) bridge for the [iii engine](https://github.com/iii-hq/iii),
shipped as a single self-contained Rust binary worker. Registers an HTTP
endpoint (default `POST /mcp`) that speaks MCP 2025-06-18 JSON-RPC and maps
each method onto an `iii.trigger` call. Install [`skills`](../skills)
alongside `mcp` so MCP clients see registered skills as `iii://{id}`
resources and slash-commands as MCP prompts.

## Install

```bash
iii worker add mcp
```

To populate the resources and prompts surfaces, also install `skills`:

```bash
iii worker add skills
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Quickstart

Sanity-check the bridge with `curl` against MCP Inspector or any MCP client.
Substitute your engine's HTTP port (default `3000`).

Responses use the **MCP Streamable HTTP** transport: `POST /mcp` returns
`text/event-stream` with a single `event: message` SSE event whose `data:`
line is the JSON-RPC reply. Pure JSON-RPC notifications (no `id`) get HTTP
`202 Accepted` with no body.

```bash
curl -sNX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -H 'accept: text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  | sed -n 's/^data: //p' | jq '.result.protocolVersion'

curl -sNX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -H 'accept: text/event-stream' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | sed -n 's/^data: //p' | jq '.result.tools[].name'

curl -sNX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -H 'accept: text/event-stream' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"demo__hello","arguments":{}}}' \
  | sed -n 's/^data: //p' | jq '.result.content[0].text'
```

`-N` disables curl's output buffering so the SSE frame appears as soon as the
bridge writes it.

## Configuration

```yaml
hidden_prefixes:
  - "engine::"
  - "state::"
  - "stream::"
  - "iii."
  - "iii::"
  - "mcp::"
  - "a2a::"
  - "skills::"
  - "prompts::"
require_expose: false
api_path: "/mcp"
```

`require_expose: true` advertises only functions whose `metadata.mcp.expose ==
true`. `api_path` lets you co-deploy with the TypeScript bridge in
`src/mcp/` by binding a different path (e.g. `/mcp-rs`).

### Hidden namespaces

Function ids starting with any of the prefixes listed in `hidden_prefixes`
are excluded from `tools/list` and rejected at `tools/call`. The defaults
mirror the hard floor enforced by the `skills` worker: `engine::`, `state::`,
`stream::`, `iii.`, `iii::`, `mcp::`, `a2a::`, `skills::`, `prompts::`. The
bridge's own handler id (`mcp::handler`) is already covered by the `mcp::`
prefix, so it is auto-hidden.

### Tool naming

iii function ids contain `::`, which MCP tool names disallow. The bridge
transparently maps `myworker::echo` ↔ `myworker__echo` on the way out and
back. Pass tool names with `__` in `tools/call`; the bridge translates them
back to the iii function id.

If the config file is missing or malformed the worker logs a warning and
falls back to the defaults; boot is never blocked by a bad config path.

## MCP method coverage

The v0.1 surface implements the core methods needed for tool/resource/prompt
discovery and invocation:

| Method | Behaviour |
|---|---|
| `initialize` | Returns `{ protocolVersion: "2025-06-18", capabilities: { tools, resources, prompts }, serverInfo }` |
| `ping` | Returns `{}` |
| `notifications/initialized` | Acknowledged with no response |
| `tools/list` | Enumerates non-hidden iii functions |
| `tools/call` | Invokes the named iii function via `iii.trigger` |
| `resources/list` | Delegates to `skills::resources-list` |
| `resources/read` | Delegates to `skills::resources-read` |
| `resources/templates/list` | Delegates to `skills::resources-templates` |
| `prompts/list` | Delegates to `prompts::mcp-list` |
| `prompts/get` | Delegates to `prompts::mcp-get` |

Out of scope in v0.1: `resources/subscribe`, `resources/unsubscribe`,
`completion/complete`, `logging/setLevel`, MCP notification fan-out
(`tools/list_changed`, `resources/updated`, `notifications/message`,
`notifications/progress`), and the stdio transport.
