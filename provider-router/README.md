# provider-router

The `router::*` provider router on the iii bus. Owns the routing surface
that `turn-orchestrator` calls when an assistant message is generated, plus
helpers that push steering / follow-up messages onto session inboxes and
flip the abort flag for a session.

Renamed from `harness-runtime`; the turn-execution loop lives in
`turn-orchestrator`, not here.

## Installation

```bash
iii worker add provider-router
```

`iii worker add` resolves and installs the declared dependencies
(`session-inbox`, `llm-budget`). Abort uses `state::*` directly (no
`state-flag` dependency since that worker was removed in favor of
inline `state::set` / `state::get` calls).

## Run

```bash
iii-provider-router --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `router::stream_assistant` | Provider router. Calls `provider::<name>::complete` (with optional `router::decide` indirection when `llm-router` is on the bus). |
| `router::abort` | Set the abort flag for a session via `state::set` on `session/<id>/abort_signal` (scope `agent`). |
| `router::push_steering` | Push messages onto the session's steering inbox via `inbox::push`. |
| `router::push_followup` | Push messages onto the session's follow-up inbox via `inbox::push`. |

Plus three HTTP triggers under `agent/{session_id}/...` for the same
three push/abort handlers (HTTP path stable for backwards compat).

## Configuration

| Env var | Default | Effect |
|---|---|---|
| `III_URL` | `ws://127.0.0.1:49134` | Engine WebSocket URL. |

## Dependencies

| Worker | Range | Reason |
|---|---|---|
| `session-inbox` | `^0.1.0` | `router::push_steering` and `router::push_followup` call `inbox::push`. |
| `llm-budget` | `^0.1.0` | `router::stream_assistant` calls `budget::check` and `budget::record`. |

`router::decide` (from `llm-router`) is consulted when present but is
not required.

## Build / Test

```bash
cargo build --release
cargo test
```
