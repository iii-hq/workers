# rbac-proxy E2E harness

A self-asserting end-to-end harness that boots a real `iii` engine, starts the
`rbac-proxy` binary as a host process, and drives a downstream worker **through
the proxy port** — asserting the RBAC contract end-to-end. Modeled on
[`database/tests/e2e`](../../../database/tests/e2e), minus the docker-compose
stack (the proxy has no external dependencies beyond the engine + the
`configuration` worker, which the engine enables by default).

## Run

```bash
./run-tests.sh                 # build the worker, boot engine + proxy, run the suite
./run-tests.sh --no-build      # reuse an already-built target/release/rbac-proxy
./run-tests.sh --keep          # leave engine + proxy running after the run
./run-tests.sh --filter=prefix # run only cases whose name contains "prefix"
```

Requires the `iii` engine on `PATH` (or at `$HOME/.local/bin/iii`):

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
```

## What it does

```
run-tests.sh
  ├─ cargo build --release --bin rbac-proxy
  ├─ start engine (config.yaml; configuration worker is default-on)
  ├─ write data/seed.yaml  (auth/middleware/hooks/expose → support::* fns)
  ├─ start rbac-proxy --url ws://…:49134 --config data/seed.yaml   (binds :49271)
  └─ run the Node harness (workers/harness), which:
       1. registers the support fns on the engine:
          support::auth (bearer-token gate, per-tenant prefix + context),
          support::middleware, support::on-fn-reg, api::echo, secret::echo
       2. connects a downstream worker THROUGH the proxy (Bearer test-token)
       3. runs the cases, streams per-case results, emits HARNESS_DONE: PASS n/m
```

## Cases

| Case | Asserts |
|---|---|
| `exposed-call-through-middleware` | an exposed call succeeds **and** flows through middleware (wraps + invokes the target) |
| `forbidden-call-rejected` | a non-exposed call rejects with `FORBIDDEN` (before middleware) |
| `discovery-functions-filtered` | `engine::functions::list` shows the exposed `api::echo`, hides `secret::echo` |
| `discovery-workers-internals-stripped` | `engine::workers::list` strips `ip_address` / `isolation` |
| `prefix-applied-and-hook-ran-on-registration` | a bare `myfn` is namespaced to `tenant1::myfn`; the `on_function_registration` hook stamps metadata (verified against the engine's unfiltered view) |
| `prefix-self-invoke-roundtrip` | the session invokes its own bare `myfn`; the proxy resolves + dispatches back (prefix stripped) and the handler runs |
| `trigger-to-forbidden-denied` | a trigger bound to a forbidden function is never forwarded (no such binding on the engine) |
| `channel-roundtrip-through-proxy` | a channel round-trips through the proxy's `/ws/channels` bridge |

## Output

A per-case summary prints at the end; the machine-readable report is written to
`reports/report.json`:

```json
{ "pass": 8, "total": 8, "results": [ { "case": "...", "status": "PASS", "duration_ms": 12 } ] }
```

Engine / proxy / harness logs are written to `reports/{engine,proxy,harness}-<ts>.log`.
