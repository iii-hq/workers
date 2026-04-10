# iii-introspect

When you build complex workflows with fan-out, reactive state, emit/subscribe chains, and 20+ steps — it becomes very hard to reason about what's happening. iii-introspect solves this. It traces specific workflows through their trigger chains, explains what each function does in plain language, generates focused Mermaid diagrams, and runs health checks. Ask it "how does my onboarding workflow work?" and it walks the full execution path.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-introspect --url ws://your-engine:49134`. It registers 9 functions and starts caching topology every 5 minutes. Call `introspect::trace_workflow` with any function ID to trace its dependency chain, or `introspect::explain` to get a business-level explanation of what it does and how it's triggered.

## Functions

| Function ID | Description |
|---|---|
| `introspect::functions` | List all registered functions in the engine |
| `introspect::workers` | List all connected workers |
| `introspect::triggers` | List all registered triggers |
| `introspect::topology` | Full system topology with stats (cached with TTL) |
| `introspect::diagram` | Generate a Mermaid flowchart of the system topology |
| `introspect::health` | Health check for orphaned functions, empty workers, duplicate IDs |
| `introspect::topology_refresh` | Cron-triggered cache refresh (internal, not exposed via HTTP) |

## iii Primitives Used

- **State** -- cached topology snapshot at `introspect:cache:topology`
- **Cron** -- periodic topology cache refresh
- **HTTP** -- all public functions exposed as GET endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`

## Build

```bash
cargo build --release
```

## Usage

```bash
./target/release/iii-introspect --url ws://127.0.0.1:49134 --config ./config.yaml
```

```
Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```

## Configuration

```yaml
cron_topology_refresh: "0 */5 * * * *"  # refresh cache every 5 minutes
cache_ttl_seconds: 300                   # TTL for cached topology data
```

## Tests

```bash
cargo test
```
