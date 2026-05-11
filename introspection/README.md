# iii-introspection

Slim, streamable, just-in-time engine introspection worker.

Wraps `engine::workers::list` and `engine::functions::list` with progressive disclosure: slim by default, full schema only on `describe`. Avoids the per-turn context bloat from dumping every function schema.

## Functions

| Function ID | Purpose |
|---|---|
| `introspection::workers::list` | Slim worker list (name, status, function_count, description). `include=full` for raw `engine::workers::list` graph. |
| `introspection::workers::describe` | One worker, full detail. |
| `introspection::functions::list` | Slim function list (id + description). Optional `worker` filter, `filter` substring. |
| `introspection::functions::describe` | Just-in-time full schema for one function id. |
| `introspection::stream::subscribe` | Snapshot today. Live stream lands when engine emits on pubsub channel `introspection.registrations`. |
| `introspection::registry::query` | Search `workers.iii.dev/registry/index.json`. |

## Why this worker

The goal: tell the agent everything static about the engine and stream new things as they register.

Today the agent calls `engine::workers::list` → response includes every function's full request/response schema. Context bloat. The agent only needs function names and descriptions until it picks one; the full schema can wait.

This worker enforces progressive disclosure at the introspection boundary:

1. `introspection::functions::list` → slim ids only.
2. Agent picks one. 
3. `introspection::functions::describe id` → full schema only for the chosen function.
4. Agent calls.

Same shape as the progressive-disclosure pattern used elsewhere: descriptions in context, full schemas on demand.

## Build

```bash
cd workers/introspection
cargo build --release
```

## Run

```bash
./target/release/iii-introspection --url ws://127.0.0.1:49134 --config ./config.yaml
```

## SDK

Pinned to `iii-sdk = "=0.11.6"` (max stable on crates.io as of 2026-05-11). Engine HEAD is `0.11.7-next.1` but unreleased.

## Config

```yaml
registry_url: https://workers.iii.dev
default_timeout_ms: 5000
```

## License

Apache-2.0.
