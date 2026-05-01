# shell-subagent

Spawn child agent sessions on the iii bus under `shell::subagent::*`. Wraps
`run::start_and_wait` so a parent agent can run a sub-task in an isolated
session and consume the result inline.

## Installation

```bash
iii worker add shell-subagent
```

The install resolver pulls `turn-orchestrator` (declared via
`dependencies:` in `iii.worker.yaml`) so the `run::*` surface is available
on the bus.

## Run

```bash
iii-shell-subagent --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

`shell::subagent::start`, `shell::subagent::wait`,
`shell::subagent::cancel`.

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `turn-orchestrator` | `^0.1.0` | Provides `run::start_and_wait` consumed by every subagent invocation. |

## Build

```bash
cargo build --release
```
