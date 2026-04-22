# iii-eval

Every observability platform shows you dashboards. None of them score your function fleet's health as a single number, detect drift against a known-good baseline, or run inside the same engine your functions run on. iii-eval does. It ingests OTel spans, computes latency percentiles, scores system health, and tells you when something drifts — all as iii functions that any other worker can call.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-eval --url ws://your-engine:49134`. It connects, registers 7 functions, and starts ingesting telemetry. No config required — defaults work out of the box. Any connected worker (or the console chat bar) can call `eval::metrics`, `eval::score`, or `eval::analyze_traces` immediately.

## Functions

| Function ID | Description |
|---|---|
| `eval::ingest` | Append a span to state, keyed by function ID |
| `eval::metrics` | Compute percentiles, success rate, and throughput for a function |
| `eval::score` | Weighted health score (0-100) across all tracked functions |
| `eval::drift` | Compare current metrics against saved baselines across 5 dimensions |
| `eval::baseline` | Snapshot current metrics as the drift reference point |
| `eval::report` | Combined metrics + drift + score report for all functions |
| `eval::analyze_traces` | Aggregate span stats + error summary across a time window, grouped by function |

## iii Primitives Used

- **State** -- span storage, baselines, function index
- **PubSub** -- subscribes to `telemetry.spans` topic for automatic ingestion
- **Cron** -- periodic drift detection
- **HTTP** -- all functions exposed as REST endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`

## Build

```bash
cargo build --release
```

## Usage

```bash
./target/release/iii-eval --url ws://127.0.0.1:49134 --config ./config.yaml
```

```text
Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```

## Configuration

```yaml
retention_hours: 24              # how long to keep spans (reserved)
drift_threshold: 0.15            # 15% change triggers drift alert
cron_drift_check: "0 */10 * * * *"  # every 10 minutes
max_spans_per_function: 1000     # ring buffer size per function
baseline_window_minutes: 60      # reserved for windowed baseline
```

## Tests

```bash
cargo test
```
