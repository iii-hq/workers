use std::sync::Arc;

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Value};

use crate::config::ExperimentConfig;
use crate::functions::create::extract_score;
use crate::functions::decide;
use crate::functions::propose;
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

    let budget = definition
        .get("budget")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.default_budget as u64) as u32;

    let baseline_score = definition
        .get("baseline_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let timeout = config.timeout_per_run_ms;

    let run_state = json!({
        "experiment_id": experiment_id,
        "status": "running",
        "current_iteration": 0,
        "best_score": baseline_score,
        "baseline_score": baseline_score,
        "kept_count": 0,
    });

    state::state_set(&iii, "experiment:runs", &experiment_id, run_state)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to set run state: {e}")))?;

    let mut best_score = baseline_score;
    let mut kept_count: u64 = 0;
    let mut total_runs: u32 = 0;

    for iteration in 1..=budget {
        let current_run = state::state_get(&iii, "experiment:runs", &experiment_id)
            .await
            .unwrap_or(json!({}));

        let status = current_run
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("running");

        if status == "stopped" {
            tracing::info!(experiment_id = %experiment_id, iteration, "experiment stopped by user");
            break;
        }

        let propose_result =
            propose::handle(iii.clone(), json!({ "experiment_id": experiment_id })).await?;

        let modified_payload = propose_result
            .get("modified_payload")
            .cloned()
            .unwrap_or(json!({}));

        let target_result = iii
            .trigger(TriggerRequest {
                function_id: target_function.clone(),
                payload: modified_payload.clone(),
                action: None,
                timeout_ms: Some(timeout),
            })
            .await;

        if let Err(e) = target_result {
            tracing::warn!(
                experiment_id = %experiment_id,
                iteration,
                error = %e,
                "target_function call failed, skipping iteration"
            );
            total_runs += 1;
            continue;
        }

        let metric_result = iii
            .trigger(TriggerRequest {
                function_id: metric_function.clone(),
                payload: metric_payload.clone(),
                action: None,
                timeout_ms: Some(timeout),
            })
            .await;

        let score = match metric_result {
            Ok(ref result) => match extract_score(result, &metric_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        experiment_id = %experiment_id,
                        iteration,
                        error = %e,
                        "failed to extract score, skipping iteration"
                    );
                    total_runs += 1;
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!(
                    experiment_id = %experiment_id,
                    iteration,
                    error = %e,
                    "metric_function call failed, skipping iteration"
                );
                total_runs += 1;
                continue;
            }
        };

        let decision = decide::evaluate(&definition, score, best_score);

        if decision.kept {
            best_score = score;
            kept_count += 1;

            state::state_set(
                &iii,
                "experiment:best",
                &experiment_id,
                modified_payload.clone(),
            )
            .await
            .map_err(|e| IIIError::Handler(format!("failed to update best payload: {e}")))?;
        }

        total_runs += 1;

        let result_key = format!("{}:{}", experiment_id, iteration);
        let iteration_result = json!({
            "iteration": iteration,
            "score": score,
            "kept": decision.kept,
            "reason": decision.reason,
            "modified_payload": modified_payload,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        let _ = state::state_set(&iii, "experiment:results", &result_key, iteration_result).await;

        let updated_run = json!({
            "experiment_id": experiment_id,
            "status": "running",
            "current_iteration": iteration,
            "best_score": best_score,
            "baseline_score": baseline_score,
            "kept_count": kept_count,
        });

        let _ = state::state_set(&iii, "experiment:runs", &experiment_id, updated_run).await;

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
                        "best_score": best_score,
                    }
                }),
                action: None,
                timeout_ms: Some(5000),
            })
            .await;
    }

    let final_run = json!({
        "experiment_id": experiment_id,
        "status": "completed",
        "current_iteration": total_runs,
        "best_score": best_score,
        "baseline_score": baseline_score,
        "kept_count": kept_count,
    });

    state::state_set(&iii, "experiment:runs", &experiment_id, final_run)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to finalize run state: {e}")))?;

    let total_improvement_pct = if baseline_score.abs() > f64::EPSILON {
        ((best_score - baseline_score) / baseline_score.abs()) * 100.0
    } else {
        0.0
    };

    Ok(json!({
        "experiment_id": experiment_id,
        "total_runs": total_runs,
        "kept_count": kept_count,
        "best_score": best_score,
        "baseline_score": baseline_score,
        "total_improvement_pct": total_improvement_pct,
        "status": "completed",
    }))
}
