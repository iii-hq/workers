use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::state;

pub async fn handle(iii: Arc<III>, payload: Value) -> Result<Value, IIIError> {
    let experiment_id = payload
        .get("experiment_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: experiment_id".to_string()))?
        .to_string();

    let definition = state::state_get(&iii, "experiment:definitions", &experiment_id)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to load experiment definition: {e}")))?;

    if definition.is_null() {
        return Err(IIIError::Handler(format!(
            "experiment '{}' not found",
            experiment_id
        )));
    }

    let run_state = state::state_get(&iii, "experiment:runs", &experiment_id)
        .await
        .unwrap_or(json!({}));

    let status = run_state
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let current_iteration = run_state
        .get("current_iteration")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let best_score = run_state
        .get("best_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let baseline_score = run_state
        .get("baseline_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let budget = definition
        .get("budget")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let improvement_pct = if baseline_score.abs() > f64::EPSILON {
        ((best_score - baseline_score) / baseline_score.abs()) * 100.0
    } else {
        0.0
    };

    let mut history = Vec::new();
    for i in 1..=current_iteration {
        let result_key = format!("{}:{}", experiment_id, i);
        let result = state::state_get(&iii, "experiment:results", &result_key)
            .await
            .unwrap_or(json!(null));

        if !result.is_null() {
            history.push(json!({
                "iteration": result.get("iteration").and_then(|v| v.as_u64()).unwrap_or(i),
                "score": result.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
                "kept": result.get("kept").and_then(|v| v.as_bool()).unwrap_or(false),
            }));
        }
    }

    Ok(json!({
        "experiment_id": experiment_id,
        "status": status,
        "iterations_completed": current_iteration,
        "budget": budget,
        "best_score": best_score,
        "baseline_score": baseline_score,
        "improvement_pct": improvement_pct,
        "history": history,
    }))
}
