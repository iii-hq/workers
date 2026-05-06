# subagent

Spawn child agent sessions on the iii bus under `subagent::*`. Wraps
`run::start_and_wait` so a parent agent can run a sub-task in an isolated
session and consume the result inline.

Renamed from `shell-subagent`; nothing about this worker actually involves
a shell — it spawns agents.

## Installation

```bash
iii worker add subagent
```

The install resolver pulls `turn-orchestrator` (declared via
`dependencies:` in `iii.worker.yaml`) so the `run::*` surface is available
on the bus.

## Run

```bash
iii-subagent --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `subagent::start` | Spawn a child agent session and await its result. Args: `prompt`, `provider`, `model`, optional `system_prompt`, `max_turns`, `parent_session_id`, `max_subagent_depth` (default 3). |

## Configuration

| Env var | Default | Effect |
|---|---|---|
| `III_URL` | `ws://127.0.0.1:49134` | Engine WebSocket URL. |

## Dependencies

| Worker | Range | Reason |
|---|---|---|
| `turn-orchestrator` | `^0.1.0` | Provides `run::start_and_wait` consumed by every subagent invocation. |

## Build / Test

```bash
cargo build --release
cargo test
```
