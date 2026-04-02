# Motia Workers

Worker modules for the [III engine](https://github.com/iii-hq/iii).

## Modules

### mcp

MCP protocol worker — exposes iii-engine functions as MCP tools via stdio and Streamable HTTP.

**Protocol version:** `2025-11-25`

**Features:**
- Dual transport: stdio (Claude Desktop, Cursor) + HTTP (`POST /mcp`)
- 6 built-in tools: worker register/stop, trigger register/unregister/void/enqueue
- 4 MCP resources: `iii://functions`, `iii://workers`, `iii://triggers`, `iii://context`
- 4 MCP prompts: register-function, build-api, setup-cron, event-pipeline
- Metadata filtering: only functions with `mcp.expose: true` are exposed (unless `--expose-all`)
- Spawn Node.js/Python workers on the fly via `iii_worker_register` tool

#### Build

```bash
cd mcp
cargo build --release
```

#### Usage

```bash
iii-mcp                     # MCP stdio (Claude Desktop, Cursor)
iii-mcp --no-stdio          # MCP HTTP only (POST /mcp)
iii-mcp --expose-all        # show all functions, ignore metadata filter
```

Tag functions for MCP exposure:
```js
iii.registerFunction({
  id: 'orders::process',
  metadata: { "mcp.expose": true }
}, handler)
```

#### Testing with MCP Inspector

Use [MCP Inspector](https://github.com/modelcontextprotocol/inspector) to debug and validate the MCP worker interactively.

**Setup:**

Create `mcp-inspector-config.json`:
```json
{
  "mcpServers": {
    "iii-mcp": {
      "command": "./mcp/target/release/iii-mcp",
      "args": []
    }
  }
}
```

**Web UI (interactive):**
```bash
npx @modelcontextprotocol/inspector \
  --config mcp-inspector-config.json \
  --server iii-mcp
```
Opens a browser at `http://localhost:6274` where you can list tools, call functions, read resources, and test prompts.

**CLI (scriptable):**
```bash
# List tools
npx @modelcontextprotocol/inspector --cli \
  --config mcp-inspector-config.json \
  --server iii-mcp \
  --method tools/list

# Call a tool
npx @modelcontextprotocol/inspector --cli \
  --config mcp-inspector-config.json \
  --server iii-mcp \
  --method tools/call \
  --tool-name demo__echo \
  --tool-arg 'message=hello'

# List resources
npx @modelcontextprotocol/inspector --cli \
  --config mcp-inspector-config.json \
  --server iii-mcp \
  --method resources/list
```

**End-to-end test:**

```bash
# Terminal 1: start engine
iii --use-default-config

# Terminal 2: start MCP worker
iii-mcp --debug

# Terminal 3: start a test worker with metadata
node -e "
import { registerWorker } from 'iii-sdk'
const iii = registerWorker('ws://localhost:49134')
iii.registerFunction({
  id: 'demo::echo',
  description: 'Echo input',
  metadata: { \"mcp.expose\": true },
  request_format: { type: 'object', properties: { message: { type: 'string' } }, required: ['message'] }
}, async (input) => ({ echoed: input.message }))
setInterval(() => {}, 10000)
" --input-type=module

# Terminal 4: validate (wait ~6s for function discovery)
npx @modelcontextprotocol/inspector --cli \
  --config mcp-inspector-config.json \
  --server iii-mcp \
  --method tools/list
# Should show demo__echo alongside the 6 built-in tools
```

> Functions registered without `mcp.expose: true` metadata will not appear in `tools/list` unless `--expose-all` is set. The engine polls for new functions every 5 seconds, so allow a brief delay after registration.

---

### a2a

A2A protocol worker — exposes iii-engine functions as A2A agent skills via HTTP.

**Features:**
- Full A2A type system: AgentCard, Task (8 states), Message, Part, Artifact
- Methods: `message/send`, `tasks/get`, `tasks/cancel`, `tasks/list`
- Agent card at `GET /.well-known/agent-card.json`
- Task state stored via engine KV (`a2a:tasks` scope)
- Metadata filtering: only functions with `a2a.expose: true` are exposed (unless `--expose-all`)

#### Build

```bash
cd a2a
cargo build --release
```

#### Usage

```bash
iii-a2a                     # A2A HTTP (POST /a2a + GET /.well-known/agent-card.json)
iii-a2a --expose-all        # show all functions as skills
```

Tag functions for A2A exposure:
```js
iii.registerFunction({
  id: 'orders::process',
  metadata: { "a2a.expose": true }
}, handler)
```

---

### image-resize

A Rust-based image resize worker that connects to the III engine via WebSocket and processes images through stream channels.

**Supported formats:** JPEG, PNG, WebP (including cross-format conversion)

**Resize strategies:**

| Strategy | Behavior |
|---|---|
| `scale-to-fit` | Scales the image to fit within the target dimensions, preserving aspect ratio (default) |
| `crop-to-fit` | Scales and center-crops to fill the exact target dimensions |

**Features:**
- EXIF orientation auto-correction
- Configurable quality per format (JPEG, WebP)
- Per-request parameter overrides (dimensions, quality, strategy, output format)
- Module manifest output (`--manifest`)

#### Prerequisites

- Rust 1.70+
- A running III engine instance

#### Build

```bash
cd image-resize
cargo build --release
```

#### Usage

```bash
# Run with defaults (connects to ws://127.0.0.1:49134)
./target/release/image-resize

# Custom config and engine URL
./target/release/image-resize --config ./config.yaml --url ws://host:port

# Print module manifest
./target/release/image-resize --manifest
```

#### Configuration

Create a `config.yaml` file:

```yaml
width: 200        # default target width
height: 200       # default target height
strategy: scale-to-fit  # or crop-to-fit
quality:
  jpeg: 85
  webp: 80
```

All fields are optional and fall back to the defaults shown above.

#### Tests

```bash
cd image-resize
cargo test
```
