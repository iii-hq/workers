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

When started by the engine, the worker reads its `config:` block from
`--config <path>`. Defaults:

```yaml
threshold_pct: 0.85
fallback_context_window: 200000
```

`CONTEXT_COMPACTION_THRESHOLD_PCT` and
`CONTEXT_COMPACTION_FALLBACK_CONTEXT_WINDOW` override the config file for
direct runtime overrides. `fallback_context_window` is used when
`models-catalog` cannot resolve a model-specific window.

## Registered functions

| Function | Description |
|---|---|
| `context_compaction::watcher`, `context_compaction::compactor` | Stream-triggered surfaces bound to `agent::events`. Inspect each event and decide whether to compact. |

## Build

```bash
cargo build --release
```
