# audit-log

Append-only audit-log subscriber on `agent::after_tool_call`. Writes one
JSON object per line to a configurable path with the shape
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

## Registered functions

| Function | Description |
|---|---|
| `policy::audit_log` | Subscriber bound to `agent::after_tool_call`. Appends one JSON line per call; reply is informational (`{ ok: true }`). |

Concurrent writers serialise per-path through a process-wide mutex map so
large tool outputs don't interleave bytes.

## Runtime expectations

Same as `policy-denylist` — needs a publisher of `agent::after_tool_call`
on the bus (typically `harness-runtime`).

## Build

```bash
cargo build --release
```
