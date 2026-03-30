# Motia Workers

Worker modules for the [III engine](https://github.com/iii-hq/iii).

## Modules

### mcp

MCP protocol worker — exposes iii-engine functions as MCP tools via stdio and Streamable HTTP.

**Protocol version:** `2025-11-25`

**Features:**
- Dual transport: stdio (Claude Desktop, Cursor) + HTTP (`POST /mcp`)
- 6 builtin tools: worker register/stop, trigger register/unregister/void/enqueue
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
