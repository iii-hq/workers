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

    let run_state = state::state_get(&iii, "experiment:runs", &experiment_id)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to load run state: {e}")))?;

    if run_state.is_null() {
        return Err(IIIError::Handler(format!(
            "experiment '{}' has no run state",
            experiment_id
        )));
    }

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

    let kept_count = run_state
        .get("kept_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let stopped_run = json!({
        "experiment_id": experiment_id,
        "status": "stopped",
        "current_iteration": current_iteration,
        "best_score": best_score,
        "baseline_score": baseline_score,
        "kept_count": kept_count,
    });

    state::state_set(&iii, "experiment:runs", &experiment_id, stopped_run)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to update run state: {e}")))?;

    Ok(json!({
        "stopped": true,
        "experiment_id": experiment_id,
        "iterations_completed": current_iteration,
    }))
}
