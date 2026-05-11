# iii-introspection

Slim, streamable, just-in-time engine introspection worker.

Wraps `engine::workers::list` with progressive disclosure: slim by default, full schema only on `describe`. Solves Mike's "context bloat from dumping every function schema" gripe (May 8 sync).

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

Mike (May 8 sync): *"introspection is tell me everything static about my engine and stream new things about my engine"*.

Today the agent calls `engine::workers::list` → response includes every function's full request/response schema. Context bloat. Mike's gripe: *"only the function names and descriptions is necessary, and then it dives deeper if we find the right function"*.

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
