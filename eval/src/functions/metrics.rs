use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::functions::state::state_get;

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn compute_metrics(spans: &[Value], function_id: &str) -> Value {
    if spans.is_empty() {
        return json!({
            "function_id": function_id,
            "p50_ms": 0,
            "p95_ms": 0,
            "p99_ms": 0,
            "success_rate": 0.0,
            "total_invocations": 0,
            "avg_duration_ms": 0.0,
            "error_count": 0,
            "throughput_per_min": 0.0,
        });
    }

    let mut durations: Vec<u64> = spans
        .iter()
        .filter_map(|s| s.get("duration_ms").and_then(|v| v.as_u64()))
        .collect();
    durations.sort_unstable();

    let total = spans.len() as u64;
    let successes = spans
        .iter()
        .filter(|s| s.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
        .count() as u64;
    let error_count = total - successes;
    let success_rate = if total > 0 {
        successes as f64 / total as f64
    } else {
        0.0
    };

    let sum: u64 = durations.iter().sum();
    let avg_duration_ms = if durations.is_empty() {
        0.0
    } else {
        sum as f64 / durations.len() as f64
    };

    let throughput_per_min = if spans.len() >= 2 {
        let timestamps: Vec<i64> = spans
            .iter()
            .filter_map(|s| {
                s.get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                    .map(|dt| dt.timestamp_millis())
            })
            .collect();

        if timestamps.len() >= 2 {
            let min_ts = timestamps.iter().copied().min().unwrap_or(0);
            let max_ts = timestamps.iter().copied().max().unwrap_or(0);
            let range_ms = (max_ts - min_ts) as f64;
            if range_ms > 0.0 {
                (timestamps.len() as f64 / range_ms) * 60_000.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    } else {
        0.0
    };

    json!({
        "function_id": function_id,
        "p50_ms": percentile(&durations, 50.0),
        "p95_ms": percentile(&durations, 95.0),
        "p99_ms": percentile(&durations, 99.0),
        "success_rate": success_rate,
        "total_invocations": total,
        "avg_duration_ms": avg_duration_ms,
        "error_count": error_count,
        "throughput_per_min": throughput_per_min,
    })
}

pub async fn handle(iii: &Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let function_id = payload
        .get("function_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing function_id".to_string()))?;

    let existing = state_get(iii, crate::functions::ingest::SCOPE_SPANS, function_id).await.unwrap_or(json!(null));

    let spans: Vec<Value> = if existing.is_array() {
        serde_json::from_value(existing).unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(compute_metrics(&spans, function_id))
}
