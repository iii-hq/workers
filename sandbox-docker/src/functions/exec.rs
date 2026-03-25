use iii_sdk::{IIIError, TriggerRequest};
use serde_json::{json, Value};

use crate::config::SandboxWorkerConfig;
use crate::docker;

pub async fn handle_run(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    config: &SandboxWorkerConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let command = payload
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: command".to_string()))?;

    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.max_cmd_timeout * 1000)
        .min(config.max_cmd_timeout * 1000);

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

    let result = docker::exec_in_container(
        docker_client,
        &container_name,
        vec!["sh".to_string(), "-c".to_string(), command.to_string()],
        timeout_ms,
    )
    .await
    .map_err(|e| IIIError::Handler(format!("exec failed: {e}")))?;

    serde_json::to_value(&result)
        .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))
}

pub async fn handle_code(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    config: &SandboxWorkerConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let code = payload
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: code".to_string()))?;

    let language = payload
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("python");

    let timeout_ms = payload
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.max_cmd_timeout * 1000)
        .min(config.max_cmd_timeout * 1000);

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

    let (ext, interpreter) = match language {
        "python" | "python3" => ("py", "python3"),
        "javascript" | "js" | "node" => ("js", "node"),
        "bash" | "sh" => ("sh", "bash"),
        other => {
            return Err(IIIError::Handler(format!("unsupported language: {other}")));
        }
    };

    let container_name = format!("iii-sbx-{id}");
    let unique_id = uuid::Uuid::new_v4();
    let code_path = format!("/tmp/code_{unique_id}.{ext}");

    docker::copy_to_container(docker_client, &container_name, &code_path, code.as_bytes())
        .await
        .map_err(|e| IIIError::Handler(format!("failed to write code file: {e}")))?;

    let result = docker::exec_in_container(
        docker_client,
        &container_name,
        vec![interpreter.to_string(), code_path.clone()],
        timeout_ms,
    )
    .await
    .map_err(|e| IIIError::Handler(format!("code exec failed: {e}")))?;

    let _ = docker::exec_in_container(
        docker_client,
        &container_name,
        vec!["rm".to_string(), "-f".to_string(), code_path],
        5000,
    )
    .await;

    serde_json::to_value(&result)
        .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))
}
