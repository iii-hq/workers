# database

> Connect to PostgreSQL, MySQL, and SQLite. Run queries, prepared statements, transactions, and subscribe to row-level change feeds.

| field | value |
|-------|-------|
| type | binary |
| supported_targets | x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu |
| author | iii |

## Install

```sh
iii trigger compose::add worker=database
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
    tls: { mode: disable }    # a local server without TLS; see "TLS" below
    pool: { max: 5 }
history_max_entries: 200 # console query history caps, defaults shown
history_max_bytes: 262144 # 0 disables recording
```

`history_max_entries` / `history_max_bytes` cap the per-database console query history stored on the [`state`](../state) worker — whichever cap hits first, oldest entries are dropped. Applied live.

Env placeholders use **`${VAR:default}`** syntax. The configuration worker expands them on every `configuration::get` call, so env changes propagate without a restart.

The configuration id is `database`, or the name a supervisor passes in `III_CONFIG_NAME` (under compose: `<namespace>-<container>`, e.g. `default-database`). `database::listDatabases` reports it as `config_id`.

### Connection entries

Each entry under `databases` is **either** a connection `url` (its scheme picks the driver: `sqlite:`, `postgres://`, `postgresql://`, `mysql://`) **or** the structured form below — never both (`CONFIG_ERROR` names the offending field). The structured form is what connection editors (the console, the iii app) write:

```yaml
databases:
  shop:
    driver: postgres              # postgres | mysql | sqlite
    host: db.example.com          # postgres/mysql
    port: 5432                    # default 5432 / 3306
    user: app
    password: secret://DATABASE_SHOP_PASSWORD   # literal | secret://NAME | env://NAME
    database: shop                # sqlite: the file path on the engine host (or :memory:)
    tls:
      mode: verify-full
      ca_cert: secret://SHOP_CA   # path | inline PEM | secret://NAME | env://NAME
    display: { icon: database, color: "#22C55E" }   # how clients draw it (Lucide icon, #RRGGBB)
    folder: /home/me/shop/queries                   # absolute path of its .sql query files
    pool: { max: 10 }
    capture: statements
```

- `display` and `folder` are metadata for clients; the worker validates and stores them, never reads the folder. `folder` must be absolute; neither `folder` nor a sqlite `database` path may contain `..` segments.
- `pool.max` is 1–500, `pool.idle_timeout_ms` at most 24 h, `pool.acquire_timeout_ms` 1 ms–10 min (the schema and `saveConnection` refuse anything else).
- Connection names written through `database::saveConnection` are 1–64 letters, digits, `_`, `-` or `.`, starting with a letter, digit or `_`.
- A `url` names its server in the authority, once: postgres urls may not carry `host`, `hostaddr`, `port`, `service` or `password` parameters or list several hosts, mysql urls may not carry `socket` or `password` (`CONFIG_ERROR`). Put the password in `password` (or the request's `credentials`); for several hosts or a socket, use one entry per server.
- `saveConnection` and `testConnection` refuse keys an entry does not define (`pasword`, `tls.clientkey` — `CONFIG_ERROR` naming each path), so a mistyped credential is neither stored nor sent where its value would be traced. An entry already stored with unknown keys still loads; the worker logs a warning naming the paths (never their values).
- A connection that cannot be built (an unresolvable secret, an SSH failure, an unreadable certificate — or a hand-edited entry that does not parse or validate) does **not** fail the configuration: it gets no pool, `database::listDatabases` reports its `error` and `error_stage`, and its calls fail with `CONFIG_ERROR` naming the cause. Every other connection keeps working, and the worker retries it every 15 s.

### Secrets

`password`, `tls.ca_cert`, `tls.client_cert`, `tls.client_key`, `ssh.password`, `ssh.private_key` and `ssh.passphrase` accept a **reference** instead of a literal:

- `secret://NAME` — a value in the [`secrets`](../secrets) worker's encrypted vault;
- `env://NAME` — an environment variable the secrets worker reads (the project's `.env`, else its own environment).

Only a well-formed reference, or a `${NAME}` engine template **without** a default (`${NAME}` / `${NAME:}`), names a value. Anything else in a sensitive field is a literal credential: a malformed `secret://…`, and a template with a default (`${DB_PASSWORD:hunter2}` carries the password itself). `getConnection` returns those as `"__redacted__"` and the rules below refuse them inside `connection`. A url's password counts whether it is in the userinfo (percent-encoding decoded first) or a `?password=` parameter.

The worker resolves them through `secrets::resolve` as consumer **`database`**, so each secret must list `database` in its consumers (`secrets::set` / `secrets::access`). Resolved values are cached (re-read every 5 minutes, failures after 10 s), wiped from memory when replaced, and never logged, returned or written to the configuration. When a secret is rotated, deleted or re-shared, `secrets::changed` makes the worker re-read it and rebuild **only** the connections whose resolved values changed. The worker depends on the `secrets` worker (`iii.worker.yaml`).

**Literal credentials and traces.** A worker's trace spans record each call's payload, redacting keys that look sensitive (`password`, `private_key`, `secret`, `credential`, …) but not `passphrase`, `client_key` or a password inside `url`. So `database::testConnection` and `database::saveConnection` **refuse** a literal credential inside `connection` (a password, an inline key, a passphrase, a url password — `CONFIG_ERROR` naming the field): send it as a `secret://` / `env://` reference (safe anywhere), or in the request's **`credentials`** object — the same shape as the entry, sensitive fields only (`password`, `url`, `tls.client_key`, `ssh.password`, `ssh.private_key`, `ssh.passphrase`), merged over `connection` by the worker — which traces redact whole. Key file paths, `${VAR}` templates and `"__redacted__"` stay in `connection`. Responses never carry a credential (`getConnection` returns literals as `"__redacted__"`), no client can observe these three functions, and the configuration-change observer receives no configuration value (it re-reads it). What traces can still show: a literal that sits in the stored configuration itself (written by hand or by `configuration::set`) whenever another path ships the stored value — and a refused call's own payload: refusing happens inside the worker, after the engine traced the request, so a literal sent in `connection` by mistake is in that trace (redacted only when its key is on the SDK's list). Change that credential. A key file path stays in `connection` only while it looks like one (one line, at most 4096 bytes, not a pasted base64 key that happens to start with `/`). The lasting fix is upstream: the SDK's trace redaction (`iii-helpers`) should add `passphrase` and `client_key` to its key list and scrub url credentials.

**Where a credential may go.** `database::testConnection` (denied to agents) sends credentials only where they belong: a `"__redacted__"` value taken from the stored entry `name`, or a `secret://` / `env://` reference that a stored connection already uses, is accepted only when the candidate targets that connection's server — the same driver, every host, address and port the driver would try (read from the driver's own parse of the entry, not the url's text), the same SSH server and user, and an effective TLS mode (see the url `sslmode` note under TLS) no weaker than the stored one (for `ssh.*` fields: the same SSH server and user). Anything else is refused with `stage: "config"` before a connection is made; enter the value itself to test against a different server. References no stored connection uses (a new connection's fresh secret) are free.

An unresolvable reference fails only its connection, with a message that names the reference and what to do (`secret://X did not resolve: database is not allowed to read secret X; add database to the secret's consumers`), never a value.

### Hot reload

When configuration changes (`configuration::set`, `database::saveConnection`, or an external edit to `./data/configuration/database.yaml`), the worker rebuilds only the connections whose entry or resolved secrets changed; the others keep their pools, tunnels and open connections. Invalid configs are rejected and the previous pools are kept. In-flight prepared-statement handles and open transactions continue on their original pool until they expire.

### TLS (postgres + mysql)

Postgres and mysql connections default to **`tls.mode: require`** — TLS handshake required, certificate chain validated against the system trust store, hostname verification skipped (matches libpq's `sslmode=require`). Override per-database:

```yaml
databases:
  primary:
    url: postgres://app@db.example.com:5432/app
    tls:
      mode: verify-full           # disable | prefer | require | verify-ca | verify-full   (default: require)
      ca_cert: /etc/ssl/internal-ca.pem    # optional; extends the system trust store
      trust_native: true                   # default true; set false to trust only ca_cert
  local:
    url: postgres://dev@localhost:5432/dev
    tls:
      mode: disable               # plaintext, local development only
```

| Mode | Encrypted | Chain checked | Host name checked | Server without TLS |
|---|---|---|---|---|
| `disable` | no | — | — | connects |
| `prefer` | when offered | yes | no | connects in plaintext (a failed certificate check is still an error) |
| `require` (default) | yes | yes | no | error |
| `verify-ca` | yes | yes | no | error |
| `verify-full` | yes | yes | yes | error |

> **Upgrading from 0.5.21 or earlier.** On postgres, `require` (the default when an entry has no `tls` block) used to fall back to plaintext when the server offered no TLS: tokio-postgres connected with `sslmode=prefer` underneath. It now refuses, as the mode always claimed — that fallback is exactly what a man in the middle exploits. An entry that never set `tls` and talks to a server without TLS (typically a local or docker postgres) now fails with `pool connection failed (tls)`; add `tls: { mode: disable }` (trusted network) or `tls: { mode: prefer }` (the old behaviour), or write `?sslmode=disable` into the url. MySQL already refused plaintext under `require`. SQLite and the built-in default (`sqlite:./data/iii.db`) are unaffected.

`require` and `verify-ca` perform the same checks (libpq names them apart because libpq's `require` skips the chain; this worker never does). On postgres, a `sslmode` written into the url (`disable`, `prefer` or `require`) decides **whether** TLS is used, over `tls.mode`; `tls.mode` still decides the certificate checks whenever TLS is used (`sslmode=require` under `tls.mode: verify-full` checks the host name; `sslmode=disable` under any `tls.mode` is plaintext). Without a url `sslmode`, `tls.mode` decides both, and `require` really requires TLS (tokio-postgres alone would fall back to plaintext). `testConnection`'s credential binding compares this effective mode. `verify-full` is the production default for managed services (RDS, Neon, Supabase).

`ca_cert` lets you point at a CA bundle for self-hosted databases or managed providers whose root isn't in the OS trust store. **Additive by default**: the supplied certs extend the system trust store rather than replacing it. Set `tls.trust_native: false` to trust only the `ca_cert` certs. Postgres only — `mysql_async`'s rustls path always bundles `webpki_roots` and offers no upstream knob to suppress it.

**Mutual TLS.** `tls.client_cert` (the client certificate chain) and `tls.client_key` (its key) present a client certificate; both are required together. Each is a path on the engine host, the PEM itself, or a `secret://` / `env://` reference (a referenced value must be PEM text). Postgres takes PKCS#8, PKCS#1 and SEC1 keys; MySQL takes RSA keys only (PKCS#8 or PKCS#1) — `mysql_async` cannot present EC or Ed25519 client keys.

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

### SSH tunnel (postgres + mysql)

```yaml
databases:
  private:
    driver: postgres
    host: pg-private             # as the SSH server resolves it
    port: 5432
    user: shop
    password: secret://PRIVATE_PG_PASSWORD
    tls: { mode: verify-full, ca_cert: /etc/ssl/internal-ca.pem }
    ssh:
      host: bastion.example.com
      port: 22                   # default
      user: tunnel
      private_key: secret://BASTION_KEY     # OpenSSH or PEM (RSA, ECDSA, Ed25519): inline, a path, or a reference
      passphrase: secret://BASTION_KEY_PASSPHRASE
      password: secret://BASTION_PASSWORD   # optional; tried after the key
      host_key: "SHA256:0rGFgr3lbFUjBJmay4CvRxX16hnJLF7mvototEUXoW0"
```

The worker opens one SSH session per connection (pure Rust, no OpenSSL), listens on an ephemeral `127.0.0.1` port and forwards each pool connection through a `direct-tcpip` channel to `host:port` as the SSH server sees it; a dropped session is reopened on the next connection. TLS still runs end to end and verifies the **database** host name (`pg-private` above), not `127.0.0.1`. Works with the url form too (the url's host is the forwarding target).

- **Host key.** With `ssh.host_key` (the OpenSSH SHA-256 fingerprint, as `ssh-keygen -lf` prints it) any other key is refused, and the error names both fingerprints. Without it any key is accepted (trust on first use) and `database::testConnection` returns `ssh_host_key: {algorithm, fingerprint}` so a client can pin it.
- **Authentication.** `private_key` (with `passphrase` when encrypted) is tried first, then `password`. Errors name the SSH server and user, never a credential.
- The local listener accepts connections only on loopback; any process on the engine host could use it while the pool is up, as with `ssh -L`.

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
| `database::query` | Read SQL (in a read-only transaction). Returns `{ rows, row_count, columns, truncated, elapsed_ms }`. **`max_rows`** caps the rows returned: the worker stops reading after `max_rows + 1` (a postgres portal fetch, mysql's `sql_select_limit`, sqlite's lazy stepping — the SQL is never rewritten) and `truncated` says whether more existed. `elapsed_ms` is the time the statement took, pool acquisition excluded. One statement per call: a sqlite read with a second statement after `;` is refused (`MULTI_STATEMENT`). |
| `database::execute` | Write SQL. Returns `{ affected_rows, last_insert_id, returned_rows, elapsed_ms }`.<br>**`returned_rows`** come from a `RETURNING` clause written into the SQL (SQLite, Postgres — MySQL has none): `INSERT INTO t (n) VALUES (?) RETURNING id`. The `returning: [...]` option never adds the clause; SQLite rejects it without one, Postgres and MySQL ignore it.<br>**`last_insert_id` semantics:** SQLite/MySQL surface the engine's `last_insert_rowid()` / `LAST_INSERT_ID()` (only populated for INSERT). Postgres has no equivalent — `last_insert_id` is set from the **first column of the first RETURNING row** of an INSERT, so put your PK first: `RETURNING id, name`, not `RETURNING name, id`. |
| `database::executeBatch` | Convenience form of `transaction`: statements may be bare SQL strings or `{ sql, params }` objects (use `params` for dynamic values instead of inlining them). Same envelope and semantics as `transaction` — atomic, rolls back on first failure, reports `failed_index`, supports `isolation`. |
| `database::prepareStatement` | Pin a connection and return `{ handle: { id, expires_at } }`. |
| `database::runStatement` | Run a prepared SQL statement by handle. (No `timeout_ms` — uses the pinned connection's session lifetime; configure via `ttl_seconds` on `prepareStatement`.) |
| `database::transaction` | Atomic batch sequence; rolls back on first failure. One-shot — pass all statements together. Rejects bare transaction-control SQL (`BEGIN`/`COMMIT`/`ROLLBACK`/…) and empty statements with `INVALID_PARAM`. |
| `database::beginTransaction` | Open an interactive transaction. Returns `{ transaction: { id, expires_at } }`. Configurable `timeout_ms` (default 30 000, max 300 000) auto-rolls back if the deadline elapses. |
| `database::transactionQuery` | Read SQL inside an interactive transaction. Same envelope as `query`. |
| `database::transactionExecute` | Write SQL inside an interactive transaction. Same envelope as `execute`. Rejects bare `BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT`/`SET TRANSACTION` with `INVALID_PARAM` — finalize via the dedicated handlers below. |
| `database::commitTransaction` | Commit an open SQL transaction, making its writes permanent. Subsequent calls against the same id return `TRANSACTION_NOT_FOUND`. |
| `database::rollbackTransaction` | Roll back an open SQL transaction, discarding its uncommitted writes. Subsequent calls against the same id return `TRANSACTION_NOT_FOUND`. |

### Connections

| Function | Description |
|---|---|
| `database::listDatabases` | Configured connections: `{ databases, count, config_id }`. Each entry: `name`, `driver`, credential-redacted `url`, `host`, `port`, `user`, `database`, `display`, `folder`, `ssh` (`host`, `port`, `user`, `host_key_pinned`), `pool`, `tls` (`mode`, `ca_cert_present`, `client_cert_present`, `trust_native`), `uses_secrets`, and `error` / `error_stage` when the connection could not be built. Config only — no secrets, no health checks. |
| `database::getConnection` | `{ name }` → `{ name, connection }`: the entry as stored, `${VAR}` templates and `secret://` references intact; a literal password, passphrase, inline private/client key or url password comes back as `"__redacted__"`. |
| `database::saveConnection` | `{ name, previous_name?, connection, credentials? }` → `{ name, saved, error?, error_stage? }`. Creates (`previous_name` absent; a taken name is refused), updates (`previous_name` = `name`) or renames (`previous_name` = the old name). A field still holding `"__redacted__"` keeps its stored value. Read-modify-write of the whole configuration through `configuration::set` (raw, so other entries' templates survive); returns after the pools reloaded, with the saved connection's build error, if any (it is saved either way). |
| `database::deleteConnection` | `{ name }` → `{ deleted }`. The last connection cannot be removed. |
| `database::testConnection` | Probe a candidate entry before saving (people only — agents are denied it): `{ connection, credentials?, name?, timeout_ms? }` (legacy `{ url, tls }` still works, a url password going in `credentials.url`; `name` resolves `"__redacted__"` from the stored entry, for the same server only — see "Where a credential may go"). One throwaway connection (and tunnel); secrets resolve like a saved connection's. Returns `{ ok, driver, latency_ms, server_version?, message?, stage?, ssh_host_key? }` — `stage` is where it failed: `config`, `secrets`, `ssh`, `tcp`, `tls`, `auth` or `database`. Failures are data, not errors. |

### Reading the schema

| Function | Description |
|---|---|
| `database::listTables` | Every table and view, with its kind, (postgres) its schema, and `row_count_estimate` where the catalog keeps one cheaply — postgres `reltuples` (tables and materialized views, once analyzed), mysql `TABLE_ROWS`; absent on sqlite and for views. An estimate, not a count. |
| `database::describeTable` | One table: columns with type, nullability, default, primary-key membership, foreign-key target, **`auto_increment`** (the database fills it in: postgres identity or a `nextval(…)` default such as `serial`, mysql `AUTO_INCREMENT`, a sqlite `INTEGER PRIMARY KEY` — the rowid — outside `WITHOUT ROWID` tables) and **`generated`** (computed, cannot be written: postgres `GENERATED ALWAYS AS … STORED`, mysql virtual/stored generated columns — not `DEFAULT CURRENT_TIMESTAMP` — and sqlite generated columns); plus indexes and a planner row estimate. Foreign keys are structured `{ schema, table, column }`, not a joined string. An unqualified postgres name is the table the search_path finds (`orders` is `public.orders` even when `analytics.orders` exists). |
| `database::describeSchema` | The same shape for every table at once. One catalog query per aspect across the whole database rather than one call per table, so a 200-table schema costs a handful of queries. `include_indexes` is off by default. |
| `database::schemaDiagram` | Positioned table nodes and routed foreign-key edges, plus each table's hub `degree`, the `isolated` tables, and remaining edge `crossings`. Layout runs server-side, so a renderer only draws. |

### Reading data

| Function | Description |
|---|---|
| `database::browseTable` | Paged, sorted, filtered table read — no SQL from the caller. Filters are structured (`{ column, op, value }`) and compile to a parameterised `WHERE` for the driver in hand; `total` honours the same filters. Values compare as the column's type: on postgres each value (`value`, `value2`, every `values` entry) is bound as text and cast to the column's catalog type (`$n::text::<type>`), so `total gt 100` on a `numeric`, a date, a timestamptz, a uuid, an inet, a boolean, an enum or an int8 means what it says, numbers may be strings, and a value the type cannot read is an error (never a match-everything); a filter on a column the table does not have is `INVALID_PARAM`. Pattern operators (`contains`, `starts_with`, …) on a non-text column compare its text form. mysql and sqlite compare by their own coercion rules (numeric for number columns). Sorts accept a `mode` (`natural`, `length`, `absolute_value`, `random`) applied across the whole table, not just the page. To follow a foreign key, filter on equality with `page_size: 1`. A filter may instead be **`{ raw: "<SQL boolean expression>" }`**, ANDed in its own parentheses (`column`/`op` ignored). A raw filter can never write: it is refused when it could leave its parentheses (a `;`, a comment, an unbalanced `)`, a placeholder, or a backslash whose meaning depends on server settings — quotes, dollar quotes, backticks and sqlite brackets are lexed like the server lexes them), and it runs in a read-only transaction (postgres, mysql) or under `PRAGMA query_only` (sqlite) through the single-statement protocol. Returns `elapsed_ms` too. |
| `database::distinctValues` | `{ table, schema?, column, search?, limit? }` → `{ values: [{ value, count }], truncated, elapsed_ms }`: a column's distinct values, most frequent first (`limit` 1–500, default 50; 0 is refused). `search` keeps values whose text form contains it, case-insensitively and literally. For filter pickers. |
| `database::formatSql` | `{ sql, db?, driver?, uppercase?, indent? }` → `{ sql }`: pretty-printed for the driver's dialect — one of `db` or `driver` is required (literal and identifier quoting differ per dialect) (keywords uppercased by default, 2-space indent); statements and comments preserved, literal bytes (strings, quoted identifiers, dollar-quoted bodies, sqlite `[identifiers]`) never change. Refused: NUL bytes, parentheses nested more than 100 deep, and input that could format to more than 8 MB. Nothing runs. |
| `database::explain` | The query plan as a tree with per-node cost, row estimates and warnings, instead of the driver's raw text. `analyze` collects real timings by **running** the statement, so it defaults to `false` and is refused for anything that is not a single read. |
| `database::columnStats` | Profile a table's columns. Reads the planner's own statistics by default — free and approximate, labelled `source: planner`. `exact: true` runs real aggregates and scans the table; it is refused above a row-count ceiling. To profile rows you already hold, pipe a `browseTable` result through the `fp` worker instead. |

### Writing rows

| Function | Description |
|---|---|
| `database::insertRow` | `{ table, schema?, values: { column: value } }` → `{ row, affected_rows, elapsed_ms }`. Columns left out take their default. Each value is converted to its column's catalog type: postgres binds text and casts it (`$n::text::<type>`), so numbers as strings, booleans, uuids, dates, json and arrays (from a JSON array) all parse; bytea/blob/binary columns take base64; mysql/sqlite booleans become 1/0. On integer and decimal columns a JSON number at or above 2^53 in magnitude is refused (it may already have lost digits in the client — send it as a string), and an integer column takes only decimal digit strings (`"0x10"`, `"1e3"` are refused). `row` is the inserted row (`RETURNING *` on postgres/sqlite; on mysql a re-select by the auto-increment id or the supplied primary key, else null). Fires `database::row-changed` like `execute`. |

### Operations

| Function | Description |
|---|---|
| `database::health` | Check live pool occupancy plus active queries, table sizes, blocking locks and cache hit ratio. Each section reports separately as `available`, `unsupported` or `denied`, so a driver gap or a restricted role never reads as an empty result. |
| `database::terminateQuery` | Terminate a backend session, or cancel just its statement with `cancel_only`. Takes an id from `health`. Separate from `health` because it is a write. |

### Saved queries and history

Stored in the [`state`](https://github.com/iii-hq/workers/tree/main/state) worker, scoped per database, so they survive restarts and any agent can read them.

| Function | Description |
|---|---|
| `database::saveQuery` | Save a named query. Saving under an existing name replaces it. |
| `database::listSavedQueries` | Saved queries for a database, sorted by name. |
| `database::deleteSavedQuery` | Delete by id or by name. |
| `database::history` | Recent queries, newest first. Best effort — recording never blocks or fails a query, so this is a convenience rather than an audit log. For an audit trail bind `database::row-changed`. Stored history is capped per database (`history_max_entries` / `history_max_bytes`, defaults 200 entries / 256KB — oldest dropped first, `0` disables) and holds metadata only: SQL text (truncated to 4000 chars), verb, timing, row count — never result rows. An oversized or unreadable stored value is replaced wholesale on the next write. |

### Chat mentions

The worker is a [mention provider](../crates/mention-contract) named `database`: `@database(id="<connection>")` in a chat renders as a pill reading `@<connection>` (the item's `hint` is the connection name; `description` is `<driver> · host:port/db`) with a preview card (driver, server, SSH, folder, status) and reaches the agent as a summary saying which `db` to pass (`database::query {db: "<connection>", sql}`, `database::listTables {db: "<connection>"}`). `database::mention::search` / `database::mention::get` are internal; agents read the connection through `database::listTables` (the provider's `details`).

## Cell values

Every driver returns rows as JSON objects. Values are encoded by column type, so a client parses each type one way:

| Column type | JSON |
|---|---|
| integers (`int2`, `int4`, `smallint`, `int`, `mediumint`, `year`, sqlite `INTEGER`), mysql `bigint` | number |
| postgres `int8`/`bigint`, `numeric`/`decimal` (all drivers but sqlite), `money`, mysql `bigint unsigned` | string (exact) |
| `real`, `double`, `float` | number; postgres `NaN`/`Infinity` as strings |
| `boolean` | boolean on postgres; `0`/`1` on mysql (`tinyint(1)`) and sqlite |
| text types, mysql `enum`/`set` (`"a,b"`), postgres enums, `uuid`, `xml`, `inet`, `cidr`, `macaddr`, `point`, `tsvector`, ranges | string (postgres `char(n)` keeps its padding) |
| `json`, `jsonb` (sqlite: columns declared `JSON`) | the JSON value |
| `bytea`, `blob`, `binary`, `varbinary`, `geometry` | base64 string |
| `date` | `"YYYY-MM-DD"` |
| `time` | `"HH:MM:SS[.ffffff]"` (mysql durations past 24 h or negative as mysql prints them); postgres `timetz` adds the offset (`"12:30:00+03"`) |
| `timestamp`, `timestamptz`, `datetime` | RFC 3339 with `Z`, fraction when non-zero (a zone-less value is read as UTC); postgres `infinity` as a string |
| `interval` | postgres text form (`"1 year 2 mons 3 days 04:05:06"`) |
| `bit(n)` | string of `0`/`1` |
| postgres arrays | JSON arrays of the element encoding (nested for several dimensions) |
| postgres composites / domains | JSON object by field name / the base type's encoding |
| anything else (extension types) | its bytes as text when printable, else `"\\x…"` hex |

Parameters (`params`, `insertRow` values) take the same forms back. Integers beyond 2^53 are exact when sent as JSON numbers that fit 64 bits **or as numeric strings** — send them as strings from JavaScript, whose numbers lose precision past 2^53. On postgres a JSON array bound to an array parameter (`$1::int[]`) is that array (nested arrays are dimensions).

sqlite returns what each value is stored as (its affinity): `NUMERIC` columns may hold integers or reals, and dates are whatever text was stored. A cell that cannot be decoded never fails the query.

## Triggers

### `database::row-changed`

Fires after a row change commits. Two capture modes, chosen per database in
the worker config (`capture:`):

- **`statements`** (default) — reports mutations made *through this worker*
  (`execute`, `executeBatch`, `transaction`, and the interactive transaction
  surface) by classifying the SQL. A write applied by psql, another worker, or
  a database-side trigger is invisible to it. No database setup; identical on
  SQLite, Postgres and MySQL.
- **`native`** — reports committed writes from *any* client. Postgres: per-table
  triggers + LISTEN/NOTIFY (the role needs TRIGGER on the table). File-backed
  SQLite: triggers feeding a changelog table, drained on filesystem wake-up.
  MySQL: the binlog replication stream (`REPLICATION SLAVE, REPLICATION CLIENT`
  grants, `binlog_row_metadata=FULL` for row identity). Bindings must name a
  `table`.

```yaml
triggers:
  - type: database::row-changed
    config:
      db: primary        # required
      table: orders      # optional on statements, required on native; case- and schema-insensitive
      ops: [insert]      # optional; insert / update / delete / other
```

Event: `{ db, table, op, affected_rows, returning?, truncated?, at }`, where
`op` is `insert` / `update` / `delete` / `other`. The full schema is on the
trigger type (`engine::triggers::info`).

**Which rows changed — `returning`.** The event never invents row data:

- On `statements` it is the writer's own `RETURNING` projection, verbatim. A
  listener that needs to know *which* row changed depends on the writer having
  written `RETURNING <primary key>` into its SQL — the `returning` option on
  `execute` does not add the clause. Without it the event carries only
  `affected_rows`. `UPDATE` reports the new values, `DELETE` the removed row.
  MySQL has no `RETURNING`, so statements-mode events there never carry rows.
- On `native` it is the primary-key columns of each changed row (every column
  of a composite key; `UPDATE` reports the new key), so a listener gets row
  identity without the writer's cooperation. Capped at 100 rows per event —
  `truncated: true` says the rest were dropped, `affected_rows` stays exact.
  Absent for a table without a primary key. Values are encoded as
  `database::query` returns the same column (Postgres `bigint` as a string,
  for example).
- When not to bind: if the writer is you and already holds the new values in
  `execute`'s `returned_rows`, there is nothing to wait for.

Worth knowing:

- **Announced on commit, never before.** Statements inside an interactive
  transaction are buffered until `commitTransaction`; a rollback — including
  the timeout watcher's — drops the buffer. Atomic batches announce their
  statements in order only after the whole batch commits. Native capture is
  commit-gated by the database itself.
- **Delivery is best-effort.** Dispatch happens after commit and is not durable
  or atomic with the database write. There is no replay, retry, or exactly-once
  guarantee; a crash between commit and dispatch can lose an event. Subscriber
  failures are logged and never fail the write. A harness wake notification
  renders the event as JSON within ~8,000 characters; an over-long `returning`
  keeps its leading rows and ends with one marker string, `"…N more entries
  omitted (array shortened for this notification; JSON is complete)"` (N is
  what the wake dropped, on top of any `truncated` cap above), so the text
  still parses and `affected_rows` stays exact. Only an event whose
  non-array fields alone exceed the budget is cut as plain text with an
  `…(truncated)` tail. Re-read the table on wake when a binding can see wide
  or many-row writes.
- **`table` can be null on `statements`.** The table is read off the SQL. A
  CTE-wrapped write (`WITH … INSERT`) still fires, with `table: null`, rather
  than being dropped; a binding that named a table simply does not match it.
  Omit `table` to match every write, including these. The classifier reads
  keywords rather than parsing, so any `WITH` statement whose text contains
  `INSERT`/`UPDATE`/`DELETE`/`MERGE` — even inside a string literal — fires
  the same way.
- **`runStatement` does not fire on `statements`.** The prepared-run path
  returns rows, not an affected-row count, and an event that invented one would
  be lying. Use `execute` when you need the change announced. On a `native`
  database the database itself reports the write, whichever path it took.
- **Native caveats.** Postgres additionally trims keys to fit NOTIFY's
  8000-byte payload. SQLite's triggers call `json_object`, so every process
  writing the file needs libsqlite ≥ 3.38 (JSON built in) — an older client
  fails its own INSERT with `no such function`. MySQL carries keys only under
  `binlog_row_metadata=FULL`; on the `MINIMAL` default the events are
  count-only and registration warns.

## Isolation between callers

- **Postgres** pooled connections are reset when they come back to the pool (`RESET ALL`, `UNLISTEN *`, `pg_advisory_unlock_all()`, temporary tables and sequences, open cursors), so one call's `set_config(…, false)` or advisory lock never reaches the next caller. Reads run in read-only transactions.
- **Postgres prepared statements.** Session-level `PREPARE` / `EXECUTE` / `DEALLOCATE` are refused in every statement a call carries — `query`, `execute`, `transaction` / `executeBatch`, the interactive transaction calls and `prepareStatement` (`INVALID_PARAM`): a named statement would outlive the call on the pooled session (the reset cannot `DEALLOCATE ALL`: the driver's own cached statements live there too), where the next caller could run it. Use `prepareStatement` / `runStatement`, which pin a connection.
- **SQLite** connections refuse `ATTACH` (and so `VACUUM`, which attaches underneath): a call can only ever see and write its own database file. They also refuse setting `writable_schema`, `schema_version`, `ignore_check_constraints`, `trusted_schema`, `legacy_alter_table`, `temp_store_directory` and `data_store_directory` (reading them is fine), and run with double-quoted string literals off, so `"name"` is always an identifier — a misspelt column is an error, not the string `'name'`.
- **`saveConnection` is not bound to a server.** `testConnection` sends a stored credential only to that connection's own server, but `saveConnection` writes whatever entry it is given: a client allowed to call it can save an entry that points a `secret://` reference the `database` consumer can read at a server of its choosing, and the next connection attempt delivers it. That is the function's purpose (connection editors save new servers), so it is gated by permission instead: agents are denied `saveConnection`, `deleteConnection`, `getConnection` and `testConnection` (`iii-permissions.yaml`); grant them through RBAC only to clients you would let manage your connections, and share secrets with the `database` consumer only when every such client may use them.
- **Observers.** `database::query`, `execute`, `browseTable`, `insertRow` and the other data functions are ordinary observable functions: an observer capturing `input` / `output` sees other clients' SQL and rows — the same reach as calling `database::query` itself, so grant `observe` on them as you would grant the call. `testConnection`, `saveConnection` and `getConnection` refuse every observer.

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
| `DRIVER_ERROR` | Wraps underlying driver error with `driver` and `inner_code` (nullable). `inner_code` format is per-driver: Postgres = SQLSTATE 5-char string (e.g. `42P01`), MySQL = server error number as string, SQLite = `rusqlite::ErrorCode` debug name. Pool-acquire failures use the message form `pool connection failed (<class>)` where `<class>` is one of `tls`, `auth`, `network`, `ssh`, `server-policy`, or `unknown` — a redacted hint so untrusted callers can self-triage without seeing host/userinfo/db fragments. The full driver error is in the worker's stderr via `tracing::warn!`. |
| `CONFIG_ERROR` | Config parse or pool init failure; a connection that is configured but unavailable (`connection \`x\` is unavailable (secrets): secret://X did not resolve: …`); a `saveConnection` / `deleteConnection` the worker refused (validation, duplicate name, `"__redacted__"` with nothing stored). Messages name fields and references, never values. |

## Driver compatibility

A few operations are no-ops on certain drivers. They emit a `tracing::warn!` rather than an error:

| Operation | SQLite | Postgres | MySQL |
|---|---|---|---|
| `execute` / `transactionExecute` with a `RETURNING` clause in the SQL | ✓ rows returned | ✓ rows returned | no RETURNING in MySQL |
| `execute` `returning: [...]` option (never adds the clause) | rejected without a clause (`RETURNING_MISMATCH`) | accepted, no effect (warn) | accepted, no effect (warn) |
| `transaction` `isolation: read_committed` / `repeatable_read` | warn + use serializable | ✓ | ✓ |
| `transaction` `isolation: serializable` | ✓ (`BEGIN IMMEDIATE`) | ✓ | ✓ |


## Troubleshooting

- **Pool exhausted (`POOL_TIMEOUT`)**: bump `pool.max` or shorten the longest-running query. Live `prepareStatement` handles each pin one connection from the pool until they expire.
- **`STATEMENT_NOT_FOUND` from a long-lived handle**: handles are bounded to `ttl_seconds` (default 3600, max 86400). Re-prepare and retry.
- **`DRIVER_ERROR` "pool connection failed (...)"**: the parenthesized class tells you where to look.
    - `(tls)` — handshake or cert-chain failure. For managed providers (Supabase, self-signed corporate CAs), supply `tls.ca_cert`; see "Connecting to managed providers" above.
    - `(auth)` — credential or pg_hba/SCRAM rejection. Includes Neon's `?channel_binding=require` failing through the pooler endpoint (drop the URL param, use `tls.mode` in YAML).
    - `(network)` — TCP refuse, DNS, route, or peer reset. Check host/port reachability and any firewalls. Through an SSH tunnel: the SSH server could not reach the database host.
    - `(ssh)` — the connection's SSH tunnel failed (server unreachable, authentication, host key). The worker log names the reason; `database::testConnection` reports it with `stage: "ssh"`.
    - `(server-policy)` — server reachable and TLS+auth OK, but the server actively refused (e.g. `max_connections` exceeded, admin shutdown). Look at the worker stderr for the underlying driver message.

## License

Apache 2.0 — see [LICENSE](https://github.com/iii-hq/workers/blob/main/LICENSE).
