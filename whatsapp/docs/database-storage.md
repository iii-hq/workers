# Database worker storage

The WhatsApp executable stores native whatsmeow keys, message archives, Cloud
delivery receipts and webhook deduplication markers through the existing iii
database worker. It does not link SQLite. The worker remains a Go executable;
the database worker owns the SQLite connection and persistence.

Local `data_dir` still contains session metadata, the process lock, development
tunnel preferences and immutable legacy database backups after migration.
Existing session IDs, metadata and Cloud phone IDs are preserved. Starting the
worker does not connect a native WhatsApp account or send a message.

## Configuration

Create a **dedicated** SQLite database in the database worker's configuration.
Add this entry to its existing `databases` map; keep unrelated entries intact:

```yaml
databases:
  whatsapp:
    url: sqlite:/absolute/private/path/whatsapp.db
    pool:
      max: 1
      idle_timeout_ms: 30000
      acquire_timeout_ms: 5000
```

The database worker's own `III_CONFIG_NAME` determines its configuration ID.
In the current development session it is `my-project-database`, with functions
registered in `my-project`. The default ID is `database` when no override exists.
Do not replace its complete configuration with the example above.

Create the parent directory with mode `0700`, run the database worker with
`umask 077`, and back up this database along with WhatsApp's local metadata.
Native keys require the same protection as paired-device credentials.
Set `III_DISABLE_TRACE_PAYLOADS=1` on the **database worker**, as WhatsApp already
does for its own SDK. SQL statements use bound parameters; the database worker's
SQL history records statement text rather than key bytes or message values.
If query history is not needed, set `history_max_entries: 0` and
`history_max_bytes: 0` in the database configuration.

WhatsApp configuration:

```yaml
database:
  name: whatsapp
  namespace: my-project
data_dir: data/whatsapp-go
```

`database.name` defaults to `whatsapp`. Empty `database.namespace` uses the
WhatsApp worker's namespace. The ADE configuration form exposes both fields.
Changes to `database` require a restart and do not copy an existing database
to a new target. Configure and start the database worker first. The Compose
definition declares that startup dependency and disables database payload traces.

SQLite 3.41 or later with `unhex()` is required ([SQLite 3.41 release notes](https://sqlite.org/releaselog/3_41_0.html)); the installed database worker
uses SQLite 3.45.0. PostgreSQL/MySQL are not supported by this adapter. The
single-connection pool preserves SQLite connection settings used by the pinned
whatsmeow SQL store. Foreign keys and secure deletion are enabled around writes
and transactions. Increasing the pool size is rejected rather than risking
incorrect schema upgrades or lost key cleanup.

## Isolation and SQL contracts

Each store has a deterministic table prefix derived from the WhatsApp
namespace, `III_CONFIG_NAME`, account scope and store kind:

```text
wa_<128-bit SHA-256 prefix>_inbox_messages
wa_<different prefix>_whatsmeow_device
```

Native sessions use `session:<session_id>` and separate `inbox` / `device`
stores. Cloud archives use `cloud:<phone_number_id>` and `inbox`. Namespaces,
configuration IDs, sessions and transports therefore cannot select each
other's tables. Keep namespace and configuration ID stable across upgrades.
Run only one WhatsApp process for a given namespace/configuration ID; use
different identities for independent deployments. Local file locks prevent
two processes from opening the same `data_dir` on one host.

The adapter uses the installed contracts directly:

```json
{"function":"database::query","request":{"db":"whatsapp","sql":"SELECT text FROM wa_<scope>_inbox_messages WHERE message_id=?1","params":["wamid.example"]}}
```

```json
{"function":"database::execute","request":{"db":"whatsapp","sql":"INSERT INTO wa_<scope>_whatsmeow_sessions(our_jid,their_id,session) VALUES(?1,?2,unhex(?3))","params":["15550000001@s.whatsapp.net","peer","00ff2a"]}}
```

Binary values use hex **bound parameters** and SQLite `unhex()`, because the
database worker's current parameter contract has no binary input type. Returned
`bytea`/`BLOB` columns are decoded from the worker's base64 representation.
JSON numbers are decoded without losing integer precision above `2^53`.

Transactions use `database::beginTransaction`, `transactionQuery`,
`transactionExecute`, and `commitTransaction` / `rollbackTransaction`. The
transaction ID from `response.transaction.id` pins every statement to the same
connection. No database mutation or commit is automatically retried. A lost
commit acknowledgement returns an unknown-outcome error; a best-effort rollback
only releases an uncommitted transaction and does not replay any writes.
Database error details are suppressed to keep keys and message values out of
WhatsApp results. Prepared `database/sql` statements do not create long-lived
database-worker prepared handles.

Session deletion clears only that session's device and inbox rows. Logout clears
device rows, including independent LID maps, and retires its migrated local
credential backup. Schema versions remain so a recreated session can safely
reuse its tables. A stopped session retains its persisted keys.

## Migrate existing Go storage

Migration is explicit. If a nonempty legacy `inbox.db` or `device.db` exists,
the new worker refuses to open an empty remote store. It requires a committed
migration marker whose source fingerprint matches that file. The migration
command refuses a destination that already contains scoped tables.

1. Stop the old WhatsApp worker and back up its complete `data_dir`. Keep the
   database worker running with the dedicated pool configured. Close/checkpoint
   active journals first; the tool refuses nonempty WAL/journal files.
2. Build the separate migration executable. It uses the existing Go dependency,
   requires no new library installation, and is not part of the runtime worker:

   ```sh
   GOWORK=off make migrate
   ```

3. Inspect the inventory. Use the actual runtime namespace and configuration ID:

   ```sh
   ./bin/whatsapp-migrate \
     --data-dir /absolute/path/to/data/whatsapp-go \
     --namespace my-project \
     --config-name my-project-whatsapp-go \
     --database whatsapp \
     --database-namespace my-project
   ```

4. Repeat with `--apply` to copy those stores:

   ```sh
   ./bin/whatsapp-migrate \
     --engine ws://127.0.0.1:49134 \
     --data-dir /absolute/path/to/data/whatsapp-go \
     --namespace my-project \
     --config-name my-project-whatsapp-go \
     --database whatsapp \
     --database-namespace my-project \
     --apply
   ```

The tool acquires the worker's file lock, opens source databases read-only, and
copies each store in a transaction. It preserves tables, indexes, message row
IDs, AUTOINCREMENT high-water marks, keys, receipts and deduplication markers.
It verifies row counts and the unchanged source fingerprint before commit.
Transfers are batched to at most 100 rows / 900 parameters / approximately
256 KiB; a single row estimated above 1 MiB is refused with rollback. The
database worker limits a transaction to five minutes. Successful stores can be
rerun safely; partial failed stores roll back. No credentials or message bodies
are printed, and source files remain in place for recovery.

5. Build/start the new WhatsApp executable using the same `data_dir`, namespace
   and configuration ID. Validate archive reads and stopped-session listings
   before explicitly starting a paired account.

Do not run the old SQLite-writing executable after migration while also using
the new remote store. The stores diverge, and the new worker refuses modified
legacy backups. Recovery requires stopping consumers and choosing a consistent
backup; changing the configured database is not a merge operation.

## Verification and size

```sh
GOWORK=off go test ./...
GOWORK=off go vet ./...
III_BINARY=/path/to/iii \
DATABASE_BINARY=/path/to/database \
WHATSAPP_MIGRATE_BINARY=/path/to/whatsapp-migrate \
GOWORK=off go test -run '^TestDatabaseWorker' -count=1 -v .

III_BINARY=/path/to/iii \
DATABASE_BINARY=/path/to/database \
WHATSAPP_BINARY=/path/to/new/whatsapp \
HTTP_BINARY=/path/to/http \
GOWORK=off go test -run '^TestEngineSmoke$' -count=1 -v .

# Must print no modernc/sqlite implementation dependency for the main program.
GOWORK=off go list -deps . | rg 'modernc|sqlite'
```

The tests use isolated temporary engines/databases, synthetic identities and
loopback webhooks. They never pair an account or send real Meta/native messages.
Real database-worker tests cover native key/binary round trips, session isolation,
archive persistence, Cloud deduplication across restarts, session deletion,
legacy migration rollback, batched copying, AUTOINCREMENT preservation, a native
schema upgrade, and the migration command’s process lock. Existing HTTP tests cover Cloud text, templates, media,
webhook verification and signatures without calling Meta.

Measured on October 5, 2026 with Go 1.27.1 linux/amd64 and stripped release flags:
the previous embedded-SQLite release was 22,184,096 bytes. The remote-storage
release is 18,329,760 bytes (18.33 MB / 17.48 MiB), a 17.37% reduction. See
[build-size.md](build-size.md) for the complete measurement history.
The size reduction moves database execution into an existing worker; it does
not remove storage costs from the complete deployment. Reads/writes now incur
engine RPC latency and require the database worker to be available. Existing
SQLite benchmark results describe the old local backend, not RPC performance.

The template validation suite currently has unrelated pre-existing failures:
6 failures and 4 errors across 40 tests (agent profile skill prefixes, generated
snapshot provenance, existing Compose ordering/worker inventory and missing
seed/env/skill files). Running the same suite with only this task's Compose
changes removed reproduces the identical counts. The Compose shell syntax check
passes. These template issues are outside the WhatsApp storage migration.
