# harness

Meta-worker that composes 14 modular workers into a runnable iii chat
surface, exposes a single browser-facing HTTP bridge, and ships a Vite/React
UI that talks to the bus through it.

The harness owns no chat, agent, or provider logic itself. It registers
exactly two functions on the iii bus and assumes the rest of the surface
(`turn-orchestrator`, `provider-router`, `session-tree`, the shell tools,
etc.) is provided by other workers. Architecture details are in
`ARCHITECTURE.md`.

## Installation

```bash
iii worker add harness
```

## Run

```bash
iii-harness
```

Defaults to `ws://127.0.0.1:49134`; override with `III_URL`.

## Registered functions

| Function | Description |
|---|---|
| `harness::status` | Returns the bundle name, version, and the list of expected runtime workers. The cheapest "is the bundle alive" probe. |
| `bridge::trigger` | Forwards `{function_id, payload}` from HTTP onto `iii.trigger(...)`. Backed by an HTTP trigger at `POST /bridge/trigger` with a 4-minute timeout. |

`bridge::trigger` is intentionally not advertised as an LLM tool — it
exists only as the browser's call-anything escape hatch.

## Expected workers

The 14 workers the harness assumes are running on the bus, sourced from
`EXPECTED_WORKERS` in `src/lib.rs`:

- `turn-orchestrator`, `provider-router`
- `session-tree`, `session-inbox`
- `models-catalog`, `hook-fanout`, `policy-denylist`
- `shell-bash`, `shell-filesystem`, `subagent`
- `provider-anthropic`, `provider-openai`
- `auth-credentials`, `llm-budget`

`tests/integration.rs` keeps `EXPECTED_WORKERS` and the `dependencies:`
block of `iii.worker.yaml` in sync.

## Local stack

`scripts/demo.sh` brings up the entire bundle directly from a checkout:

```bash
./scripts/demo.sh build    # cargo build --release for harness + 14 workers
./scripts/demo.sh engine   # start `iii --use-default-config` in background
./scripts/demo.sh start    # spawn all 14 workers + harness as nohup processes
./scripts/demo.sh verify   # call harness::status and models::list
./scripts/demo.sh web      # vite dev server on :5173 in a tmux session
./scripts/demo.sh stop     # kill every PID + engine + tmux
./scripts/demo.sh all      # build + engine + start + verify
```

PIDs and per-worker logs live under `~/iii-harness-demo` by default.

## Build

```bash
cargo build --release
```
