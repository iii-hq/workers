# mcp

Model Context Protocol (MCP) bridge for the [iii engine](https://github.com/iii-hq/iii).
Registers a single HTTP endpoint at `POST /mcp` that speaks MCP 2025-06-18
JSON-RPC and maps each method onto an `iii.trigger` call:

| MCP method | Backed by |
|---|---|
| `tools/list`, `tools/call` | `iii.list_functions` + `iii.trigger(<id>, ...)` |
| `resources/list`, `resources/read`, `resources/templates/list` | the [`skills`](../skills) worker |
| `prompts/list`, `prompts/get` | the [`skills`](../skills) worker |

Install `skills` alongside `mcp` so MCP clients see registered skills as
`iii://{id}` resources and slash-commands as MCP prompts.

## Install

```bash
iii worker add mcp
```

To populate the resources and prompts surfaces, also install
[`skills`](../skills):

```bash
iii worker add skills
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Run

```bash
iii start
```

Once both `mcp` and the engine's `iii-http` worker are running, the MCP
endpoint is reachable at `http://<engine_http>/mcp`.

## Quickstart

Sanity-check the bridge with `curl` against MCP Inspector or any MCP
client. Substitute your engine's HTTP port (default `3000`).

```bash
# 1. initialize — handshake, returns the negotiated protocol version
curl -sX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  | jq '.result.protocolVersion'

# 2. tools/list — every iii function except hidden namespaces
curl -sX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | jq '.result.tools[].name'

# 3. tools/call — invoke one of those tools
curl -sX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"demo__hello","arguments":{}}}' \
  | jq '.result.content[0].text'

# 4. resources/list — index of skills (requires `skills` worker)
curl -sX POST http://127.0.0.1:3000/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":4,"method":"resources/list"}' \
  | jq '.result.resources'
```

### Tool naming

iii function ids contain `::`, which MCP tool names disallow. The
bridge transparently maps `myworker::echo` ↔ `myworker__echo` on the
way out and back. Pass tool names with `__` in `tools/call`; the
bridge translates them back to the iii function id.

### Hidden namespaces

Function ids starting with any of the following prefixes are excluded
from `tools/list` and rejected at `tools/call`:

`engine::`, `state::`, `stream::`, `iii.`, `iii::`, `mcp::`, `a2a::`,
`skills::`, `prompts::`

The list mirrors the hard floor enforced by the [`skills`](../skills)
worker. Add deploy-specific overrides through `hidden_prefixes` in the
config below.

## Configuration

```yaml
api_path: mcp                # POST /<api_path> on the engine HTTP port
state_timeout_ms: 30000      # per-trigger upper bound (ms)

# Optional. Defaults to the always-hidden list. Add custom prefixes to
# hide additional namespaces from MCP clients.
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

# When true, only functions whose `metadata.mcp.expose == true` are
# advertised in `tools/list`. Workers like `agentmemory` tag every
# agent-callable handler with this flag and intentionally omit it from
# HTTP wrappers, sub-skill handlers, and prompt handlers. Recommended
# for agent-facing deployments. Default false for backwards
# compatibility with workers that haven't adopted the flag.
require_expose: false
```

CLI flags:

```text
--config <PATH>    Path to config.yaml [default: ./config.yaml]
--url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
--manifest         Output the module manifest as JSON and exit
-h, --help         Print help
```

If the config file is missing or malformed the worker logs a warning
and falls back to the defaults — boot is never blocked by a bad config
path.

## MCP method coverage

The v0.1 surface implements the core methods needed for tool/resource/
prompt discovery and invocation:

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
`notifications/progress`), and the stdio transport. The skills worker
still emits its `skills::on-change` / `prompts::on-change` triggers,
so a future revision can plumb them through to MCP notifications
without changing the on-the-wire shape of the methods above.

## Local development & testing

### Run from source

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

### Tests

```bash
# Fast, offline — pure helpers (protocol types, name mapping, hidden-prefix
# matcher, parse_body) without an iii engine.
cargo test --lib
cargo test --test bdd -- --tags @pure

# Full suite — requires an iii engine on ws://127.0.0.1:49134
# (or III_ENGINE_WS_URL). Boots `skills` in-process so resources/prompts
# scenarios also pass.
cargo test

# One feature group at a time. Available tags:
#   @pure  @core
#   @engine  @tools  @resources  @prompts
cargo test --test bdd -- --tags @tools
```

The BDD harness lives under [tests/](tests/). Feature files mirror the
MCP method groups; step definitions under [tests/steps/](tests/steps/)
drive each method through the same `handler::handle` path the production
binary uses.
