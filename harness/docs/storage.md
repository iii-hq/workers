# Harness storage

Shared configuration for harness workers that persist data through the
engine [`database`](https://github.com/iii-ai/iii/tree/main/workers/database)
worker.

## Purpose

Several harness workers (`auth-credentials`, `provider-config`, and any
future SQL-backed store) talk to the same logical SQLite pool via
`database::query` and `database::execute`. Instead of repeating
`database_name` in every worker section, operators can set it once under
a top-level `storage:` block in [config.yaml](harness/config.yaml).

## Configuration

```yaml
storage:
  database_name: harness   # shared default for every DbStore consumer

# optional per-worker override (highest precedence)
auth_credentials:
  database_name: harness_secrets
```

### Precedence

When a worker resolves its pool name, this order applies:

1. Worker section `database_name` (e.g. `auth_credentials.database_name`)
2. Shared `storage.database_name`
3. `"harness"` (built-in default)

Existing configs that only set `auth_credentials.database_name` or
`provider_config.database_name` keep working unchanged.

## Mapping to the engine database worker

The harness `database_name` is a **logical pool key**. It must match a
key in the engine `database` worker's `databases:` map, which defines
the actual connection URL (for example `sqlite:~/.iii/harness.db`).

```yaml
# engine database worker config (not harness config.yaml)
databases:
  harness:
    url: sqlite:~/.iii/harness.db
```

Harness workers never open SQLite directly; they always route through
the bus.

## Adding a new DbStore consumer

1. Add a table-backed store (or wrap `DbStore<T>`) in your worker.
2. In `register.ts`, load config once with `loadConfig`, then construct
   the store with `createDbStore`:

```ts
import { createDbStore } from '../runtime/database-store.js';

const cfg = await loadConfig(ctx.configPath);
const store = createDbStore<MyPayload>(iii, cfg, {
  workerSection: 'my_worker',   // optional; omit to use only storage.database_name
  tableName: 'my_worker_table',
});
await store.init();
```

3. Document any worker-specific overrides in your worker's doc page and
   link back here for the shared `storage:` block.

See [src/runtime/storage-config.ts](harness/src/runtime/storage-config.ts)
and [src/runtime/database-store.ts](harness/src/runtime/database-store.ts).
