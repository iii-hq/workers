use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::functions::state::{state_get, state_set};

pub async fn handle(iii: &Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let function_id = payload
        .get("function_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing function_id".to_string()))?;

    // Surface backend read failures instead of flattening them into
    // "no span data" — that message would otherwise hide an unreachable
    // state store and make the baseline look like a data problem.
    let existing = state_get(iii, crate::functions::ingest::SCOPE_SPANS, function_id).await?;
    let spans: Vec<Value> = if existing.is_array() {
        serde_json::from_value(existing).unwrap_or_default()
    } else if existing.is_null() {
        Vec::new()
    } else {
        return Err(IIIError::Handler(format!(
            "unexpected span state shape for {}: {}",
            function_id, existing
        )));
    };

    if spans.is_empty() {
        return Err(IIIError::Handler(format!(
            "no span data for function {}",
            function_id
        )));
    }

    let metrics = crate::functions::metrics::compute_metrics(&spans, function_id);

    let baseline = json!({
        "function_id": function_id,
        "p50_ms": metrics.get("p50_ms"),
        "p95_ms": metrics.get("p95_ms"),
        "p99_ms": metrics.get("p99_ms"),
        "success_rate": metrics.get("success_rate"),
        "avg_duration_ms": metrics.get("avg_duration_ms"),
        "total_invocations": metrics.get("total_invocations"),
        "created_at": chrono::Utc::now().to_rfc3339(),
    });

    state_set(
        iii,
        crate::functions::ingest::SCOPE_BASELINES,
        function_id,
        baseline.clone(),
    )
    .await?;

    Ok(json!({
        "saved": true,
        "function_id": function_id,
        "baseline": baseline,
    }))
}
