# browser worker — end-to-end harness

Self-asserting smoke harness for the `browser` worker. Validates
all 10 native `browser::*` parse functions (extract, css, xpath, regex, find,
find-by-text, find-by-regex, find-similar, describe, to-markdown) over the
real iii bus — worker built and run as a real binary, engine as a real
process, harness as a real WebSocket client — plus adaptive/XPath compatibility,
outbound policy errors, browser rendering/screenshots, crawl validation, and
private HTTP/dynamic/stealthy session lifecycle.

Modeled on `database/tests/e2e/` in this repo, trimmed for this worker: it needs
no database driver or dialect matrix. Most parse cases are stateless; the
adaptive case exercises the worker-managed SQLite path. No Docker service or
application schema is required.

Runs locally and in CI (`.github/workflows/browser-scrapling-e2e.yml`).

## Prerequisites

- Rust toolchain (`cargo` on `$PATH`)
- Node.js 20+ (`npm` on `$PATH`)
- The iii engine on `$PATH`. Install with:
  ```sh
  curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
  ```
  The script drops the binary at `$HOME/.local/bin/iii` (override with
  `BIN_DIR=...` or `PREFIX=...`).

## IMPORTANT: port isolation

**Do not run this suite against a stack that has another browser worker
registered.** Python uses distinct `scrapling::*` ids; this native worker uses
root `browser::*` ids. A second browser worker at the same ids can still win
dispatch and silently test the wrong implementation.

For that reason this suite does **not** default to the iii engine's own
default port (49134), which a local dev stack commonly already occupies.
`config.yaml` overrides `iii-worker-manager`'s (the engine's own builtin
WebSocket-listener worker) `port` to **49234** by default. `run-tests.sh`:

1. Preflights `E2E_PORT` (49234 unless overridden) and hard-fails if
   anything is already bound there — it never kills, reconfigures, or
   reuses another running engine.
2. Threads `E2E_PORT` through the engine config, the worker's `--url`, and
   the harness's `III_URL` so all three always agree.

Override with `E2E_PORT=<port> ./run-tests.sh` if 49234 is also taken.

There is no `--port` flag on the `iii` CLI and no `iii start` subcommand —
the override lives entirely in config.yaml, on the builtin
`iii-worker-manager` worker's `config.port` key (see the comment in
`config.yaml`).

## Run

```sh
./run-tests.sh                     # full suite
./run-tests.sh --filter=xpath      # only cases whose name contains "xpath"
```

Builds the worker (`cargo build --release --bin browser`), starts
the engine, starts the browser worker, and runs the harness. Exits
0 on PASS, 1 on any FAIL.

### Startup order

1. iii engine (`config.yaml`, booted from an untracked
   `reports/config.runtime.yaml` copy — see "Config-rewrite dodge" below)
2. browser worker binary (host process, `--url ws://127.0.0.1:$E2E_PORT`)
3. Harness test suite (`npm run dev`)

Neither the worker nor the harness is engine-managed; both connect over
WebSocket like external clients. The harness writes a runtime-only worker seed
that enables loopback and points at the downloaded frozen Chrome artifact.

### Config-rewrite dodge

The iii engine is known to rewrite the config file it boots from (observed
on `database/tests/e2e/config.yaml`, a tracked file, in this repo).
`run-tests.sh` copies `config.yaml` to the untracked
`reports/config.runtime.yaml` at startup and points `iii -c` at that copy,
so a local run never dirties the tracked config. Verify with `git status`
after a run.

## Flags

| Flag | Effect |
|---|---|
| `--keep` | Leave the engine + worker running after the run (debugging) |
| `--no-build` | Skip the cargo build step |
| `--filter=X` | Run only harness cases whose name contains `X` (substring match) |
| `-h`, `--help` | Print usage |

## Env overrides

The script auto-detects paths relative to its own location, but each can be
overridden:

| Var | Default | Purpose |
|---|---|---|
| `E2E_PORT` | `49234` | Engine WebSocket port — see "port isolation" above |
| `WORKER_SRC` | `../..` (the `browser/` crate) | Where to `cargo build` |
| `III_BIN` | `$(command -v iii)` then `$HOME/.local/bin/iii` | Engine binary |
| `WORKER_BIN_TARGET` | `$WORKER_SRC/target/release/browser` | Built worker |
| `HARNESS_TIMEOUT` | `120` | Seconds to wait for the harness sentinel |

## Layout

| File | Role |
|---|---|
| `run-tests.sh` | Orchestrator |
| `config.yaml` | Engine infra only (worker-manager port override + observability) |
| `workers/harness/` | TypeScript smoke-test worker (runs as a host process) |
| `workers/harness/src/cases.ts` | All 27 cases; parse expectations come from `../../../../tests/golden/behavior/**`, while outbound cases exercise hermetic validation/state paths |
| `workers/harness/src/runner.ts` | Runs the cases, records pass/fail, writes `reports/report.json` |
| `workers/harness/src/worker.ts` | Entry point; registers with the bus, emits the `HARNESS_DONE` sentinel |
| `reports/report.json` | Per-case results (latest run) |

## Cases

The suite currently contains 27 cases: ten parse-function examples, adaptive
and XPath compatibility, limit/error cases, outbound security policy, crawl
validation, and a private HTTP-session lifecycle. The authoritative names and
assertions live in `workers/harness/src/cases.ts`; keep this summary grouped so
it does not become a second manually numbered source of truth.

| # | Case | Asserts |
|---|---|---|
| 1 | `extract` mixed selector list | css/xpath precedence, regex short-circuit, `attr`, `html`, `all` |
| 2 | `css` first + attr | `first` scalar result, `attr` pulls an attribute over text |
| 3 | `xpath` positional predicate | `//li[2]/a`, first, text |
| 4 | `regex` first capture group | `first: true` returns the group, not the whole match |
| 5 | `find` tag + attrs | combined filter narrows to the same set as attrs alone |
| 6 | `find-by-text` leading-text match | exact match on the leading text run |
| 7 | `find-by-regex` leading-text pattern | case-insensitive default |
| 8 | `find-similar` li anchor | count 2, default `{text, html}` item shape |
| 9 | `describe` h1 | `found`, `classes`, `parent_tag`, full identity object |
| 10 | `to-markdown` text mode | exact string (deterministic; no markdown formatting choices) |
| 11 | `css` adaptive direct match | adaptive query succeeds without saving fixture state |
| 12 | `css` invalid selector | error starting `Invalid CSS selector 'li:::bad':` |
| 13 | `xpath` ancestor axis | reverse-axis positional semantics |
| 14 | `find` limit 0 | items clamp to `[]`; `count` stays the true (pre-cap) total |
| 15–27 | outbound/browser/session | SSRF and safe-policy errors, dynamic/stealthy rendering, screenshot wire shape, crawl delivery, HTTP cookie state, UUID session metadata, persistent browser sessions, foreign-id rejection |

## CI

The harness runs in `.github/workflows/browser-scrapling-e2e.yml` on any PR
that touches `browser/**`. The workflow installs the engine via the
install script (always tracks `main`, no version pin), builds the worker,
and shells out to `./run-tests.sh`. CI runners are ephemeral, so the
port-isolation concern above doesn't apply there — 49234 is used anyway, for
parity with local runs.

## Troubleshooting

- **`port $E2E_PORT is already in use`**: something else — possibly a dev
  engine — is bound to it. Stop it, or re-run with `E2E_PORT=<a-free-port>`.
  This script never kills or reuses another running engine.
- **`worker binary missing`**: run without `--no-build` once.
- **`iii engine binary missing`**: install with the script above.
- **browser worker did not respond**: tail
  `reports/browser-*.log`.
- **Sentinel timeout**: tail `reports/harness-*.log` for the harness output.
