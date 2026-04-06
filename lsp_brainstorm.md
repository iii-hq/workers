# III LSP Design

## Overview

A Language Server Protocol implementation for the III engine that provides editor-agnostic autocompletion and hover information for functions, triggers, services, and other engine constructs.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Data source | Live engine connection | Reuses existing `list_functions`, `list_triggers`, `on_functions_available` — no reimplementation of discovery. Cross-worker visibility for free. |
| Language | Rust | Shares protocol types with the engine, lives in the same workspace. `tower-lsp` crate is mature. |
| Editor target | Editor-agnostic | Build a compliant LSP server binary. Any editor with LSP support (VS Code, Neovim, Zed, Helix) works with minimal client config. |
| V1 features | Completions + hover | Completions alone feel incomplete. Hover is low-cost once we have the data. Diagnostics deferred to v2. |
| Engine connection | Register as a worker | LSP connects as a lightweight worker (`iii-lsp`) that never registers functions — only listens. Zero engine changes needed. |
| Context detection | Tree-sitter | Parses TS/Python/Rust files to determine if cursor is in a completable position. More accurate than string matching, handles formatting and aliasing. |
| CLI integration | `iii lsp` subcommand | Delegates to separate `iii-lsp` binary (same pattern as `iii cloud`). Keeps LSP deps out of the engine binary. |

## Architecture

```
┌──────────┐    stdio/JSON-RPC    ┌──────────┐    WebSocket     ┌──────────┐
│  Editor  │ ◄──────────────────► │ iii-lsp  │ ◄─────────────► │  Engine  │
│          │                      │ (binary) │  port 49134     │          │
└──────────┘                      └──────────┘                  └──────────┘
                                       │
                                  Tree-sitter
                                  TS/Py/Rust
```

### Workspace Layout

```
iii/
├── engine/          # existing — CLI dispatches `iii lsp` to iii-lsp binary
├── lsp/             # new crate
│   ├── Cargo.toml   # deps: tower-lsp, tree-sitter, tree-sitter-{typescript,python,rust}
│   └── src/
│       ├── main.rs          # binary entry, starts tower-lsp server on stdio
│       ├── engine_client.rs # connects to engine as worker, caches registry
│       ├── analyzer.rs      # tree-sitter context detection
│       ├── completions.rs   # completion provider
│       └── hover.rs         # hover provider
├── sdk/
└── ...
```

### Components

1. **LSP Server** (`tower-lsp`) — Handles JSON-RPC from the editor. Implements `completion`, `hover`, `initialize`, `shutdown`.
2. **Engine Client** — Connects to the III engine as a worker named `iii-lsp`. Subscribes to `FunctionsAvailable` events. Maintains an in-memory cache of all functions, triggers, trigger types, and services.
3. **Context Analyzer** (tree-sitter) — On each completion/hover request, parses the current file with the appropriate grammar and walks the AST to determine if the cursor is in a completable position (e.g., inside a `function_id` argument of a `trigger()` call).

## Flow

1. User runs `iii lsp` → CLI dispatches to `iii-lsp` binary, starts on stdio
2. Editor connects to the running LSP server
3. `iii-lsp` connects to engine at `ws://localhost:49134` as worker `iii-lsp`
4. Engine pushes `FunctionsAvailable` events **continuously** → LSP keeps its cache in sync
5. User types inside a `trigger()` call → editor sends `textDocument/completion`
6. LSP parses file with tree-sitter, detects cursor is in `function_id` position
7. LSP returns cached function IDs as completion items (with descriptions)
8. User hovers a function ID → LSP returns description, request/response formats, worker info

## Engine Client

The engine client is a lightweight worker that never registers functions — it only listens.

### Connection Lifecycle

```
iii-lsp starts
    │
    ├─► Connect to ws://localhost:49134
    ├─► Receive WorkerRegistered (worker_id assigned)
    ├─► Call list_functions → seed initial cache
    ├─► Call list_triggers → seed initial cache
    │
    └─► Event loop:
         ├── FunctionsAvailable → update function cache
         ├── Connection lost → mark cache as stale, retry with backoff
         └── Reconnected → re-seed cache
```

### Cache Structure

```rust
struct EngineCache {
    functions: DashMap<String, FunctionInfo>,        // "todos::create" → description, formats, worker_id
    triggers: DashMap<String, TriggerInfo>,           // trigger_id → type, function_id, config
    trigger_types: DashMap<String, TriggerTypeInfo>,  // "http", "cron", etc.
    services: DashSet<String>,                        // derived from function IDs ("todos", "math", ...)
    connected: AtomicBool,                            // engine connection status
}
```

### Engine URL Resolution (priority order)

1. `--address` / `--port` CLI flags on `iii lsp`
2. `III_URL` environment variable
3. Parse `iii-config.yaml` worker module port/host
4. Default: `ws://localhost:49134`

## Completion Targets

| Context | Completes with |
|---------|---------------|
| `function_id: '▏'` in `trigger()` | All registered function IDs |
| `trigger_type: '▏'` in `registerTrigger()` | All registered trigger types (`http`, `cron`, `queue`, `stream`, ...) |
| `function_id: '▏'` in `registerTrigger()` | All registered function IDs |
| Service namespace (`todos::▏`) | Functions within that service |

## Hover Targets

| Hover on | Shows |
|----------|-------|
| Function ID string (e.g., `'todos::create'`) | Description, request/response format (JSON Schema), worker name |
| Trigger type string (e.g., `'http'`) | Trigger type description, expected config shape |

## Future Work (not in v1)

- **Diagnostics** — Red squiggles for invalid function IDs, unknown trigger types, missing required config fields.
- **Go-to-definition** — Jump to the file where a function is registered.
- **Static analysis fallback** — Parse source files for `registerFunction` calls when engine is not running.
- **Config file support** — Completions and validation for `iii-config.yaml` (module classes, adapter classes, config keys).
- **VS Code extension** — Thin client that auto-detects III projects and spawns `iii lsp`.
