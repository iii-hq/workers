use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::analysis::{compute_score, hash_path, scan_directory};
use crate::config::SensorConfig;
use crate::state;

pub async fn handle(
    iii: Arc<III>,
    config: Arc<SensorConfig>,
    payload: Value,
) -> Result<Value, IIIError> {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: path".to_string()))?
        .to_string();

    let min_score = payload
        .get("min_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(config.thresholds.min_score);

    let max_degradation_pct = payload
        .get("max_degradation_pct")
        .and_then(|v| v.as_f64())
        .unwrap_or(config.thresholds.degradation_pct);

    let extensions = config.scan_extensions.clone();
    let max_kb = config.max_file_size_kb;
    let scan_result = tokio::task::spawn_blocking({
        let path = path.clone();
        move || scan_directory(&path, &extensions, max_kb)
    })
    .await
    .map_err(|e| IIIError::Handler(format!("scan task failed: {e}")))?;

    let current = compute_score(&scan_result, &config);

    let mut passed = true;
    let mut reasons: Vec<String> = Vec::new();

    if current.score < min_score {
        passed = false;
        reasons.push(format!(
            "score {:.1} below minimum {:.1}",
            current.score, min_score
        ));
    }

    let path_hash = hash_path(&path);
    let scope = format!("sensor:baselines:{path_hash}");
    if let Ok(baseline_val) = state::state_get(&iii, &scope, "default").await {
        if !baseline_val.is_null() {
            if let Some(baseline_score) = baseline_val.get("score").and_then(|v| v.as_f64()) {
                if baseline_score > 0.0 {
                    let degradation = ((baseline_score - current.score) / baseline_score) * 100.0;
                    if degradation > max_degradation_pct {
                        passed = false;
                        reasons.push(format!(
                            "degradation {:.1}% exceeds maximum {:.1}%",
                            degradation, max_degradation_pct
                        ));
                    }
                }
            }
        }
    }

    let reason = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    };

    Ok(serde_json::json!({
        "passed": passed,
        "score": current.score,
        "grade": current.grade,
        "reason": reason,
        "details": {
            "dimensions": current.dimensions,
            "file_count": current.file_count,
            "min_score": min_score,
            "max_degradation_pct": max_degradation_pct,
        }
    }))
}
