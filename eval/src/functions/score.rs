use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::functions::state::state_get;

pub async fn handle(iii: &Arc<III>, _payload: Value) -> Result<Value, IIIError> {
    let index_val = state_get(iii, crate::functions::ingest::SCOPE_INDEX, crate::functions::ingest::INDEX_KEY).await.unwrap_or(json!(null));
    let function_ids: Vec<String> = if index_val.is_array() {
        serde_json::from_value(index_val).unwrap_or_default()
    } else {
        Vec::new()
    };

    if function_ids.is_empty() {
        return Ok(json!({
            "overall_score": 100,
            "issues": [],
            "suggestions": ["No functions tracked yet. Ingest span data to begin evaluation."],
            "functions_evaluated": 0,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }));
    }

    let mut total_score: f64 = 0.0;
    let mut issues: Vec<Value> = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();
    let mut evaluated = 0u64;

    for fid in &function_ids {
        let existing = state_get(iii, crate::functions::ingest::SCOPE_SPANS, fid).await.unwrap_or(json!(null));
        let spans: Vec<Value> = if existing.is_array() {
            serde_json::from_value(existing).unwrap_or_default()
        } else {
            continue;
        };

        if spans.is_empty() {
            continue;
        }

        let metrics = crate::functions::metrics::compute_metrics(&spans, fid);
        evaluated += 1;

        let success_rate = metrics
            .get("success_rate")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let p99 = metrics
            .get("p99_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // score and drift measure different things: score is an absolute
        // health snapshot (is this function good in isolation?), drift is
        // relative to the function's own baseline (has it regressed?). A
        // function can be healthy but drifted (baseline was higher) or
        // unhealthy but stable (always bad). These thresholds pick the
        // absolute bar deliberately — treat them as ops SLOs, not
        // regression signals.
        let mut fn_score: f64 = 100.0;

        if success_rate < 0.95 {
            let penalty = (0.95 - success_rate) * 200.0;
            fn_score -= penalty;
            issues.push(json!({
                "function_id": fid,
                "issue": "low_success_rate",
                "value": success_rate,
            }));
        }

        if p99 > 5000 {
            let penalty = ((p99 as f64 - 5000.0) / 1000.0).min(30.0);
            fn_score -= penalty;
            issues.push(json!({
                "function_id": fid,
                "issue": "high_p99_latency",
                "value_ms": p99,
            }));
        }

        if success_rate < 0.80 {
            suggestions.push(format!("{}: success rate {:.1}% is critically low", fid, success_rate * 100.0));
        }

        if p99 > 10000 {
            suggestions.push(format!("{}: P99 latency {}ms exceeds 10s threshold", fid, p99));
        }

        total_score += fn_score.max(0.0);
    }

    let overall = if evaluated > 0 {
        (total_score / evaluated as f64).round() as u64
    } else {
        100
    };

    Ok(json!({
        "overall_score": overall.min(100),
        "issues": issues,
        "suggestions": suggestions,
        "functions_evaluated": evaluated,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}
