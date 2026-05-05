# harness

All-in-one harness orchestrator for demos. Composes the modular workers via
`iii.worker.yaml` runtime dependencies — the iii engine pulls and starts each
declared worker as its own process. The harness binary itself only registers
`harness::status` so the demo has one bus call to verify the bundle is up.

## Demo run (local, registry-free)

Neither the harness nor its 23 deps are released to `registry/index.json` yet,
so `iii worker add` can't find them. Each worker is just an iii-sdk client, so
we can run them directly: the script builds every binary and spawns it in the
background against a single `iii --use-default-config` engine.

```bash
# one-shot: builds + starts engine + spawns all workers + runs verify
./scripts/demo.sh all

# stop everything (workers + engine)
./scripts/demo.sh stop
```

Granular subcommands: `build`, `engine`, `start`, `verify`, `logs <worker>`,
`web`, `stop`. Pass an optional `DEMO_DIR` (defaults to `~/iii-harness-demo`)
as the second arg. `DEMO_DIR/logs/*.log` per worker, `DEMO_DIR/pids/*.pid`
for teardown, `DEMO_DIR/engine.log` for the engine.

## Browser console

A small React frontend in `harness/web/` exercises every operation visually:

```bash
./scripts/demo.sh web      # installs deps if missing, starts vite in tmux
# → http://localhost:5173
```

It talks to the bus through a single endpoint, `POST /bridge/trigger`, which
the harness worker exposes (`bridge::trigger` registered with
`trigger_type: "http"`). The browser sends `{function_id, payload}` and gets
the bus response back. From there the UI drives:

| UI                | Bus call                                                     |
|-------------------|--------------------------------------------------------------|
| status pill       | `harness::status`                                            |
| auth panel        | `auth::set_token`                                            |
| send + reply      | `run::start_and_wait` (synchronous; full transcript returned) |
| sessions rail     | `state::list scope=agent prefix=session/` filtered for `turn_state` |
| selected session  | `state::get scope=agent key=session/<id>/messages`           |

Stop the web session with `./scripts/demo.sh stop` (kills workers, engine,
and the tmux). Or attach to view dev logs: `tmux attach -t harness-web`.

`verify` returns:

```json
{
  "ok": true,
  "name": "iii-harness",
  "version": "0.1.0",
  "expected_workers": ["turn-orchestrator", "provider-router", ...]
}
```

## What's included by default

- **Orchestration**: `turn-orchestrator`, `provider-router`, `context-compaction`, `session-tree`, `session-corpus`, `document-extract`, `models-catalog`
- **Auth + policy**: `auth-credentials`, `auth-rbac`, `audit-log`, `policy-denylist`, `dlp-scrubber`, `guardrails`, `llm-budget`
- **Primitives**: `session-inbox`, `hook-fanout`
- **Shells**: `shell-bash`, `shell-filesystem`, `shell-subagent`
- **Providers**: `provider-cli`, `provider-anthropic`, `provider-openai`

OAuth flows are excluded by default (each requires browser interaction). Add
extras by editing `iii.worker.yaml` — e.g. to add the Google provider:

```yaml
dependencies:
  # ...
  provider-google: "^0.1.0"
```

Keep `EXPECTED_WORKERS` in `src/lib.rs` in sync; the integration test fails on drift.

## Known caveat

`auth-rbac` exits unless `AUTH_HMAC_SECRET` is set. Expected — the binary
refuses to run without a configured secret. Export one in your shell before
`./scripts/demo.sh start` if you want it on the bus:

```bash
export AUTH_HMAC_SECRET=$(openssl rand -hex 32)
```

## Build

```bash
cargo build --release
```
