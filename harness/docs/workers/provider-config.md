# provider-config

Runtime provider settings store on the iii bus (`provider_config::*`).

## Purpose

Centralised storage for non-secret provider overrides -- base URL and max
tokens -- that the five active provider workers read at completion time.
Stored entries live in a single JSON file at
`~/.iii/provider-config.json` (path configurable). All mutations serialize
through an in-process write queue and use atomic `tmp` + `rename` writes.

`provider_config::get` returns the overrides for a single provider (empty
object when none are stored). Provider workers call this in parallel with
`auth::get_token` so a runtime override takes precedence over the
`config.yaml` value without requiring a restart.

[iii-permissions.yaml](iii-permissions.yaml) denies
`provider_config::set` and `provider_config::clear` to in-run agents
(kernel rules). `provider_config::get` and `provider_config::list` are on
the bare-string allow list.

## Registered functions

- `provider_config::get` -- Fetch runtime overrides for a provider.
- `provider_config::set` -- Set runtime overrides for a provider.
- `provider_config::clear` -- Remove runtime overrides for a provider.
- `provider_config::list` -- List all providers with runtime overrides.

## Triggers

None.

## On-disk layout

Persistence is delegated to the [`database`](https://github.com/iii-ai/iii/tree/main/workers/database)
worker via `database::query` / `database::execute`. A single SQLite file
(default `~/.iii/harness.db`, configurable in the engine's `database`
worker config) backs both `provider-config` and its sibling
`auth-credentials`. Each worker owns one table.

| Table | Columns | Purpose |
|---|---|---|
| `provider_config` | `provider TEXT PRIMARY KEY, payload TEXT NOT NULL, updated_at INTEGER NOT NULL` | One row per provider; `payload` is `{ "default_api_url"?: string, "default_max_tokens"?: number }` serialized as JSON. |

Mutations use `INSERT ... ON CONFLICT(provider) DO UPDATE`, so a single
`set` is atomic at the SQL layer. Partial-update (merge) semantics are
preserved inside the worker via an in-process write queue around a
read-modify-upsert sequence -- callers can still patch one field at a
time without clobbering the rest of a row.

## Configuration

Shared pool name resolution is documented in
[storage.md](../storage.md). From [config.yaml](harness/config.yaml):

```yaml
storage:
  database_name: harness   # default for auth-credentials and provider-config

provider_config:
  database_name: harness   # optional override (wins over storage.database_name)
```

- `storage.database_name` (default `harness`) -- shared logical pool for
  every harness `DbStore` consumer.
- `provider_config.database_name` (optional) -- per-worker override;
  must match a key in the engine `database` worker's `databases:` map.

## Dependencies

The `database` worker must be on the bus -- this worker calls
`database::query` and `database::execute` on every read/write. See
[src/provider-config/iii.worker.yaml](harness/src/provider-config/iii.worker.yaml).

## Source layout

| File | Purpose |
|---|---|
| [src/provider-config/main.ts](harness/src/provider-config/main.ts) | Binary entry point (`iii-provider-config`). |
| [src/provider-config/register.ts](harness/src/provider-config/register.ts) | Composes the four handlers around a `DbOverridesStore`; runs the JSON-import migration. |
| [src/provider-config/config.ts](harness/src/provider-config/config.ts) | Loads the `provider_config` section. |
| [src/provider-config/types.ts](harness/src/provider-config/types.ts) | `ProviderOverrides` type definition. |
| [src/provider-config/store.ts](harness/src/provider-config/store.ts) | `OverridesStore` interface + `InMemoryStore` + `DbOverridesStore` (wraps the shared `DbStore<ProviderOverrides>`). |
| [src/runtime/database-store.ts](harness/src/runtime/database-store.ts) | Shared SQL-backed key-value store used by both `provider-config` and `auth-credentials`. |
| [src/provider-config/handlers/get.ts](harness/src/provider-config/handlers/get.ts) | `provider_config::get` handler. |
| [src/provider-config/handlers/set.ts](harness/src/provider-config/handlers/set.ts) | `provider_config::set` handler. |
| [src/provider-config/handlers/clear.ts](harness/src/provider-config/handlers/clear.ts) | `provider_config::clear` handler. |
| [src/provider-config/handlers/list.ts](harness/src/provider-config/handlers/list.ts) | `provider_config::list` handler. |
| [src/provider-config/iii.worker.yaml](harness/src/provider-config/iii.worker.yaml) | Worker manifest. |
