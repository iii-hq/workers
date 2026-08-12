# database

> Connect to PostgreSQL, MySQL, and SQLite. Run queries, prepared statements, transactions, and subscribe to row-level change feeds.

| field | value |
|-------|-------|
| type | binary |
| supported_targets | x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu |
| author | iii |

## Install

```sh
iii worker add database
```

## Skills

Install the `database` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill database
```

Browse or install every worker skill at once:

```bash
npx skills add iii-hq/workers --list
npx skills add iii-hq/workers --all
```

## Configure

Runtime settings live in the **`configuration` worker** under id **`database`**. The worker registers its JSON Schema at startup, reads the live value via `configuration::get`, and hot-reloads connection pools when the value changes.

Persisted values default to `./data/configuration/database.yaml` (fs adapter). Edit that file directly or call `configuration::set` — both propagate without a worker restart.

### Zero-config default

With no seed file and no stored configuration value, the worker uses a built-in default:

```yaml
databases:
  primary:
    url: sqlite:./data/iii.db
    pool:
      max: 10
      idle_timeout_ms: 30000
      acquire_timeout_ms: 5000
```

This is seeded into the `configuration` worker on first register and used as a runtime fallback when the stored value is `null`.

### Optional seed file

Pass `--config <path>` to supply a YAML seed file. When present, its `databases` block is passed as `initial_value` on `configuration::register` (overriding the built-in default for first-time registration). See [`config.yaml.example`](config.yaml.example).

Engine-managed deployments can inline config under the worker entry; the engine delivers it via `--config` as before.

### Value shape

SQLite is the recommended starting point — no server, just a file:

```yaml
databases:
  primary:
    url: sqlite:./data/iii.db
    pool:
      max: 10
      idle_timeout_ms: 30000
      acquire_timeout_ms: 5000
  analytics:
    url: ${ANALYTICS_URL:postgres://localhost/analytics}
    pool: { max: 5 }
history_max_entries: 200 # console query history caps, defaults shown
history_max_bytes: 262144 # 0 disables recording
```

`history_max_entries` / `history_max_bytes` cap the per-database console query history stored on the [`state`](../state) worker — whichever cap hits first, oldest entries are dropped. Applied live.

Set or replace the whole value:

```bash
iii trigger configuration::get id=database
iii trigger configuration::set id=database value='{"databases":{"primary":{"url":"sqlite:./data/iii.db"}}}'
```

Env placeholders use **`${VAR:default}`** syntax. The configuration worker expands them on every `configuration::get` call, so env changes propagate without a restart.

URL scheme picks the driver: `sqlite:`, `postgres://`, `postgresql://`, `mysql://`.

### Hot reload

When configuration changes (`configuration::set`, or an external edit to `./data/configuration/database.yaml`), the worker rebuilds connection pools in place. Invalid configs are rejected and the previous pools are kept. In-flight prepared-statement handles and open transactions continue on their original pool until they expire.

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
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')

await iii.trigger({
  function_id: 'database::execute',
  payload: {
    db: 'primary',
    sql: 'CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, email TEXT)',
  },
})

await iii.trigger({
  function_id: 'database::execute',
  payload: {
    db: 'primary',
    sql: 'INSERT INTO users (email) VALUES (?), (?)',
    params: ['a@x', 'b@x'],
  },
})

const { rows } = await iii.trigger({
  function_id: 'database::query',
  payload: {
    db: 'primary',
    sql: 'SELECT id, email FROM users ORDER BY id',
  },
})
```

## Functions

| Function | Purpose |
|---|---|
| `database::query` | Read SQL. Returns `{ rows, row_count, columns }`. |
| `database::execute` | Write SQL. Returns `{ affected_rows, last_insert_id, returned_rows }`.<br>**`last_insert_id` semantics:** SQLite/MySQL surface the engine's `last_insert_rowid()` / `LAST_INSERT_ID()` (only populated for INSERT). Postgres has no equivalent — `last_insert_id` is set from the **first column of the first RETURNING row**, so put your PK first: `RETURNING id, name`, not `RETURNING name, id`. |
| `database::executeBatch` | Convenience form of `transaction`: statements may be bare SQL strings or `{ sql, params }` objects (use `params` for dynamic values instead of inlining them). Same envelope and semantics as `transaction` — atomic, rolls back on first failure, reports `failed_index`, supports `isolation`. |
| `database::prepareStatement` | Pin a connection and return `{ handle: { id, expires_at } }`. |
| `database::runStatement` | Run a previously-prepared handle. (No `timeout_ms` — uses the pinned connection's session lifetime; configure via `ttl_seconds` on `prepareStatement`.) |
| `database::transaction` | Atomic batch sequence; rolls back on first failure. One-shot — pass all statements together. Rejects bare transaction-control SQL (`BEGIN`/`COMMIT`/`ROLLBACK`/…) and empty statements with `INVALID_PARAM`. |
| `database::beginTransaction` | Open an interactive transaction. Returns `{ transaction: { id, expires_at } }`. Configurable `timeout_ms` (default 30 000, max 300 000) auto-rolls back if the deadline elapses. |
| `database::transactionQuery` | Read SQL inside an interactive transaction. Same envelope as `query`. |
| `database::transactionExecute` | Write SQL inside an interactive transaction. Same envelope as `execute`. Rejects bare `BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT`/`SET TRANSACTION` with `INVALID_PARAM` — finalize via the dedicated handlers below. |
| `database::commitTransaction` | Commit and finalize an interactive transaction. Subsequent calls against the same id return `TRANSACTION_NOT_FOUND`. |
| `database::rollbackTransaction` | Rollback and finalize an interactive transaction. Subsequent calls against the same id return `TRANSACTION_NOT_FOUND`. |
| `database::listDatabases` | List configured databases. Returns `{ databases, count }`; each entry has `name`, `driver`, credential-redacted `url`, `pool` settings, and `tls` (`mode`, `ca_cert_present`, `trust_native`). Config only — no health checks or live pool stats. |

### Reading the schema

| Function | Description |
|---|---|
| `database::listTables` | Every table and view, with its kind and (postgres) its schema. |
| `database::describeTable` | One table: columns with type, nullability, default, primary-key membership and foreign-key target; plus indexes and a planner row estimate. Foreign keys are structured `{ schema, table, column }`, not a joined string. |
| `database::describeSchema` | The same shape for every table at once. One catalog query per aspect across the whole database rather than one call per table, so a 200-table schema costs a handful of queries. `include_indexes` is off by default. |
| `database::schemaDiagram` | Positioned table nodes and routed foreign-key edges, plus each table's hub `degree`, the `isolated` tables, and remaining edge `crossings`. Layout runs server-side, so a renderer only draws. |

### Reading data

| Function | Description |
|---|---|
| `database::browseTable` | Paged, sorted, filtered table read — no SQL from the caller. Filters are structured (`{ column, op, value }`) and compile to a parameterised `WHERE` for the driver in hand; `total` honours the same filters. Sorts accept a `mode` (`natural`, `length`, `absolute_value`, `random`) applied across the whole table, not just the page. To follow a foreign key, filter on equality with `page_size: 1`. |
| `database::explain` | The query plan as a tree with per-node cost, row estimates and warnings, instead of the driver's raw text. `analyze` collects real timings by **running** the statement, so it defaults to `false` and is refused for anything that is not a single read. |
| `database::columnStats` | Profile a table's columns. Reads the planner's own statistics by default — free and approximate, labelled `source: planner`. `exact: true` runs real aggregates and scans the table; it is refused above a row-count ceiling. To profile rows you already hold, pipe a `browseTable` result through the `fp` worker instead. |

### Operations

| Function | Description |
|---|---|
| `database::health` | Live pool occupancy plus active queries, table sizes, blocking locks and cache hit ratio. Each section reports separately as `available`, `unsupported` or `denied`, so a driver gap or a restricted role is never mistaken for an empty result. |
| `database::terminateQuery` | Terminate a backend session, or cancel just its statement with `cancel_only`. Takes an id from `health`. Separate from `health` because it is a write. |

### Saved queries and history

Stored in the [`state`](https://github.com/iii-hq/workers/tree/main/state) worker, scoped per database, so they survive restarts and any agent can read them.

| Function | Description |
|---|---|
| `database::saveQuery` | Save a named query. Saving under an existing name replaces it. |
| `database::listSavedQueries` | Saved queries for a database, sorted by name. |
| `database::deleteSavedQuery` | Delete by id or by name. |
| `database::history` | Recent queries, newest first. Best effort — recording never blocks or fails a query, so this is a convenience rather than an audit log. For an audit trail bind `database::row-changed`. Stored history is capped per database (`history_max_entries` / `history_max_bytes`, defaults 200 entries / 256KB — oldest dropped first, `0` disables) and holds metadata only: SQL text (truncated to 4000 chars), verb, timing, row count — never result rows. An oversized or unreadable stored value is replaced wholesale on the next write. |

## Triggers

### `database::row-changed`

Fires after this worker commits a row change. Driver-agnostic — no logical
replication, no per-database setup, identical on SQLite, Postgres and MySQL.

```yaml
triggers:
  - type: database::row-changed
    config:
      db: primary        # required
      table: orders      # optional; case- and schema-insensitive
      ops: [insert]      # optional; insert / update / delete / other
```

Event: `{ db, table, op, affected_rows, returning?, at }`, where `op` is
`insert` / `update` / `delete` / `other`.

**This is not change data capture.** It reports mutations made *through this
worker* — `execute`, `executeBatch`, `transaction`, and the interactive
transaction surface. A write applied by psql, another worker, or a
database-side trigger is invisible to it. That covers the case it exists for
(the worker is the only writer, and something needs to know when rows land)
and nothing more.

Four things worth knowing:

- **Announced on commit, never before.** Statements inside an interactive
  transaction are buffered until `commitTransaction`; a rollback — including
  the timeout watcher's — drops the buffer. Atomic batches announce their
  statements in order only after the whole batch commits.
- **Delivery is best-effort.** Dispatch happens after commit and is not durable
  or atomic with the database write. There is no replay, retry, or exactly-once
  guarantee; a crash between commit and dispatch can lose an event. Subscriber
  failures are logged and never fail the write.
- **`table` can be null.** The table is read off the SQL. A CTE-wrapped write
  (`WITH … INSERT`) still fires, with `table: null`, rather than being dropped;
  a binding that named a table simply does not match it. Omit `table` to match
  every write, including these.
- **`runStatement` does not fire.** The prepared-run path returns rows, not an
  affected-row count, and an event that invented one would be lying. Use
  `execute` when you need the change announced.

## Errors

Returned `IIIError::Handler` bodies carry a stable `code` field:

| Code | Meaning |
|---|---|
| `POOL_TIMEOUT` | Pool acquire exceeded `acquire_timeout_ms`. |
| `QUERY_TIMEOUT` | Query exceeded `timeout_ms`. |
| `STATEMENT_NOT_FOUND` | Handle expired or unknown — re-prepare. |
| `TRANSACTION_NOT_FOUND` | Transaction id unknown, already committed/rolled back, or timed out (auto-rolled-back by the watcher). |
| `UNKNOWN_DB` | `db` parameter doesn't match any configured database. |
| `INVALID_PARAM` | JSON value couldn't be coerced for the target driver, transaction-control SQL was sent to `transactionExecute` (use `commitTransaction` / `rollbackTransaction`), or a `transaction`/`executeBatch` batch contained transaction-control SQL or an empty statement. |
| `DRIVER_ERROR` | Wraps underlying driver error with `driver` and `inner_code` (nullable). `inner_code` format is per-driver: Postgres = SQLSTATE 5-char string (e.g. `42P01`), MySQL = server error number as string, SQLite = `rusqlite::ErrorCode` debug name. Pool-acquire failures use the message form `pool connection failed (<class>)` where `<class>` is one of `tls`, `auth`, `network`, `server-policy`, or `unknown` — a redacted hint so untrusted callers can self-triage without seeing host/userinfo/db fragments. The full driver error is in the worker's stderr via `tracing::warn!`. |
| `CONFIG_ERROR` | Config parse or pool init failure. |

## Driver compatibility

A few operations are no-ops on certain drivers. They emit a `tracing::warn!` rather than an error:

| Operation | SQLite | Postgres | MySQL |
|---|---|---|---|
| `execute` with `returning: [...]` | ✓ | ✓ | warn-once + ignore |
| `transaction` `isolation: read_committed` / `repeatable_read` | warn + use serializable | ✓ | ✓ |
| `transaction` `isolation: serializable` | ✓ (`BEGIN IMMEDIATE`) | ✓ | ✓ |


## Troubleshooting

- **Pool exhausted (`POOL_TIMEOUT`)**: bump `pool.max` or shorten the longest-running query. Live `prepareStatement` handles each pin one connection from the pool until they expire.
- **`STATEMENT_NOT_FOUND` from a long-lived handle**: handles are bounded to `ttl_seconds` (default 3600, max 86400). Re-prepare and retry.
- **`DRIVER_ERROR` "pool connection failed (...)"**: the parenthesized class tells you where to look.
    - `(tls)` — handshake or cert-chain failure. For managed providers (Supabase, self-signed corporate CAs), supply `tls.ca_cert`; see "Connecting to managed providers" above.
    - `(auth)` — credential or pg_hba/SCRAM rejection. Includes Neon's `?channel_binding=require` failing through the pooler endpoint (drop the URL param, use `tls.mode` in YAML).
    - `(network)` — TCP refuse, DNS, route, or peer reset. Check host/port reachability and any firewalls.
    - `(server-policy)` — server reachable and TLS+auth OK, but the server actively refused (e.g. `max_connections` exceeded, admin shutdown). Look at the worker stderr for the underlying driver message.

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
