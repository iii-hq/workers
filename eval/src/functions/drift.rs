use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::EvalConfig;
use crate::functions::state::state_get;

pub async fn handle(iii: &Arc<III>, config: &EvalConfig, payload: Value) -> Result<Value, IIIError> {
    let function_ids: Vec<String> = if let Some(fid) = payload.get("function_id").and_then(|v| v.as_str()) {
        vec![fid.to_string()]
    } else {
        // Propagate read failures — "no current data" would otherwise hide
        // a broken state backend behind a benign-looking "no drift" result.
        let index_val = state_get(iii, crate::functions::ingest::SCOPE_INDEX, crate::functions::ingest::INDEX_KEY)
            .await
            .map_err(|e| IIIError::Handler(format!("failed to read function index: {e}")))?;
        if index_val.is_array() {
            serde_json::from_value(index_val).unwrap_or_default()
        } else {
            Vec::new()
        }
    };

    let threshold = config.drift_threshold;
    let mut results: Vec<Value> = Vec::new();

    for fid in &function_ids {
        let baseline_val = state_get(iii, crate::functions::ingest::SCOPE_BASELINES, fid)
            .await
            .map_err(|e| IIIError::Handler(format!("failed to read baseline for {fid}: {e}")))?;

        if baseline_val.is_null() {
            results.push(json!({
                "function_id": fid,
                "drifted": false,
                "reason": "no_baseline",
            }));
            continue;
        }

        let existing = state_get(iii, crate::functions::ingest::SCOPE_SPANS, fid)
            .await
            .map_err(|e| IIIError::Handler(format!("failed to read spans for {fid}: {e}")))?;
        let spans: Vec<Value> = if existing.is_array() {
            serde_json::from_value(existing).unwrap_or_default()
        } else {
            Vec::new()
        };

        if spans.is_empty() {
            results.push(json!({
                "function_id": fid,
                "drifted": false,
                "reason": "no_current_data",
            }));
            continue;
        }

        let current = crate::functions::metrics::compute_metrics(&spans, fid);

        let dimensions = ["p50_ms", "p95_ms", "p99_ms", "success_rate", "avg_duration_ms"];

        for dim in &dimensions {
            let baseline_v = baseline_val.get(dim).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let current_v = current.get(dim).and_then(|v| v.as_f64()).unwrap_or(0.0);

            if baseline_v == 0.0 && current_v == 0.0 {
                continue;
            }

            let delta_pct = if baseline_v == 0.0 {
                if current_v > 0.0 { 1.0 } else { 0.0 }
            } else {
                (current_v - baseline_v).abs() / baseline_v
            };

            if delta_pct > threshold {
                results.push(json!({
                    "function_id": fid,
                    "drifted": true,
                    "dimension": dim,
                    "baseline_value": baseline_v,
                    "current_value": current_v,
                    "delta_pct": delta_pct,
                }));
            }
        }

        if !results.iter().any(|r| {
            r.get("function_id").and_then(|v| v.as_str()) == Some(fid)
                && r.get("drifted").and_then(|v| v.as_bool()) == Some(true)
        }) {
            results.push(json!({
                "function_id": fid,
                "drifted": false,
            }));
        }
    }

    Ok(json!({
        "results": results,
        "threshold": threshold,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}
