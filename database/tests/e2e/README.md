# database worker — end-to-end harness

Self-asserting smoke harness for the `database` worker. Validates the
function surface (query / execute / prepareStatement / runStatement /
transaction), the **interactive-transaction** surface (begin /
transactionQuery / transactionExecute / commit / rollback),
`database::row-changed` delivery, and the side-channel-finalization repros
from the `/review` of branch
`feat/database-and-skills`
against real **SQLite**, **PostgreSQL 16**, and **MySQL 8.4** with one
command.

Runs locally and in CI (`.github/workflows/database-e2e.yml`).

## Prerequisites

- Docker (for the postgres + mysql containers). Rootless podman works too —
  see the `COMPOSE` env override below.
- Rust toolchain (`cargo` on `$PATH`)
- Node.js 20+ (`npm` on `$PATH`)
- The iii engine on `$PATH`. Install with:
  ```sh
  curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
  ```
  The script drops the binary at `$HOME/.local/bin/iii` (override with
  `BIN_DIR=...` or `PREFIX=...`).

## Run

```sh
./run-tests.sh                  # full suite + bypass repros
./run-tests.sh --bypass-only    # ONLY the 4 side-channel-finalization repros
./run-tests.sh --no-bypass      # full suite without the bypass repros (use
                                # when the worker hasn't shipped the fix yet)
```

Builds the worker (`cargo build --release --bin database`), brings up
the docker stack, starts the engine, seeds the
`database` configuration entry, starts the database worker, and runs the
selected case groups across all 3 drivers. Exits 0 on PASS, 1 on any FAIL.

### Startup order

1. Docker compose (postgres + mysql)
2. iii engine (`config.yaml` — queue + observability only)
3. `npm run seed-config` — registers `configuration::register` for id `database`
4. Database worker binary (host process, reads config via `configuration::get`)
5. Harness test suite (`npm run dev`)

Neither the database worker nor the harness is engine-managed; both connect
over WebSocket like external clients.

## Flags

| Flag | Effect |
|---|---|
| `--keep` | Leave the compose stack up after the run for debugging |
| `--no-build` | Skip the cargo build step |
| `--with-cargo-test` | Also run `cargo test --all-features` with live DB URLs (CI uses this) |
| `--filter=<sqlite_db\|pg_db\|mysql_db>` | Run only one driver |
| `--bypass-only` | Run only the side-channel-finalization bypass repros (4 cases × 3 drivers, gated per case) |
| `--no-bypass` | Run the full suite without the bypass repros |

## Env overrides

The script auto-detects paths relative to its own location, but each can be
overridden:

| Var | Default | Purpose |
|---|---|---|
| `WORKER_SRC` | `../..` (the `database/` crate) | Where to `cargo build` |
| `III_BIN` | `$(command -v iii)` then `$HOME/.local/bin/iii` | Engine binary |
| `WORKER_BIN_TARGET` | `$WORKER_SRC/target/release/database` | Built worker |
| `WORKER_BIN_LINK` | `$HOME/.iii/workers/database` | Symlink the engine reads |
| `TEST_POSTGRES_URL` | `postgres://iii:iii@127.0.0.1:55432/iii_test` | Override the PostgreSQL E2E database |
| `TEST_MYSQL_URL` | `mysql://iii:iii@127.0.0.1:53306/iii_test` | Override the MySQL E2E database |
| `COMPOSE` | `docker compose` | Compose command. Set to `podman-compose` for rootless podman; the script auto-switches its healthcheck strategy to `podman inspect` (since podman-compose 1.x doesn't implement compose v2's `--wait`). |
| `HARNESS_MODE` | `full` | `full` / `no-bypass` / `bypass-only`. Set by the flags above; you usually don't need to set this directly. |
| `HARNESS_TIMEOUT` | `180` | Seconds to wait for the test sentinel |
| `HEALTH_TIMEOUT` | `60` | Seconds to wait for db healthchecks |

## Bypass repros (`--bypass-only`)

The bypass-repro cases live in
`workers/harness/src/cases-tx-control-bypass.ts` and demonstrate the three
ways the `is_transaction_control_sql` filter in
`src/handlers/transaction_execute.rs` was bypassable before the fix in this
branch. Each case stages a row inside an interactive transaction, attempts
the bypass, and — if the worker accepts the SQL — proves the desync by
counting outside-txn rows that shouldn't be visible. PASS = defense holds;
FAIL = bypass confirmed (the failure message names the exact mode and the
leaked-row count).

| # | Bypass | Drivers | Maps to |
|---|---|---|---|
| 1 | `/* */COMMIT` via `transactionExecute` | all 3 | finding #1 (block-comment strip) |
| 1b | `--\nCOMMIT` via `transactionExecute` | pg + mysql | finding #1 (line-comment strip) |
| 2 | `COMMIT` via `transactionQuery` | all 3 | finding #2 (missing guard on query path) |
| 3 | `START TRANSACTION` via `transactionExecute` | mysql only | finding #3 (`START TRANSACTION` keyword) |

On the post-fix worker all four PASS. On a pre-fix worker each FAILing case
prints the desync evidence (e.g. `BYPASS CONFIRMED [#1]: '/* */COMMIT' was
accepted; outside-tx COUNT=1`).

## Layout

| File | Role |
|---|---|
| `run-tests.sh` | Orchestrator |
| `docker-compose.yml` | Postgres + MySQL with healthchecks |
| `config.yaml` | Engine infra only (queue, observability) |
| `workers/harness/src/seed-configuration.ts` | Bootstrap: `configuration::register` for id `database` |
| `workers/harness/src/database-config.ts` | E2e `databases` value (sqlite + pg + mysql) |
| `workers/harness/fixtures/database.schema.json` | JSON Schema fixture (sync with Rust via export test) |
| `workers/harness/` | TypeScript smoke-test worker (runs as a host process) |
| `workers/harness/src/cases-interactive-tx.ts` | Interactive-transaction lifecycle cases |
| `workers/harness/src/cases-row-changed.ts` | Row-change trigger delivery cases |
| `workers/harness/src/cases-native-capture.ts` | `capture: native` (postgres LISTEN/NOTIFY) cases — cross-client delivery via the `pg_native_db` handle, no double-fire on own writes, table-less binding rejection |
| `workers/harness/src/cases-tx-control-bypass.ts` | Side-channel-finalization repros |
| `reports/report.json` | Per-case results (latest run) |

### Regenerating the schema fixture

When `WorkerConfig` changes in Rust, refresh the harness schema:

```sh
cd ../..   # database/ crate root
EXPORT_E2E_SCHEMA=1 cargo test -p database export_e2e_schema_fixture -- --ignored
```

## CI

The harness runs in `.github/workflows/database-e2e.yml` on any PR
that touches `database/**`. The workflow installs the engine via the install
script (always tracks `main`, no version pin), builds the worker, brings up
the same docker compose stack used locally, and shells out to
`./run-tests.sh`.

## Troubleshooting

- **Port already in use** (55432 or 53306): something else is bound to the
  test ports. Stop it, or edit `docker-compose.yml`.
- **`worker binary missing`**: run without `--no-build` once.
- **`iii engine binary missing`**: install with the script above.
- **Database worker did not respond**: tail `reports/database-*.log`. Common
  causes: configuration seed failed, or postgres/mysql not healthy.
- **Sentinel timeout**: tail `reports/harness-*.log` for the harness output.
- **Docker daemon not running**: start Docker Desktop (or `colima start`)
  and re-run. Or use rootless podman: `COMPOSE='podman-compose' ./run-tests.sh`.
- **Bypass cases FAIL**: expected on a worker that hasn't shipped the
  side-channel-finalization fix. Re-run with `--no-bypass` to confirm the
  rest of the suite is clean.
