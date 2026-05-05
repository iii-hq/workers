# durable-queue

Per-session durable queues on the iii bus under `queue::*`. Items live in iii
state under `session/<id>/<name>` so they survive worker restarts.

## Installation

```bash
iii worker add durable-queue
```

## Run

```bash
iii-durable-queue --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `queue::push` | Append `{ session_id, name, item }` to the queue. |
| `queue::drain` | Remove and return all items for `{ session_id, name }`. |
| `queue::peek` | Return all items without removing them. |

## Build

```bash
cargo build --release
```
