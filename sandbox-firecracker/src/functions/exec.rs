use iii_sdk::IIIError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::config::SandboxConfig;
use crate::VmRegistry;

pub fn build_run_handler(
    _url: String,
    config: Arc<SandboxConfig>,
    registry: VmRegistry,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let config = config.clone();
        let registry = registry.clone();

        Box::pin(async move {
            let sandbox_id = payload
                .get("sandbox_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IIIError::Handler("missing required field: sandbox_id".to_string())
                })?
                .to_string();

            let command = payload
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IIIError::Handler("missing required field: command".to_string())
                })?
                .to_string();

            let timeout_secs = payload
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(config.max_cmd_timeout);

            let timeout_secs = timeout_secs.min(config.max_cmd_timeout);
            let timeout = Duration::from_secs(timeout_secs);
            let max_output = config.max_output_bytes;

            let vm_arc = {
                let map = registry.read().await;
                map.get(&sandbox_id).cloned().ok_or_else(|| {
                    IIIError::Handler(format!("sandbox not found: {sandbox_id}"))
                })?
            };

            let result = tokio::task::spawn_blocking(move || {
                let mut vm = vm_arc.blocking_lock();
                crate::exec::run_command(&mut vm, &command, timeout, max_output)
            })
            .await
            .map_err(|e| IIIError::Handler(format!("spawn_blocking join error: {e}")))?
            .map_err(|e| IIIError::Handler(format!("exec failed: {e}")))?;

            Ok(serde_json::to_value(&result)
                .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))?)
        })
    }
}

pub fn build_code_handler(
    _url: String,
    config: Arc<SandboxConfig>,
    registry: VmRegistry,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let config = config.clone();
        let registry = registry.clone();

        Box::pin(async move {
            let sandbox_id = payload
                .get("sandbox_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IIIError::Handler("missing required field: sandbox_id".to_string())
                })?
                .to_string();

            let code = payload
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    IIIError::Handler("missing required field: code".to_string())
                })?
                .to_string();

            let language = payload
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let timeout_secs = payload
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(config.max_cmd_timeout);

            let timeout_secs = timeout_secs.min(config.max_cmd_timeout);
            let timeout = Duration::from_secs(timeout_secs);
            let max_output = config.max_output_bytes;
            let lang = language.unwrap_or_else(|| config.default_language.clone());

            let vm_arc = {
                let map = registry.read().await;
                map.get(&sandbox_id).cloned().ok_or_else(|| {
                    IIIError::Handler(format!("sandbox not found: {sandbox_id}"))
                })?
            };

            let result = tokio::task::spawn_blocking(move || {
                let mut vm = vm_arc.blocking_lock();
                crate::exec::run_code(&mut vm, &code, &lang, timeout, max_output)
            })
            .await
            .map_err(|e| IIIError::Handler(format!("spawn_blocking join error: {e}")))?
            .map_err(|e| IIIError::Handler(format!("exec failed: {e}")))?;

            Ok(serde_json::to_value(&result)
                .map_err(|e| IIIError::Handler(format!("serialization failed: {e}")))?)
        })
    }
}
