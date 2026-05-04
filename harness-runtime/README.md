# harness-runtime

The `agent::*` provider router on the iii bus. Owns the routing surface
that turn-orchestrator calls when an assistant message is generated, plus
helpers that push steering / follow-up messages onto session queues and
flip the abort flag for a session.

In the bundled `harness/` worker, `harness-runtime` also fanned in the
shell + primitive registrations. In modular mode those are independent
workers — see the `dependencies:` block below.

## Installation

```bash
iii worker add harness-runtime
```

`iii worker add` resolves and installs the declared dependencies
(`durable-queue`, `state-flag`, `llm-budget`).

## Run

```bash
iii-harness-runtime --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `agent::stream_assistant` | Provider router. Calls `provider::<name>::complete` (with optional `router::decide` indirection when `llm-router` is on the bus). |
| `agent::abort` | Set the abort flag for a session via `flag::set`. |
| `agent::push_steering` | Push messages onto the session's steering queue via `queue::push`. |
| `agent::push_followup` | Push messages onto the session's follow-up queue via `queue::push`. |

Plus three HTTP triggers under `agent/{session_id}/...` for the same
three push/abort handlers.

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `durable-queue` | `^0.1.0` | `agent::push_steering` and `agent::push_followup` call `queue::push`. |
| `state-flag` | `^0.1.0` | `agent::abort` calls `flag::set`. |
| `llm-budget` | `^0.1.0` | `agent::stream_assistant` calls `budget::check` and `budget::record`. |

`router::decide` (from `llm-router`) is consulted when present but is
not required.

## Build

```bash
cargo build --release
```
