# session-inbox

Per-session inbox on the iii bus under `inbox::*`. Producers push items keyed
by `(session_id, name)`; consumers drain them atomically at session boundaries
(typically between agent turns). Items live in iii state under
`session/<id>/<name>` so they survive worker restart.

This is **not** a job queue (compare `iii-queue`, the engine builtin with
async consumers, retries, and DLQ). `session-inbox` is a pull-mode list:
producers fire-and-forget, consumers drain when they decide.

## Installation

```bash
iii worker add session-inbox
```

## Run

```bash
iii-session-inbox --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Payload | Returns |
|---|---|---|
| `inbox::push` | `{ session_id, name, item }` | `{ ok: true }` |
| `inbox::drain` | `{ session_id, name }` | `{ items: [...] }` (atomic read+clear) |
| `inbox::peek` | `{ session_id, name }` | `{ items: [...] }` (no mutation) |

`drain` is atomic: implemented via `state::update` with a single `set` op,
which returns the prior value AND writes `[]` in one round-trip. Producers
that push during a drain see no items dropped.

## Configuration

| Env var | Default | Effect |
|---|---|---|
| `III_URL` | `ws://127.0.0.1:49134` | Engine WebSocket URL. |

## Dependencies

Reads from / writes to scope `agent` of `iii-state` (the engine builtin).
No worker dependencies.

## Build / Test

```bash
cargo build --release
cargo test
```
