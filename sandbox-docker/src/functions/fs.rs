use iii_sdk::{IIIError, TriggerRequest};
use serde_json::{json, Value};

use crate::config::SandboxWorkerConfig;
use crate::docker;

fn validate_path(path: &str, workspace_dir: &str) -> Result<(), IIIError> {
    if path.contains("..") {
        return Err(IIIError::Handler(
            "path traversal not allowed: '..' components rejected".to_string(),
        ));
    }

    if !path.starts_with(workspace_dir) && !path.starts_with("/tmp") {
        return Err(IIIError::Handler(format!(
            "path must be under {workspace_dir} or /tmp"
        )));
    }

    Ok(())
}

pub async fn handle_read(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    config: &SandboxWorkerConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: path".to_string()))?;

    validate_path(path, &config.workspace_dir)?;

    let existing = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({
                "scope": "sandbox",
                "key": id,
            }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(format!("state get failed: {e}")))?;

    if existing.is_null() {
        return Err(IIIError::Handler(format!("sandbox not found: {id}")));
    }

    let container_name = format!("iii-sbx-{id}");
    let timeout_ms = config.max_cmd_timeout * 1000;

    let content = docker::read_file(docker_client, &container_name, path, timeout_ms)
        .await
        .map_err(|e| IIIError::Handler(format!("read failed: {e}")))?;

    Ok(json!({
        "path": path,
        "content": content,
    }))
}

pub async fn handle_write(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    config: &SandboxWorkerConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: path".to_string()))?;

    let content = payload
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: content".to_string()))?;

    validate_path(path, &config.workspace_dir)?;

    let existing = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({
                "scope": "sandbox",
                "key": id,
            }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(format!("state get failed: {e}")))?;

    if existing.is_null() {
        return Err(IIIError::Handler(format!("sandbox not found: {id}")));
    }

    let container_name = format!("iii-sbx-{id}");

    docker::copy_to_container(docker_client, &container_name, path, content.as_bytes())
        .await
        .map_err(|e| IIIError::Handler(format!("write failed: {e}")))?;

    Ok(json!({
        "path": path,
        "size": content.len(),
    }))
}

pub async fn handle_list(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    config: &SandboxWorkerConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(&config.workspace_dir);

    validate_path(path, &config.workspace_dir)?;

    let existing = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({
                "scope": "sandbox",
                "key": id,
            }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(format!("state get failed: {e}")))?;

    if existing.is_null() {
        return Err(IIIError::Handler(format!("sandbox not found: {id}")));
    }

    let container_name = format!("iii-sbx-{id}");
    let timeout_ms = config.max_cmd_timeout * 1000;

    let entries = docker::list_dir(docker_client, &container_name, path, timeout_ms)
        .await
        .map_err(|e| IIIError::Handler(format!("list failed: {e}")))?;

    serde_json::to_value(&entries)
        .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))
}
