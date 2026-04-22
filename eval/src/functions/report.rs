use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::EvalConfig;
use crate::functions::state::state_get;

pub async fn handle(iii: &Arc<III>, config: &EvalConfig, payload: Value) -> Result<Value, IIIError> {
    let index_val = state_get(iii, crate::functions::ingest::SCOPE_INDEX, crate::functions::ingest::INDEX_KEY).await.unwrap_or(json!(null));
    let function_ids: Vec<String> = if index_val.is_array() {
        serde_json::from_value(index_val).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut function_reports: Vec<Value> = Vec::new();

    for fid in &function_ids {
        let existing = state_get(iii, crate::functions::ingest::SCOPE_SPANS, fid).await.unwrap_or(json!(null));
        let spans: Vec<Value> = if existing.is_array() {
            serde_json::from_value(existing).unwrap_or_default()
        } else {
            Vec::new()
        };

        if spans.is_empty() {
            continue;
        }

        let metrics = crate::functions::metrics::compute_metrics(&spans, fid);

        let baseline_val = state_get(iii, crate::functions::ingest::SCOPE_BASELINES, fid).await.unwrap_or(json!(null));
        let has_baseline = !baseline_val.is_null();

        let drift_result = if has_baseline {
            let drift_payload = json!({ "function_id": fid });
            crate::functions::drift::handle(iii, config, drift_payload)
                .await
                .ok()
        } else {
            None
        };

        function_reports.push(json!({
            "function_id": fid,
            "metrics": metrics,
            "has_baseline": has_baseline,
            "drift": drift_result,
        }));
    }

    let score_result = crate::functions::score::handle(iii, payload).await?;

    Ok(json!({
        "functions": function_reports,
        "score": score_result,
        "total_functions": function_reports.len(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}
