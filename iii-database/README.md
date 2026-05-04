# iii-database

> Connect to PostgreSQL, MySQL, and SQLite. Run queries, prepared statements, transactions, and subscribe to row-level change feeds.

| field | value |
|-------|-------|
| version | 1.0.0 |
| type | binary |
| supported_targets | x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu |
| author | iii |

## Install

```sh
iii worker add iii-database@1.0.0
```

## Configure

Add a single `databases` block to your `config.yaml`. SQLite is the recommended starting point — no server, just a file:

```yaml
workers:
  - name: iii-database
    config:
      databases:
        primary:
          url: sqlite:./data/iii.db
          pool:
            max: 10
            idle_timeout_ms: 30000
            acquire_timeout_ms: 5000
        analytics:
          url: ${ANALYTICS_URL}    # postgres:// or mysql://
          pool: { max: 5 }
```

URL scheme picks the driver: `sqlite:`, `postgres://`, `postgresql://`, `mysql://`.

### TLS (postgres + mysql)

Postgres and mysql connections default to **`tls.mode: require`** — TLS handshake required, certificate chain validated against the system trust store, hostname verification skipped (matches libpq's `sslmode=require`). Override per-database:

```yaml
databases:
  primary:
    url: postgres://app@db.example.com:5432/app
    tls:
      mode: verify-full           # disable | require | verify-full   (default: require)
      ca_cert: /etc/ssl/internal-ca.pem    # optional; replaces the system trust store
  local:
    url: postgres://dev@localhost:5432/dev
    tls:
      mode: disable               # plaintext, local development only
```

- **`disable`** — plaintext. Local dev only.
- **`require`** (default) — encrypted; cert chain validated; hostname is **not** verified. Catches passive eavesdropping, doesn't catch a determined MITM with their own valid-chain cert.
- **`verify-full`** — encrypted; cert chain validated; cert hostname must match the URL host. Production default for managed services (RDS, Neon, Supabase).

`ca_cert` lets you point at a private CA bundle for self-hosted databases. When set, it **replaces** the system trust store rather than extending it.

SQLite ignores the `tls` block (local-file driver).

## Quick start (SQLite)

```ts
import { call } from 'iii-sdk'

await call('iii-database::execute', {
  db: 'primary',
  sql: 'CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, email TEXT)'
})

await call('iii-database::execute', {
  db: 'primary',
  sql: 'INSERT INTO users (email) VALUES (?), (?)',
  params: ['a@x', 'b@x']
})

const { rows } = await call('iii-database::query', {
  db: 'primary',
  sql: 'SELECT id, email FROM users ORDER BY id'
})
```

## Functions

| Function | Purpose |
|---|---|
| `iii-database::query` | Read SQL. Returns `{ rows, row_count, columns }`. |
| `iii-database::execute` | Write SQL. Returns `{ affected_rows, last_insert_id, returned_rows }`.<br>**`last_insert_id` semantics:** SQLite/MySQL surface the engine's `last_insert_rowid()` / `LAST_INSERT_ID()` (only populated for INSERT). Postgres has no equivalent — `last_insert_id` is set from the **first column of the first RETURNING row**, so put your PK first: `RETURNING id, name`, not `RETURNING name, id`. |
| `iii-database::prepareStatement` | Pin a connection and return `{ handle: { id, expires_at } }`. |
| `iii-database::runStatement` | Run a previously-prepared handle. (No `timeout_ms` — uses the pinned connection's session lifetime; configure via `ttl_seconds` on `prepareStatement`.) |
| `iii-database::transaction` | Atomic sequence; rolls back on first failure. |

## Triggers

### `iii-database::query-poll`
Polls a SQL query at a fixed interval, dispatches new rows, and persists a cursor inside the watched database in `__iii_cursors`.

```yaml
triggers:
  - type: iii-database::query-poll
    config:
      db: primary
      sql: SELECT id, body FROM outbox WHERE id > COALESCE(?, 0) ORDER BY id LIMIT 50
      interval_ms: 1000
      cursor_column: id
```

The trigger binds the cursor as the single positional parameter (`?` for SQLite/MySQL, `$1` for Postgres). On the first poll the cursor binds as `NULL`.

The dispatched event includes a `cursor` field that is **always serialized as a JSON string**, regardless of the underlying column type. Callers must parse it (e.g. `parseInt(event.cursor)`) when expecting numeric comparison.

### `iii-database::row-change`
Postgres only. Streams row-level changes via logical replication (`pgoutput`).

> **NOTE (v1.0.0):** Event dispatch is not yet functional. The publication and replication slot are created at startup, but the streaming decode loop is stubbed pending an upstream `tokio-postgres` replication API release. Operators can pre-provision slots and publications now; events will start flowing in a later release.

```yaml
triggers:
  - type: iii-database::row-change
    config:
      db: primary
      schema: public
      tables: [orders, payments]
```

The worker derives slot/publication names from `trigger_id`: `iii_slot_<sanitized>_<8hex>` and `iii_pub_<sanitized>_<8hex>`, where the 8-hex-char suffix is an FNV-1a-32 hash of the original `trigger_id`. The hash guarantees that two distinct trigger_ids (e.g. `orders-v1` vs `orders.v1`) produce distinct names even though both sanitize to `orders_v1`. The sanitized prefix is truncated at 40 chars so the final name fits in Postgres' 63-byte slot-name limit. Operators can override slot/publication names explicitly with `slot_name`/`publication_name`. Drop them with `pg_drop_replication_slot('<slot>')` and `DROP PUBLICATION <name>` if the worker is decommissioned without graceful shutdown.

## Errors

Returned `IIIError::Handler` bodies carry a stable `code` field:

| Code | Meaning |
|---|---|
| `POOL_TIMEOUT` | Pool acquire exceeded `acquire_timeout_ms`. |
| `QUERY_TIMEOUT` | Query exceeded `timeout_ms`. |
| `STATEMENT_NOT_FOUND` | Handle expired or unknown — re-prepare. |
| `UNKNOWN_DB` | `db` parameter doesn't match any configured database. |
| `INVALID_PARAM` | JSON value couldn't be coerced for the target driver. |
| `DRIVER_ERROR` | Wraps underlying driver error with `driver` and `inner_code` (nullable). `inner_code` format is per-driver: Postgres = SQLSTATE 5-char string (e.g. `42P01`), MySQL = server error number as string, SQLite = `rusqlite::ErrorCode` debug name. |
| `REPLICATION_SLOT_EXISTS` | Startup-only: another instance owns the slot. |
| `UNSUPPORTED` | Operation not supported on the chosen driver. |
| `CONFIG_ERROR` | Config parse, pool init, or trigger misconfiguration (e.g. `cursor_column` not in result). |

## Driver compatibility

A few operations are no-ops on certain drivers. They emit a `tracing::warn!` rather than an error:

| Operation | SQLite | Postgres | MySQL |
|---|---|---|---|
| `execute` with `returning: [...]` | ✓ | ✓ | warn-once + ignore |
| `transaction` `isolation: read_committed` / `repeatable_read` | warn + use serializable | ✓ | ✓ |
| `transaction` `isolation: serializable` | ✓ (`BEGIN IMMEDIATE`) | ✓ | ✓ |
| `iii-database::row-change` trigger | — | setup-only in v1.0.0 (see above) | — |

## Troubleshooting

- **Pool exhausted (`POOL_TIMEOUT`)**: bump `pool.max` or shorten the longest-running query. Live `prepareStatement` handles each pin one connection from the pool until they expire.
- **`STATEMENT_NOT_FOUND` from a long-lived handle**: handles are bounded to `ttl_seconds` (default 3600, max 86400). Re-prepare and retry.
- **SQLite write contention with `query-poll`**: enable WAL mode in your DB: `PRAGMA journal_mode=WAL;` once after creation.
- **Replication slot already exists**: another instance is consuming the slot. Either reuse the slot name or run `SELECT pg_drop_replication_slot('<slot>')`.

## License

MIT.
