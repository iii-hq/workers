# iii-mcp-client

Consume external MCP servers as if they were native iii functions. Connect once over stdio or Streamable HTTP, and every tool, resource, and prompt the remote server exposes shows up in the iii registry — invokable via `iii.trigger`, exposable via HTTP triggers, callable from any other worker. This unblocks cross-protocol harness building: anything in the broader MCP ecosystem becomes available to any iii primitive.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-mcp-client --engine-url ws://your-engine:49134 --connect stdio:fs:npx:-y:@modelcontextprotocol/server-filesystem:/tmp`. Every tool the remote server lists is registered as `mcp.<server-name>::<tool-name>`. Resources land at `mcp.<server-name>.resources::<slug>`, prompts at `mcp.<server-name>.prompts::<name>`.

## Why

MCP is the lingua franca for LLM tool servers. iii is the substrate for narrow workers. Bridging the two means a harness built on iii can wire in any MCP server — filesystem, git, Slack, custom — without writing per-server adapters. Each remote tool becomes an iii function with metadata (`mcp.remote.server`, `mcp.remote.tool`, `mcp.remote.transport`) so introspection, routing, and observability all keep working.

## CLI flags

```text
Options:
  --engine-url <URL>          WebSocket URL of the iii engine [default: ws://localhost:49134]
  --connect <SPEC>            Repeatable. stdio:<name>:<bin>[:arg1:arg2:...] or http:<name>:<url>
  --namespace-prefix <PFX>    iii function ID prefix [default: mcp]
  --debug                     Enable debug-level tracing
  -h, --help                  Print help
```

The `--engine-url` flag is intentionally explicit. Sibling workers (introspect, llm-router) use `--url`; this crate follows the brief and uses `--engine-url`. Reconciling that inconsistency is tracked in the umbrella issue.

## config.yaml example

iii-mcp-client itself does not read a config file in this PR — all wiring is via CLI flags. A future revision will accept a YAML manifest for declarative session specs.

```yaml
# planned format (not yet implemented)
engine_url: ws://localhost:49134
namespace_prefix: mcp
sessions:
  - name: fs
    transport: stdio
    bin: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
  - name: github
    transport: http
    url: https://mcp.github.example/mcp
```

## How it works

```text
                           ┌─────────────────┐
                           │  iii engine     │
                           │  ws://:49134    │
                           └────────┬────────┘
                                    │  register_function (tool/resource/prompt)
                                    │  trigger (sync invoke)
                                    ▼
                           ┌─────────────────┐
                           │  iii-mcp-client │
                           └─┬──────────────┬┘
                  stdio JSON │              │ Streamable HTTP
                  (lines)    │              │ (POST + Mcp-Session-Id)
                             ▼              ▼
                   ┌──────────────┐  ┌──────────────┐
                   │ child proc   │  │ remote MCP   │
                   │ (npx/py/…)   │  │ server (HTTP)│
                   └──────────────┘  └──────────────┘
```

On startup, iii-mcp-client opens a transport per `--connect` spec, sends the MCP `initialize` request, captures server capabilities, and sends the `notifications/initialized` notification. It then calls `tools/list`, `resources/list`, `prompts/list` (gated on capabilities) and registers every entry as a local iii function. When a registered function is invoked, the handler proxies the call back through the transport and returns the JSON-RPC result to the iii engine.

## Limitations

- **Streamable HTTP SSE not consumed.** This PR reads the first `application/json` reply per request. SSE event multiplexing — long-lived `text/event-stream` connections, server-pushed notifications over HTTP — is documented as TODO and intentionally deferred.
- **Stdio child cleanup is SIGTERM-only.** `tokio::process::Command::kill_on_drop(true)` plus an explicit kill-on-shutdown handle the common cases. Children that ignore SIGTERM will leak until the OS reaps them.
- **`--engine-url` flag.** Inconsistent with sibling workers' `--url`. Tracked separately.
- **Live-engine integration.** End-to-end assertions that round-trip through `iii.trigger` require a running iii engine. The included tests cover transport and session in isolation; a `#[ignore]`d test for `register_all → list → trigger` is opt-in via `cargo test -- --ignored`.
- **`tools/list_changed` reconciliation.** `registration::reconcile` does the diff-and-replace dance, but no notification listener is wired up yet — call it manually for now.

## Build

```bash
cargo build --release
```

## Tests

```bash
cargo test
cargo test -- --ignored   # live-engine round-trip (requires ws://localhost:49134)
```
