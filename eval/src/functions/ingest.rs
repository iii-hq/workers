use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::EvalConfig;
use crate::functions::state::{state_get, state_set};

pub const SCOPE_SPANS: &str = "eval:spans";
pub const SCOPE_BASELINES: &str = "eval:baselines";
pub const SCOPE_INDEX: &str = "eval:index";
pub const INDEX_KEY: &str = "function_list";

fn require_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| IIIError::Handler(format!("missing {field}")))
}

fn require_u64(payload: &Value, field: &str) -> Result<u64, IIIError> {
    payload
        .get(field)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| IIIError::Handler(format!("missing {field}")))
}

fn require_bool(payload: &Value, field: &str) -> Result<bool, IIIError> {
    payload
        .get(field)
        .and_then(|v| v.as_bool())
        .ok_or_else(|| IIIError::Handler(format!("missing {field}")))
}

pub async fn handle(iii: &Arc<III>, config: &EvalConfig, payload: Value) -> Result<Value, IIIError> {
    let function_id = require_str(&payload, "function_id")?;
    let duration_ms = require_u64(&payload, "duration_ms")?;
    let success = require_bool(&payload, "success")?;

    let timestamp = payload
        .get("timestamp")
        .cloned()
        .unwrap_or_else(|| json!(chrono::Utc::now().to_rfc3339()));

    let span = json!({
        "function_id": function_id,
        "duration_ms": duration_ms,
        "success": success,
        "error": payload.get("error"),
        "input_hash": payload.get("input_hash"),
        "output_hash": payload.get("output_hash"),
        "timestamp": timestamp,
        "trace_id": payload.get("trace_id"),
        "worker_id": payload.get("worker_id"),
    });

    // Read errors must NOT be swallowed — treating a backend failure as an
    // empty array would cause the subsequent state_set to overwrite any
    // existing spans with just the new one. Propagate so the caller can
    // retry or surface the failure.
    // Note: the get→modify→set sequence below is not transactional. The iii
    // engine has no CAS primitive yet, so concurrent ingests for the same
    // function_id can race and drop spans. Accepted limitation; tighten by
    // serializing per-function ingest at the caller if stricter consistency
    // is required.
    let existing = state_get(iii, SCOPE_SPANS, &function_id).await?;

    let mut spans: Vec<Value> = if existing.is_array() {
        serde_json::from_value(existing).unwrap_or_default()
    } else if existing.is_null() {
        Vec::new()
    } else {
        return Err(IIIError::Handler(format!(
            "unexpected span state shape for {function_id}: {existing}"
        )));
    };

    spans.push(span);

    let max = config.max_spans_per_function;
    if spans.len() > max {
        let drain_count = spans.len() - max;
        spans.drain(0..drain_count);
    }

    state_set(iii, SCOPE_SPANS, &function_id, json!(spans)).await?;

    Ok(json!({
        "ingested": true,
        "function_id": function_id,
        "total_spans": spans.len(),
    }))
}
