# iii-database worker — end-to-end harness

Self-asserting smoke harness for the `iii-database` worker. Validates the 5
core RPC functions, the `query-poll` trigger, and the `row-change` slot/
publication derivation contract against real **SQLite**, **PostgreSQL 16**,
and **MySQL 8.4** with one command.

Runs locally and in CI (`.github/workflows/database-e2e.yml`).

## Prerequisites

- Docker (for the postgres + mysql containers)
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
./run-tests.sh
```

Builds the worker (`cargo build --release --bin iii-database`), brings up
the docker stack with `wal_level=logical`, starts the engine, and runs ~90
assertions across all 3 drivers. Exits 0 on PASS, 1 on any FAIL.

## Flags

| Flag | Effect |
|---|---|
| `--keep` | Leave docker stack up after the run for debugging |
| `--no-build` | Skip the cargo build step |
| `--filter=<sqlite_db\|pg_db\|mysql_db>` | Run only one driver |

## Env overrides

The script auto-detects paths relative to its own location, but each can be
overridden:

| Var | Default | Purpose |
|---|---|---|
| `WORKER_SRC` | `../..` (the `database/` crate) | Where to `cargo build` |
| `III_BIN` | `$(command -v iii)` then `$HOME/.local/bin/iii` | Engine binary |
| `WORKER_BIN_TARGET` | `$WORKER_SRC/target/release/iii-database` | Built worker |
| `WORKER_BIN_LINK` | `$HOME/.iii/workers/iii-database` | Symlink the engine reads |
| `HARNESS_TIMEOUT` | `180` | Seconds to wait for the test sentinel |
| `HEALTH_TIMEOUT` | `60` | Seconds to wait for db healthchecks |

## Layout

| File | Role |
|---|---|
| `run-tests.sh` | Orchestrator |
| `docker-compose.yml` | Postgres (wal_level=logical) + MySQL with healthchecks |
| `config.yaml` | Engine config (queue, observability, iii-database, harness) |
| `workers/harness/` | TypeScript smoke-test worker (runs as a host process) |
| `reports/report.json` | Per-case results (latest run) |

## CI

The harness runs in `.github/workflows/database-e2e.yml` on any PR that
touches `database/**`. The workflow installs the engine via the install
script (always tracks `main`, no version pin), builds the worker, brings up
the same docker compose stack used locally, and shells out to
`./run-tests.sh`.

## Troubleshooting

- **Port already in use** (55432 or 53306): something else is bound to the
  test ports. Stop it, or edit `docker-compose.yml`.
- **`worker binary missing`**: run without `--no-build` once.
- **`iii engine binary missing`**: install with the script above.
- **Sentinel timeout**: tail `reports/harness-*.log` for the harness output.
- **Docker daemon not running**: start Docker Desktop (or `colima start`)
  and re-run.
