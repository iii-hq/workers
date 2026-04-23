use std::sync::Arc;

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

use crate::config::ExperimentConfig;
use crate::functions::create::extract_score;
use crate::functions::decide;
use crate::state;

pub async fn handle(
    iii: Arc<III>,
    config: Arc<ExperimentConfig>,
    payload: Value,
) -> Result<Value, IIIError> {
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

    let target_function = definition
        .get("target_function")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            IIIError::Handler("malformed definition: missing target_function".to_string())
        })?
        .to_string();

    let metric_function = definition
        .get("metric_function")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            IIIError::Handler("malformed definition: missing metric_function".to_string())
        })?
        .to_string();

    let metric_path = definition
        .get("metric_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("malformed definition: missing metric_path".to_string()))?
        .to_string();

    let metric_payload = definition
        .get("metric_payload")
        .cloned()
        .unwrap_or(json!({}));

    let timeout = config.timeout_per_run_ms;

    let modified_payload =
        if let Some(proposal_id) = payload.get("proposal_id").and_then(|v| v.as_str()) {
            let proposal_key = format!("{}:{}", experiment_id, proposal_id);
            let proposal = state::state_get(&iii, "experiment:proposals", &proposal_key)
                .await
                .map_err(|e| IIIError::Handler(format!("failed to load proposal: {e}")))?;

            if proposal.is_null() {
                return Err(IIIError::Handler(format!(
                    "proposal '{}' not found",
                    proposal_id
                )));
            }

            proposal
                .get("modified_payload")
                .cloned()
                .unwrap_or(json!({}))
        } else {
            let propose_result = crate::functions::propose::handle(
                iii.clone(),
                json!({ "experiment_id": experiment_id }),
            )
            .await?;

            propose_result
                .get("modified_payload")
                .cloned()
                .unwrap_or(json!({}))
        };

    iii.trigger(TriggerRequest {
        function_id: target_function,
        payload: modified_payload.clone(),
        action: None,
        timeout_ms: Some(timeout),
    })
    .await
    .map_err(|e| IIIError::Handler(format!("target_function call failed: {e}")))?;

    let metric_result = iii
        .trigger(TriggerRequest {
            function_id: metric_function,
            payload: metric_payload,
            action: None,
            timeout_ms: Some(timeout),
        })
        .await
        .map_err(|e| IIIError::Handler(format!("metric_function call failed: {e}")))?;

    let score = extract_score(&metric_result, &metric_path)?;

    let run_state = state::state_get(&iii, "experiment:runs", &experiment_id)
        .await
        .unwrap_or(json!({}));

    let iteration = run_state
        .get("current_iteration")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;

    let baseline_score = run_state
        .get("baseline_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(score);

    let best_score = run_state
        .get("best_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(baseline_score);

    let decision = decide::evaluate(&definition, score, best_score);

    if decision.kept {
        state::state_set(
            &iii,
            "experiment:best",
            &experiment_id,
            modified_payload.clone(),
        )
        .await
        .map_err(|e| IIIError::Handler(format!("failed to update best payload: {e}")))?;
    }

    let new_best = if decision.kept { score } else { best_score };

    let kept_count = run_state
        .get("kept_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + if decision.kept { 1 } else { 0 };

    let updated_run = json!({
        "experiment_id": experiment_id,
        "status": "running",
        "current_iteration": iteration,
        "best_score": new_best,
        "baseline_score": baseline_score,
        "kept_count": kept_count,
    });

    state::state_set(&iii, "experiment:runs", &experiment_id, updated_run)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to update run state: {e}")))?;

    let result_key = format!("{}:{}", experiment_id, iteration);
    let iteration_result = json!({
        "iteration": iteration,
        "score": score,
        "kept": decision.kept,
        "reason": decision.reason,
        "modified_payload": modified_payload,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    state::state_set(&iii, "experiment:results", &result_key, iteration_result)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to save iteration result: {e}")))?;

    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".to_string(),
            payload: json!({
                "stream_name": "experiment:progress",
                "group_id": experiment_id,
                "item_id": iteration.to_string(),
                "data": {
                    "score": score,
                    "kept": decision.kept,
                    "iteration": iteration,
                    "best_score": new_best,
                }
            }),
            action: None,
            timeout_ms: Some(5000),
        })
        .await;

    let improvement_pct = if baseline_score.abs() > f64::EPSILON {
        ((new_best - baseline_score) / baseline_score.abs()) * 100.0
    } else {
        0.0
    };

    Ok(json!({
        "iteration": iteration,
        "score": score,
        "baseline_score": baseline_score,
        "best_score": new_best,
        "kept": decision.kept,
        "improvement_pct": improvement_pct,
    }))
}
