use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::analysis::{compute_score, hash_path, scan_directory, ScanResult};
use crate::config::SensorConfig;
use crate::state;

pub async fn handle(iii: Arc<III>, config: Arc<SensorConfig>, payload: Value) -> Result<Value, IIIError> {
    let scan_result: ScanResult = if let Some(sr) = payload.get("scan_result") {
        serde_json::from_value(sr.clone())
            .map_err(|e| IIIError::Handler(format!("invalid scan_result: {e}")))?
    } else {
        let path = payload
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                IIIError::Handler("missing required field: path or scan_result".to_string())
            })?
            .to_string();

        let extensions = config.scan_extensions.clone();
        let max_kb = config.max_file_size_kb;
        tokio::task::spawn_blocking(move || scan_directory(&path, &extensions, max_kb))
            .await
            .map_err(|e| IIIError::Handler(format!("scan task failed: {e}")))?
    };

    let score_result = compute_score(&scan_result, &config);

    let result_value = serde_json::to_value(&score_result)
        .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))?;

    if let Some(path) = payload.get("path").and_then(|v| v.as_str()) {
        let path_hash = hash_path(path);

        let history_scope = format!("sensor:history:{path_hash}");
        let existing = state::state_get(&iii, &history_scope, "scores").await;
        let mut scores: Vec<Value> = existing
            .ok()
            .and_then(|v| {
                if v.is_null() {
                    None
                } else {
                    serde_json::from_value(v).ok()
                }
            })
            .unwrap_or_default();

        scores.push(result_value.clone());
        let _ = state::state_set(
            &iii,
            &history_scope,
            "scores",
            serde_json::to_value(&scores).unwrap_or_default(),
        )
        .await;
    }

    Ok(result_value)
}
