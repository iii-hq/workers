use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::analysis::scan_directory;
use crate::config::SensorConfig;
use crate::state;

pub async fn handle(iii: Arc<III>, config: Arc<SensorConfig>, payload: Value) -> Result<Value, IIIError> {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: path".to_string()))?
        .to_string();

    let extensions = match payload.get("extensions").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        None => config.scan_extensions.clone(),
    };

    let scan_result = tokio::task::spawn_blocking({
        let path = path.clone();
        let max_kb = config.max_file_size_kb;
        move || scan_directory(&path, &extensions, max_kb)
    })
    .await
    .map_err(|e| IIIError::Handler(format!("scan task failed: {e}")))?;

    let result_value = serde_json::to_value(&scan_result)
        .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))?;

    let path_hash = crate::analysis::hash_path(&path);
    let _ = state::state_set(
        &iii,
        &format!("sensor:latest:{path_hash}"),
        "scan",
        result_value.clone(),
    )
    .await;

    Ok(result_value)
}
