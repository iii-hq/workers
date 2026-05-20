# database

> Connect to PostgreSQL, MySQL, and SQLite. Run queries, prepared statements, transactions, and subscribe to row-level change feeds.

| field | value |
|-------|-------|
| version | 1.0.0 |
| type | binary |
| supported_targets | x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu |
| author | iii |

## Install

```sh
iii worker add database@1.0.0
```

## Configure

Add a single `databases` block to your `config.yaml`. SQLite is the recommended starting point — no server, just a file:

```yaml
workers:
  - name: database
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
      ca_cert: /etc/ssl/internal-ca.pem    # optional; extends the system trust store
      trust_native: true                   # default true; set false to trust only ca_cert
  local:
    url: postgres://dev@localhost:5432/dev
    tls:
      mode: disable               # plaintext, local development only
```

- **`disable`** — plaintext. Local dev only.
- **`require`** (default) — encrypted; cert chain validated; hostname is **not** verified. Catches passive eavesdropping, doesn't catch a determined MITM with their own valid-chain cert.
- **`verify-full`** — encrypted; cert chain validated; cert hostname must match the URL host. Production default for managed services (RDS, Neon, Supabase).

`ca_cert` lets you point at a CA bundle for self-hosted databases or managed providers whose root isn't in the OS trust store. **Additive by default**: the supplied certs extend the system trust store rather than replacing it, so the same `TlsConfig` surface works for one database that needs a private CA and another that doesn't. Set `tls.trust_native: false` to switch to the strict-isolation posture (only the `ca_cert` certs trusted; the public web PKI is rejected). Postgres only — `mysql_async`'s rustls path always bundles `webpki_roots` and offers no upstream knob to suppress it.

#### Connecting to managed providers

**Supabase.** Every Supabase endpoint (direct, transaction pooler, session pooler) presents certificates signed by *Supabase Intermediate 2021 CA*, which is not in the OS trust store. By default `tls.mode: require` fails with `pool connection failed (tls)`. Download the CA from your project dashboard (or `https://supabase.com/downloads/prod-ca-2021.crt`) and point `tls.ca_cert` at it:

```yaml
databases:
  primary:
    url: postgresql://postgres.<project>:<password>@aws-0-<region>.pooler.supabase.com:6543/postgres
    tls:
      mode: verify-full
      ca_cert: /etc/ssl/supabase-prod-ca-2021.crt
```

`ca_cert` is additive — your existing CA pinning for other databases keeps working alongside this entry.

**Neon.** Drop `?sslmode=` and `?channel_binding=` from URLs copied out of the Neon dashboard, and configure TLS via the `tls` YAML block instead:

```yaml
databases:
  primary:
    url: postgres://user:pass@ep-xxx-pooler.<region>.aws.neon.tech/neondb
    tls:
      mode: require    # or verify-full
```

Neon's default `?channel_binding=require` cannot work through the pooler endpoint: TLS terminates at the pooler, so SCRAM-SHA-256-PLUS isn't advertised by the inner server, and `tokio-postgres` refuses to fall back. Leaving the URL param in surfaces as `pool connection failed (auth)`.

SQLite ignores the `tls` block (local-file driver).

## Quick start (SQLite)

```ts
import { call } from 'iii-sdk'

await call('database::execute', {
  db: 'primary',
  sql: 'CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, email TEXT)'
})

await call('database::execute', {
  db: 'primary',
  sql: 'INSERT INTO users (email) VALUES (?), (?)',
  params: ['a@x', 'b@x']
})

const { rows } = await call('database::query', {
  db: 'primary',
  sql: 'SELECT id, email FROM users ORDER BY id'
})
```

## Functions

| Function | Purpose |
|---|---|
| `database::query` | Read SQL. Returns `{ rows, row_count, columns }`. |
| `database::execute` | Write SQL. Returns `{ affected_rows, last_insert_id, returned_rows }`.<br>**`last_insert_id` semantics:** SQLite/MySQL surface the engine's `last_insert_rowid()` / `LAST_INSERT_ID()` (only populated for INSERT). Postgres has no equivalent — `last_insert_id` is set from the **first column of the first RETURNING row**, so put your PK first: `RETURNING id, name`, not `RETURNING name, id`. |
| `database::prepareStatement` | Pin a connection and return `{ handle: { id, expires_at } }`. |
| `database::runStatement` | Run a previously-prepared handle. (No `timeout_ms` — uses the pinned connection's session lifetime; configure via `ttl_seconds` on `prepareStatement`.) |
| `database::transaction` | Atomic batch sequence; rolls back on first failure. One-shot — pass all statements together. |
| `database::beginTransaction` | Open an interactive transaction. Returns `{ transaction: { id, expires_at } }`. Configurable `timeout_ms` (default 30 000, max 300 000) auto-rolls back if the deadline elapses. |
| `database::transactionQuery` | Read SQL inside an interactive transaction. Same envelope as `query`. |
| `database::transactionExecute` | Write SQL inside an interactive transaction. Same envelope as `execute`. Rejects bare `BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT`/`SET TRANSACTION` with `INVALID_PARAM` — finalize via the dedicated handlers below. |
| `database::commitTransaction` | Commit and finalize an interactive transaction. Subsequent calls against the same id return `TRANSACTION_NOT_FOUND`. |
| `database::rollbackTransaction` | Rollback and finalize an interactive transaction. Subsequent calls against the same id return `TRANSACTION_NOT_FOUND`. |

## Triggers

### `database::row-change`
Postgres only. Streams row-level changes via logical replication (`pgoutput`).

> **NOTE (v1.0.0):** Event dispatch is not yet functional. The publication and replication slot are created at startup, but the streaming decode loop is stubbed pending an upstream `tokio-postgres` replication API release. Operators can pre-provision slots and publications now; events will start flowing in a later release.

```yaml
triggers:
  - type: database::row-change
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
| `TRANSACTION_NOT_FOUND` | Transaction id unknown, already committed/rolled back, or timed out (auto-rolled-back by the watcher). |
| `UNKNOWN_DB` | `db` parameter doesn't match any configured database. |
| `INVALID_PARAM` | JSON value couldn't be coerced for the target driver, or transaction-control SQL was sent to `transactionExecute` (use `commitTransaction` / `rollbackTransaction`). |
| `DRIVER_ERROR` | Wraps underlying driver error with `driver` and `inner_code` (nullable). `inner_code` format is per-driver: Postgres = SQLSTATE 5-char string (e.g. `42P01`), MySQL = server error number as string, SQLite = `rusqlite::ErrorCode` debug name. Pool-acquire failures use the message form `pool connection failed (<class>)` where `<class>` is one of `tls`, `auth`, `network`, `server-policy`, or `unknown` — a redacted hint so untrusted callers can self-triage without seeing host/userinfo/db fragments. The full driver error is in the worker's stderr via `tracing::warn!`. |
| `REPLICATION_SLOT_EXISTS` | Startup-only: another instance owns the slot. |
| `UNSUPPORTED` | Operation not supported on the chosen driver. |
| `CONFIG_ERROR` | Config parse or pool init failure. |

## Driver compatibility

A few operations are no-ops on certain drivers. They emit a `tracing::warn!` rather than an error:

| Operation | SQLite | Postgres | MySQL |
|---|---|---|---|
| `execute` with `returning: [...]` | ✓ | ✓ | warn-once + ignore |
| `transaction` `isolation: read_committed` / `repeatable_read` | warn + use serializable | ✓ | ✓ |
| `transaction` `isolation: serializable` | ✓ (`BEGIN IMMEDIATE`) | ✓ | ✓ |
| `database::row-change` trigger | — | setup-only in v1.0.0 (see above) | — |


## Troubleshooting

- **Pool exhausted (`POOL_TIMEOUT`)**: bump `pool.max` or shorten the longest-running query. Live `prepareStatement` handles each pin one connection from the pool until they expire.
- **`STATEMENT_NOT_FOUND` from a long-lived handle**: handles are bounded to `ttl_seconds` (default 3600, max 86400). Re-prepare and retry.
- **`DRIVER_ERROR` "pool connection failed (...)"**: the parenthesized class tells you where to look.
    - `(tls)` — handshake or cert-chain failure. For managed providers (Supabase, self-signed corporate CAs), supply `tls.ca_cert`; see "Connecting to managed providers" above.
    - `(auth)` — credential or pg_hba/SCRAM rejection. Includes Neon's `?channel_binding=require` failing through the pooler endpoint (drop the URL param, use `tls.mode` in YAML).
    - `(network)` — TCP refuse, DNS, route, or peer reset. Check host/port reachability and any firewalls.
    - `(server-policy)` — server reachable and TLS+auth OK, but the server actively refused (e.g. `max_connections` exceeded, admin shutdown). Look at the worker stderr for the underlying driver message.
- **Replication slot already exists**: another instance is consuming the slot. Either reuse the slot name or run `SELECT pg_drop_replication_slot('<slot>')`.

## License

MIT.
