use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::analysis::{compute_score, hash_path, scan_directory, DimensionScores};
use crate::config::SensorConfig;
use crate::state;

pub async fn handle(iii: Arc<III>, config: Arc<SensorConfig>, payload: Value) -> Result<Value, IIIError> {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: path".to_string()))?
        .to_string();

    let label = payload
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let path_hash = hash_path(&path);

    let baseline_key = if let Some(bid) = payload.get("baseline_id").and_then(|v| v.as_str()) {
        let parts: Vec<&str> = bid.splitn(2, ':').collect();
        if parts.len() == 2 {
            parts[1].to_string()
        } else {
            label.clone()
        }
    } else {
        label.clone()
    };

    let scope = format!("sensor:baselines:{path_hash}");
    let baseline_val = state::state_get(&iii, &scope, &baseline_key)
        .await
        .map_err(|e| IIIError::Handler(format!("failed to load baseline: {e}")))?;

    if baseline_val.is_null() {
        return Err(IIIError::Handler(format!(
            "no baseline found for label '{baseline_key}' at path '{path}'"
        )));
    }

    let baseline_score = baseline_val
        .get("score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let baseline_dims: DimensionScores = serde_json::from_value(
        baseline_val
            .get("dimensions")
            .cloned()
            .unwrap_or(serde_json::json!({})),
    )
    .unwrap_or(DimensionScores {
        complexity: 0.0,
        coupling: 0.0,
        cohesion: 0.0,
        size: 0.0,
        duplication: 0.0,
    });

    let extensions = config.scan_extensions.clone();
    let max_kb = config.max_file_size_kb;
    let scan_result = tokio::task::spawn_blocking({
        let path = path.clone();
        move || scan_directory(&path, &extensions, max_kb)
    })
    .await
    .map_err(|e| IIIError::Handler(format!("scan task failed: {e}")))?;

    let current = compute_score(&scan_result, &config);

    let overall_delta = current.score - baseline_score;

    let dimension_deltas = serde_json::json!({
        "complexity": current.dimensions.complexity - baseline_dims.complexity,
        "coupling": current.dimensions.coupling - baseline_dims.coupling,
        "cohesion": current.dimensions.cohesion - baseline_dims.cohesion,
        "size": current.dimensions.size - baseline_dims.size,
        "duplication": current.dimensions.duplication - baseline_dims.duplication,
    });

    let threshold = config.thresholds.degradation_pct;
    let mut degraded_dimensions = Vec::new();

    let check = |_name: &str, current_val: f64, baseline_val: f64| -> bool {
        if baseline_val > 0.0 {
            let pct_drop = ((baseline_val - current_val) / baseline_val) * 100.0;
            pct_drop > threshold
        } else {
            false
        }
    };

    if check("complexity", current.dimensions.complexity, baseline_dims.complexity) {
        degraded_dimensions.push("complexity");
    }
    if check("coupling", current.dimensions.coupling, baseline_dims.coupling) {
        degraded_dimensions.push("coupling");
    }
    if check("cohesion", current.dimensions.cohesion, baseline_dims.cohesion) {
        degraded_dimensions.push("cohesion");
    }
    if check("size", current.dimensions.size, baseline_dims.size) {
        degraded_dimensions.push("size");
    }
    if check("duplication", current.dimensions.duplication, baseline_dims.duplication) {
        degraded_dimensions.push("duplication");
    }

    let degraded = !degraded_dimensions.is_empty();

    Ok(serde_json::json!({
        "degraded": degraded,
        "overall_delta": overall_delta,
        "dimension_deltas": dimension_deltas,
        "baseline_score": baseline_score,
        "current_score": current.score,
        "degraded_dimensions": degraded_dimensions,
        "timestamp": current.timestamp,
    }))
}
