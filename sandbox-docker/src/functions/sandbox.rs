use iii_sdk::{IIIError, TriggerRequest};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::SandboxWorkerConfig;
use crate::docker;
use crate::types::{Sandbox, SandboxConfig};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub async fn handle_create(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    config: &SandboxWorkerConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let existing = iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": "sandbox" }),
            action: None,
            timeout_ms: None,
        })
        .await
        .unwrap_or(json!([]));

    let count = existing.as_array().map(|a| a.len()).unwrap_or(0);

    if count >= config.max_sandboxes {
        return Err(IIIError::Handler(format!(
            "sandbox limit reached ({}/{})",
            count, config.max_sandboxes
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let image = payload
        .get("image")
        .and_then(|v| v.as_str())
        .unwrap_or(&config.default_image)
        .to_string();
    let timeout = payload
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.default_timeout);
    let memory = payload
        .get("memory")
        .and_then(|v| v.as_u64())
        .unwrap_or(config.default_memory);
    let cpu = payload
        .get("cpu")
        .and_then(|v| v.as_f64())
        .unwrap_or(config.default_cpu);
    let network = payload
        .get("network")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let workdir = payload
        .get("workdir")
        .and_then(|v| v.as_str())
        .unwrap_or(&config.workspace_dir)
        .to_string();

    let env_map: HashMap<String, String> = payload
        .get("env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    docker::ensure_image(docker_client, &image)
        .await
        .map_err(|e| IIIError::Handler(format!("image pull failed: {e}")))?;

    docker::create_container(
        docker_client,
        &docker::ContainerOpts {
            id: &id,
            image: &image,
            memory_mb: memory,
            cpu,
            network,
            env: &env_map,
            workdir: &workdir,
        },
    )
    .await
    .map_err(|e| IIIError::Handler(format!("container creation failed: {e}")))?;

    let created_at = now_secs();
    let expires_at = created_at + timeout;

    let sandbox = Sandbox {
        id: id.clone(),
        image: image.clone(),
        status: "running".to_string(),
        created_at,
        expires_at,
        config: SandboxConfig {
            image: Some(image),
            timeout: Some(timeout),
            memory: Some(memory),
            cpu: Some(cpu),
            network: Some(network),
            env: Some(env_map),
            workdir: Some(workdir),
        },
    };

    let sandbox_value = serde_json::to_value(&sandbox)
        .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))?;

    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({
            "scope": "sandbox",
            "key": id,
            "value": sandbox_value,
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .map_err(|e| IIIError::Handler(format!("state set failed: {e}")))?;

    Ok(sandbox_value)
}

pub async fn handle_get(iii: &iii_sdk::III, payload: Value) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let result = iii
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

    if result.is_null() {
        return Err(IIIError::Handler(format!("sandbox not found: {id}")));
    }

    Ok(result)
}

pub async fn handle_list(iii: &iii_sdk::III, _payload: Value) -> Result<Value, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": "sandbox" }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| IIIError::Handler(format!("state list failed: {e}")))?;

    Ok(result)
}

pub async fn handle_kill(
    iii: &iii_sdk::III,
    docker_client: &bollard::Docker,
    payload: Value,
) -> Result<Value, IIIError> {
    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

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

    docker::stop_and_remove(docker_client, id)
        .await
        .map_err(|e| IIIError::Handler(format!("container removal failed: {e}")))?;

    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({
            "scope": "sandbox",
            "key": id,
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .map_err(|e| IIIError::Handler(format!("state delete failed: {e}")))?;

    Ok(json!({ "id": id, "status": "killed" }))
}
