use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::analysis::{compute_score, hash_path, scan_directory};
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

    let extensions = config.scan_extensions.clone();
    let max_kb = config.max_file_size_kb;
    let scan_result = tokio::task::spawn_blocking({
        let path = path.clone();
        move || scan_directory(&path, &extensions, max_kb)
    })
    .await
    .map_err(|e| IIIError::Handler(format!("scan task failed: {e}")))?;

    let score_result = compute_score(&scan_result, &config);

    let path_hash = hash_path(&path);
    let baseline_id = format!("{path_hash}:{label}");
    let timestamp = chrono::Utc::now().to_rfc3339();

    let baseline = serde_json::json!({
        "baseline_id": baseline_id,
        "score": score_result.score,
        "dimensions": score_result.dimensions,
        "timestamp": timestamp,
        "label": label,
        "file_count": score_result.file_count,
        "grade": score_result.grade,
    });

    let scope = format!("sensor:baselines:{path_hash}");
    state::state_set(&iii, &scope, &label, baseline.clone())
        .await
        .map_err(|e| IIIError::Handler(format!("failed to save baseline: {e}")))?;

    Ok(baseline)
}
