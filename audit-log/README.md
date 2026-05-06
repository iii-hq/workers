# audit-log

Append-only audit-log subscriber. By default it listens on
`agent::after_tool_call` and writes one JSON object per line to a configurable
path with the shape
`{ ts_ms, tool_call, result }`.

## Installation

```bash
iii worker add audit-log
```

## Run

```bash
AUDIT_LOG_PATH=/var/log/harness/audit.jsonl iii-audit-log
```

Default path is `~/.harness/audit.jsonl`.

When started by the engine, the worker reads its `config:` block from
`--config <path>`. Supported fields:

```yaml
topic: agent::after_tool_call
log_path: ~/.harness/audit.jsonl
```

`AUDIT_LOG_TOPIC` and `AUDIT_LOG_PATH` override the config file for direct
runtime overrides.

## Registered functions

| Function | Description |
|---|---|
| `policy::audit_log` | Subscriber bound to the configured topic. Appends one JSON line per call; reply is informational (`{ ok: true }`). |

Concurrent writers serialise per-path through a process-wide mutex map so
large tool outputs don't interleave bytes.

## Runtime expectations

Same as `policy-denylist` — needs a publisher for the configured topic on
the bus (typically `provider-router`).

## Build

```bash
cargo build --release
```
