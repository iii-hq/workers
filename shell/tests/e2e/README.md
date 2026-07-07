# iii-shell worker — end-to-end harness

Self-asserting smoke harness for the `shell` worker. Validates all 5 iii
functions (`shell::exec`, `shell::exec_bg`, `shell::kill`, `shell::status`,
`shell::list`), every safety guardrail (denylist, timeout, output
truncation, env scrubbing), and the background-job lifecycle in one command.

Modeled on `database/tests/e2e/`.

## Prerequisites

- Rust toolchain (`cargo` on `$PATH`)
- Node.js 20+ (`npm` on `$PATH`)
- The iii engine on `$PATH`. Install with:
  ```sh
  curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
  ```
  The script drops the binary at `$HOME/.local/bin/iii` (override with
  `BIN_DIR=...` or `PREFIX=...`).
- **For the sandbox-target cases**: hardware virtualization on the host
  (Apple Silicon on macOS or `/dev/kvm` on Linux). The harness boots a
  real iii-sandbox microVM via `sandbox::create` and drives
  `shell::exec { target: { kind: "sandbox", ... } }` against it. On
  unsupported hosts, every case in `EXEC_SANDBOX_CASES` short-circuits
  with a `SKIP` log line — the rest of the suite still runs. First
  invocation cold-pulls the `python` image; bump `HARNESS_TIMEOUT=300`
  if the default 90s budget is too tight on a fresh machine.

## Run

```sh
./run-tests.sh
```

Builds the worker (`cargo build --release --bin iii-shell` from the `shell/` crate root), starts the
engine with `config.yaml`, and runs the full case suite across function happy
paths, safety guardrails, background jobs, and edge cases — the exact count
drifts as cases are added; see the script's own summary line or
`reports/report.json`'s `total` for the current count. Exits 0 on PASS, 1 on
any FAIL.

## Flags

| Flag | Effect |
|---|---|
| `--keep` | Leave the engine running after the run for debugging |
| `--no-build` | Skip the cargo build step |

## Env overrides

| Var | Default | Purpose |
|---|---|---|
| `WORKER_SRC` | `../..` (the `shell/` crate) | Where to `cargo build` |
| `III_BIN` | `$(command -v iii)` then `$HOME/.local/bin/iii` | Engine binary |
| `WORKER_BIN_TARGET` | `$WORKER_SRC/target/release/iii-shell` | Built worker |
| `WORKER_BIN_LINK` | `$HOME/.iii/workers/shell` | Symlink the engine reads (registered worker name from `iii.worker.yaml`). Set to `$HOME/.iii/workers/iii-shell` if your engine resolves by binary name instead. |
| `HARNESS_TIMEOUT` | `90` | Seconds to wait for the test sentinel |

## Layout

| File | Role |
|---|---|
| `run-tests.sh` | Orchestrator |
| `config.yaml` | Engine config (queue, observability, shell with tightened test config) |
| `workers/harness/` | TypeScript smoke-test worker (runs as a host process) |
| `reports/report.json` | Per-case results (latest run) |

## Filesystem cases

The harness includes ~52 `shell::fs::*` cases across four files:

- **`cases-fs-host.ts` (13)** — mkdir, ls, stat, write (channel-based
  streaming), read (channel-based streaming), rm, chmod, mv, grep, sed
  against host filesystem, plus a default-target check, an 8 MiB
  streaming round-trip, an empty-stream zero-byte write, and a cap-off
  baseline.
- **`cases-fs-host-jail.ts` (4)** — relative path → `S210`, denylisted
  paths (`/etc/passwd`, `/etc/shadow`) → `S215`, unknown `target.kind`
  → `S210`.
- **`cases-fs-sandbox.ts` (13)** — sandbox-target forwarding tests.
  The harness registers fake `sandbox::fs::*` handlers on its own SDK
  instance. The 8 simple ops use script-and-respond mocks; **`write`
  drains the caller's channel** and **`read` allocates its own channel
  and pumps known bytes**, exercising the full streaming round-trip
  end-to-end without a real sandbox VM.
- **`cases-fs-protocol-break.ts` (22)** — adversarial inputs designed
  to expose protocol gaps. Coverage includes target-envelope abuse,
  path field abuse, mode parsing, grep/sed rejection paths, and
  streaming-specific cases (missing/wrong-type `content`, malformed
  channel ref).

## v0.4.0 feature coverage

Three case files exercise the v0.4.0 API additions end-to-end over the engine:

- **`cases-fs-write-inline.ts`** — `shell::fs::write` with inline string
  `content` (host) and the multi-file `files: [...]` batch form, plus the
  ambiguity/host-only rejections (both single+`files`, empty `files`, missing
  content, inline-on-sandbox → `S210`). Reuses `fsReadStream` to confirm
  written bytes on disk.
- **`cases-exec-stdin.ts`** — the per-call `stdin` field on `shell::exec` /
  `shell::exec_bg` (piped to `cat`), the closed-stdin default, and the
  stdin-on-sandbox → `S210` rejection.
- **`cases-jobs-bg-timeout.ts`** — the separate `max_bg_timeout_ms` host bg
  cap. The e2e config sets it to `6000` (> `max_timeout_ms: 5000`), so the cap
  test also proves a bg job survives past the foreground cap (the v0.4.0
  regression fix). The shipped default `max_bg_timeout_ms: 0` (unbounded) is
  covered by the Rust unit tests.

## Vulnerability reproductions

`cases-vuln-repro.ts` and `cases-vuln-repro-jailed.ts` reproduce the
security findings from the `feat/shell-e2e-harness` review. Each case
**passes today** (the vuln is present) and is meant to be inverted or
deleted once the underlying fix lands. Names start with `vuln_repro_`
so they're easy to grep.

| Finding | Where | Suite |
|---|---|---|
| S-H1 (`chmod -R` follows symlinks) | `cases-vuln-repro.ts` | default |
| S-H2 (no `host_roots` is unjailed) | `cases-vuln-repro.ts` | default |
| S-H3 (denylist regex bypass via shell vars) | `cases-vuln-repro.ts` | default |
| S-H4 (`shell::list` cross-call argv/stdout leak) | `cases-vuln-repro.ts` | default |
| S-C1 (symlink-parent jail escape on writes) | `cases-vuln-repro-jailed.ts` | jailed |

The default suite (`./run-tests.sh`) runs the unjailed four alongside
the rest. The jailed suite (`./run-tests-jailed.sh`) boots the engine
with `config-jailed.yaml` (`host_roots: [/private/tmp/iii-shell-jailed-root]`)
and runs only the C1 repro — the rest of the suite assumes
no jail (`host_roots` unset) and would mis-fail with a jail set.

## Coder BDD coverage

The folded `coder::*` surface has a Cucumber stack under
`shell/tests/features/coder/`.

```sh
cargo test --test bdd -- --tags @direct
```

The direct scenarios call the real Rust handlers against temporary jailed
roots. `run-tests-jailed.sh` also runs the `@live` smoke while the engine is
up, using `III_ENGINE_WS_URL=ws://127.0.0.1:49134`, to prove the shell worker
actually registered the engine-dispatched `coder::*` functions.

## Troubleshooting

- **`worker binary missing`**: run without `--no-build` once.
- **`iii engine binary missing`**: install with the script above.
- **Sentinel timeout**: tail `reports/harness-*.log` for the harness output.
- **`WORKER_BIN_LINK` unresolved by engine**: try the binary-name fallback
  `WORKER_BIN_LINK=$HOME/.iii/workers/iii-shell ./run-tests.sh`.
