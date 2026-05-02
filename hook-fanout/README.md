# hook-fanout

Reusable publish-collect primitive on the iii bus. Publishes an event,
collects replies from subscribers within a deadline, then merges them by a
caller-selected `MergeRule` and returns the merged value.

## Installation

```bash
iii worker add hook-fanout
```

## Run

```bash
iii-hook-fanout --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `hooks::publish_collect` | `{ topic, payload, merge_rule, timeout_ms? }` → merged `Value`. |

## Merge rules

`first_block_wins`, `field_merge`, `pipeline_last_wins` — see source for
exact semantics.

## Build

```bash
cargo build --release
```
