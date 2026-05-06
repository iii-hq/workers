# turn-orchestrator

Durable `run::start` state machine on the iii bus. Drives each agent turn
through provisioning → assistant → tools → steering → tearing-down, with
state checkpoints that survive process crashes.

## Installation

```bash
iii worker add turn-orchestrator
```

## Run

```bash
iii-turn-orchestrator --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL` in the environment.)

## Registered functions

| Function | Description |
|---|---|
| `run::start` | Begin a new agent run; returns the session id. |
| `run::start_and_wait` | Like `run::start` but blocks until the run completes. |
| `turn::step_requested` (subscriber) | Drives one step of the durable state machine. |

## Engine functions consumed

`state::get`, `state::set`, `state::update`, `stream::set`, `publish` —
these are iii engine builtins, no extra worker required.

## Build

```bash
cargo build --release
```
