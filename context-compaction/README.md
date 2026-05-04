# context-compaction

Subscriber on the iii bus that listens to `agent::events` and triggers
session compaction once context-window thresholds are reached.

## Installation

```bash
iii worker add context-compaction
```

## Run

```bash
iii-context-compaction --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `context_compaction::on_event` | Subscriber bound to the `agent::events` topic. Inspects each event and decides whether to compact. |

## Build

```bash
cargo build --release
```
