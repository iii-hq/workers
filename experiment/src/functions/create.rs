use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::ExperimentConfig;
use crate::state;

pub async fn handle(
    iii: Arc<III>,
    config: Arc<ExperimentConfig>,
    payload: Value,
) -> Result<Value, IIIError> {
    let target_function = payload
        .get("target_function")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: target_function".to_string()))?
        .to_string();

    let metric_function = payload
        .get("metric_function")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: metric_function".to_string()))?
        .to_string();

    let metric_path = payload
        .get("metric_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: metric_path".to_string()))?
        .to_string();

    let direction = payload
        .get("direction")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: direction".to_string()))?
        .to_string();

    if direction != "minimize" && direction != "maximize" {
        return Err(IIIError::Handler(
            "direction must be 'minimize' or 'maximize'".to_string(),
        ));
    }

    let budget = payload
        .get("budget")
        .and_then(|v| v.as_u64())
        .map(|b| b as u32)
        .unwrap_or(config.default_budget)
        .min(config.max_budget);

    let description = payload
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let target_payload = payload
        .get("target_payload")
        .cloned()
        .unwrap_or(json!({}));

    let metric_payload = payload
        .get("metric_payload")
        .cloned()
        .unwrap_or(json!({}));

    let functions = iii.list_functions().await?;
    let fn_ids: Vec<String> = functions.iter().map(|f| f.function_id.clone()).collect();

    if !fn_ids.contains(&target_function) {
        return Err(IIIError::Handler(format!(
            "target_function '{}' not found in registered functions",
            target_function
        )));
    }

    if !fn_ids.contains(&metric_function) {
        return Err(IIIError::Handler(format!(
            "metric_function '{}' not found in registered functions",
            metric_function
        )));
    }

    let experiment_id = Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().to_rfc3339();

    let baseline_result = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: metric_function.clone(),
            payload: metric_payload.clone(),
            action: None,
            timeout_ms: Some(config.timeout_per_run_ms),
        })
        .await
        .map_err(|e| {
            IIIError::Handler(format!("failed to call metric_function for baseline: {e}"))
        })?;

    let baseline_score = extract_score(&baseline_result, &metric_path)?;

    let definition = json!({
        "experiment_id": experiment_id,
        "target_function": target_function,
        "metric_function": metric_function,
        "metric_path": metric_path,
        "direction": direction,
        "budget": budget,
        "description": description,
        "target_payload": target_payload,
        "metric_payload": metric_payload,
        "baseline_score": baseline_score,
        "best_score": baseline_score,
        "best_payload": target_payload,
        "created_at": timestamp,
    });

    state::state_set(&iii, "experiment:definitions", &experiment_id, definition)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to save experiment definition: {e}")))?;

    let run_state = json!({
        "experiment_id": experiment_id,
        "status": "created",
        "current_iteration": 0,
        "best_score": baseline_score,
        "baseline_score": baseline_score,
        "kept_count": 0,
    });

    state::state_set(&iii, "experiment:runs", &experiment_id, run_state)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to save run state: {e}")))?;

    state::state_set(
        &iii,
        "experiment:best",
        &experiment_id,
        target_payload.clone(),
    )
    .await
    .map_err(|e| IIIError::Handler(format!("failed to save best payload: {e}")))?;

    Ok(json!({
        "experiment_id": experiment_id,
        "baseline_score": baseline_score,
        "status": "created",
    }))
}

pub fn extract_score(result: &Value, metric_path: &str) -> Result<f64, IIIError> {
    let parts: Vec<&str> = metric_path.split('.').collect();
    let mut current = result;

    for part in &parts {
        current = current.get(*part).ok_or_else(|| {
            IIIError::Handler(format!(
                "metric_path '{}' not found in metric result: {}",
                metric_path,
                serde_json::to_string(result).unwrap_or_default()
            ))
        })?;
    }

    current.as_f64().ok_or_else(|| {
        IIIError::Handler(format!(
            "value at metric_path '{}' is not a number: {}",
            metric_path, current
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_score_simple() {
        let result = json!({"p99_ms": 42.0});
        assert!((extract_score(&result, "p99_ms").unwrap() - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_score_integer() {
        let result = json!({"count": 100});
        assert!((extract_score(&result, "count").unwrap() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_score_nested() {
        let result = json!({"results": {"score": 0.95}});
        assert!((extract_score(&result, "results.score").unwrap() - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_score_deep_nested() {
        let result = json!({"a": {"b": {"c": 3.14}}});
        assert!((extract_score(&result, "a.b.c").unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extract_score_missing_path() {
        let result = json!({"p99_ms": 42});
        assert!(extract_score(&result, "nonexistent").is_err());
    }

    #[test]
    fn test_extract_score_not_a_number() {
        let result = json!({"status": "ok"});
        assert!(extract_score(&result, "status").is_err());
    }
}
