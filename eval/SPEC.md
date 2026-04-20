# iii-eval worker

OTel-native evaluation worker for iii-engine. Consumes function execution telemetry, computes latency percentiles and success rates, scores system health, and detects metric drift against saved baselines. Designed to sit behind any worker that emits span data via the `telemetry.spans` PubSub topic.

## Why This Exists

Every observability platform (Datadog, Grafana, Honeycomb) shows you dashboards. None of them score your function fleet's health as a single number, detect drift against a known-good baseline, or run inside the same engine your functions run on.

The gap: **a self-contained evaluation loop that lives where your functions live** — no external infra, no separate deploy, no dashboards to check. Just a worker that ingests spans, computes metrics, and tells you when something drifts.

## Architecture

```
Your Workers → OTel spans → PubSub topic "telemetry.spans" → eval::ingest
                                                                    ↓
                                                              eval:spans:{fn_id} (state)
                                                                    ↓
                                              eval::metrics / eval::score / eval::drift / eval::report
```

The worker subscribes to `telemetry.spans` via a PubSub trigger. Every span ingested is stored in state keyed by function ID. Metrics, scoring, drift detection, and reporting read from that state on demand. A cron trigger runs drift detection periodically.

## State Scopes

```
eval:spans:{function_id}      — array of span objects (capped at max_spans_per_function)
eval:baselines:{function_id}  — baseline metrics snapshot for drift comparison
eval:function_index            — list of all tracked function IDs
```

## Functions (6)

### eval::ingest

```
Input:  {
  function_id: string,       (required)
  duration_ms: integer,      (required)
  success: boolean,          (required)
  error?: string,
  input_hash?: string,
  output_hash?: string,
  timestamp?: string,        (ISO 8601, defaults to now)
  trace_id?: string,
  worker_id?: string
}
Output: {ingested: true, function_id, total_spans}
```

Appends a span to `eval:spans:{function_id}`. Trims the list to `max_spans_per_function` (oldest evicted first). Maintains the `eval:function_index` for discovery by other functions.

### eval::metrics

```
Input:  {function_id: string}
Output: {
  function_id, p50_ms, p95_ms, p99_ms,
  success_rate, total_invocations, avg_duration_ms,
  error_count, throughput_per_min
}
```

Reads spans from state, sorts durations, computes percentiles via index-based lookup. Throughput calculated from timestamp range of stored spans.

### eval::score

```
Input:  {}
Output: {
  overall_score: 0-100,
  issues: [{function_id, issue, value}],
  suggestions: [string],
  functions_evaluated: integer,
  timestamp: string
}
```

Iterates all tracked functions, computes metrics for each, and produces a weighted health score. Penalties applied for:
- Success rate below 95% (up to -200 points proportional to gap)
- P99 latency above 5000ms (up to -30 points)

Score is the average across all functions, clamped to 0-100.

### eval::drift

```
Input:  {function_id?: string}  (omit to check all functions)
Output: {
  results: [{
    function_id, drifted: boolean,
    dimension?, baseline_value?, current_value?, delta_pct?
  }],
  threshold: number,
  timestamp: string
}
```

Compares current metrics against saved baselines across 5 dimensions (p50, p95, p99, success_rate, avg_duration). A dimension drifts when `|current - baseline| / baseline > drift_threshold`. If no baseline exists, returns `reason: "no_baseline"`.

### eval::baseline

```
Input:  {function_id: string}
Output: {saved: true, function_id, baseline: {...}}
```

Snapshots current metrics for a function and stores them at `eval:baselines:{function_id}`. Used as the reference point for drift detection. Call this after a known-good deploy.

### eval::report

```
Input:  {}
Output: {
  functions: [{function_id, metrics, has_baseline, drift}],
  score: {overall_score, issues, suggestions, ...},
  total_functions: integer,
  timestamp: string
}
```

Combines metrics + drift + score into a single comprehensive report across all tracked functions.

## Triggers (2)

```
Cron (1):
  expression from config (default "0 */10 * * * *") → eval::drift
  Runs periodic drift detection across all functions.

PubSub (1):
  topic "telemetry.spans" → eval::ingest
  Subscribes to OTel span data emitted by the engine or other workers.
```

## Config (config.yaml)

```yaml
retention_hours: 24              # how long to keep spans (not yet enforced, reserved)
drift_threshold: 0.15            # 15% change triggers drift alert
cron_drift_check: "0 */10 * * * *"  # every 10 minutes
max_spans_per_function: 1000     # ring buffer size per function
baseline_window_minutes: 60      # reserved for windowed baseline
```

## Integration with Other Workers

- **Any worker with OTel**: Publish spans to `telemetry.spans` topic. The eval worker picks them up automatically.
- **llm-router / llm-budget**: Ingest routing decisions and budget checks as spans to track decision latency and budget enforcement accuracy.
- **sensor**: Feed sensor readings as spans to detect telemetry pipeline degradation.
- **image-resize**: Track resize latency and error rates across different image formats.

## Example Flow

```bash
# 1. Ingest some span data
curl -X POST localhost:3111/api/eval/ingest -d '{
  "function_id": "image_resize::resize",
  "duration_ms": 45,
  "success": true,
  "trace_id": "abc123"
}'

# 2. Check metrics
curl -X POST localhost:3111/api/eval/metrics -d '{
  "function_id": "image_resize::resize"
}'
# → {"p50_ms": 42, "p95_ms": 120, "p99_ms": 180, "success_rate": 0.98, ...}

# 3. Save baseline after verified good deploy
curl -X POST localhost:3111/api/eval/baseline -d '{
  "function_id": "image_resize::resize"
}'

# 4. Later, check for drift
curl -X POST localhost:3111/api/eval/drift -d '{
  "function_id": "image_resize::resize"
}'
# → {"results": [{"function_id": "image_resize::resize", "drifted": false}]}

# 5. Get full system report
curl -X POST localhost:3111/api/eval/report -d '{}'
# → {"overall_score": 94, "functions": [...], ...}
```
