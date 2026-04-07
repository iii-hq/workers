# iii-introspect Worker Specification

Queries the iii engine registry to discover what is running — functions, workers, triggers — and generates topology maps and health checks.

## Functions

### `introspect::functions`
List all registered functions in the engine.

**Input:** `{}` (empty object)

**Output:**
```json
{
  "functions": [
    {
      "id": "string",
      "description": "string | null",
      "request_format": "object | null",
      "response_format": "object | null",
      "metadata": "object | null"
    }
  ],
  "count": 0
}
```

**HTTP:** `GET /introspect/functions`

---

### `introspect::workers`
List all connected workers.

**Input:** `{}` (empty object)

**Output:**
```json
{
  "workers": [
    {
      "id": "string",
      "name": "string | null",
      "function_count": 0,
      "functions": ["string"],
      "status": "string",
      "runtime": "string | null",
      "version": "string | null",
      "connected_at_ms": 0,
      "active_invocations": 0
    }
  ],
  "count": 0
}
```

**HTTP:** `GET /introspect/workers`

---

### `introspect::triggers`
List all registered triggers (external only).

**Input:** `{}` (empty object)

**Output:**
```json
{
  "triggers": [
    {
      "id": "string",
      "trigger_type": "string",
      "function_id": "string",
      "config": {},
      "metadata": "object | null"
    }
  ],
  "count": 0
}
```

**HTTP:** `GET /introspect/triggers`

---

### `introspect::topology`
Full system topology combining functions, workers, and triggers with stats. Results are cached in state with a configurable TTL (`cache_ttl_seconds`).

**Input:** `{}` (empty object)

**Output:**
```json
{
  "functions": [],
  "workers": [],
  "triggers": [],
  "stats": {
    "total_functions": 0,
    "total_workers": 0,
    "total_triggers": 0,
    "functions_per_worker": [
      { "worker": "string", "function_count": 0 }
    ]
  },
  "cached_at": 1700000000
}
```

**HTTP:** `GET /introspect/topology`

---

### `introspect::diagram`
Generate a mermaid flowchart diagram of the system topology. Workers are rendered as subgraphs, functions as nodes, triggers as edges.

**Input:** `{}` (empty object)

**Output:**
```json
{
  "format": "mermaid",
  "content": "graph TD\n    subgraph w1[\"my-worker\"]\n        test__echo[\"test::echo\"]\n    end\n    t1{\"t1\"} -->|http| test__echo\n"
}
```

**HTTP:** `GET /introspect/diagram`

---

### `introspect::health`
System health check. Inspects the registry for common issues.

**Checks performed:**
- `orphaned_functions` — functions with no triggers bound (status: warn)
- `empty_workers` — workers with zero registered functions (status: warn)
- `duplicate_function_ids` — same function ID registered more than once (status: fail)

**Input:** `{}` (empty object)

**Output:**
```json
{
  "healthy": true,
  "checks": [
    {
      "name": "orphaned_functions",
      "status": "pass | warn",
      "detail": "string"
    },
    {
      "name": "empty_workers",
      "status": "pass | warn",
      "detail": "string"
    },
    {
      "name": "duplicate_function_ids",
      "status": "pass | fail",
      "detail": "string"
    }
  ],
  "timestamp": "2026-04-06T12:00:00Z"
}
```

**HTTP:** `GET /introspect/health`

---

## Internal Functions

### `introspect::topology_refresh`
Called by the cron trigger to refresh the topology cache. Not exposed via HTTP.

---

## State Scopes

| Key | Purpose |
|---|---|
| `introspect:cache:topology` | Cached topology snapshot with `cached_at` timestamp |

State is accessed via `state::get` / `state::set` with scope `introspect`.

---

## Triggers

| Type | Target | Config |
|---|---|---|
| `cron` | `introspect::topology_refresh` | `{ "cron": "0 */5 * * * *" }` (configurable) |
| `http` GET | `introspect::functions` | `{ "api_path": "introspect/functions" }` |
| `http` GET | `introspect::workers` | `{ "api_path": "introspect/workers" }` |
| `http` GET | `introspect::triggers` | `{ "api_path": "introspect/triggers" }` |
| `http` GET | `introspect::topology` | `{ "api_path": "introspect/topology" }` |
| `http` GET | `introspect::diagram` | `{ "api_path": "introspect/diagram" }` |
| `http` GET | `introspect::health` | `{ "api_path": "introspect/health" }` |

---

## Configuration

File: `config.yaml`

| Field | Type | Default | Description |
|---|---|---|---|
| `cron_topology_refresh` | string | `"0 */5 * * * *"` | Cron expression for periodic cache refresh |
| `cache_ttl_seconds` | u64 | `30` | TTL in seconds for cached topology data |

---

## Integration Points

- **SDK methods:** `iii.list_functions()`, `iii.list_workers()`, `iii.list_triggers(false)`
- **Engine state:** `state::get`, `state::set` with scope `introspect`
- **Engine triggers:** Built-in `cron` and `http` trigger types
- **Telemetry:** OpenTelemetry via `iii-sdk` otel feature

---

## CLI

```
iii-introspect [OPTIONS]

Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the III engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```
