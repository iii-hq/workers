use iii_sdk::{register_worker, IIIError, InitOptions, TriggerRequest};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::SandboxConfig;
use crate::kvm::fork::fork_from_template;
use crate::kvm::template::Template;
use crate::types::Sandbox;
use crate::VmRegistry;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn build_create_handler(
    url: String,
    template: Arc<Template>,
    config: Arc<SandboxConfig>,
    registry: VmRegistry,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let url = url.clone();
        let template = template.clone();
        let config = config.clone();
        let registry = registry.clone();

        Box::pin(async move {
            {
                let map = registry.read().await;
                if map.len() >= config.max_sandboxes {
                    return Err(IIIError::Handler(format!(
                        "max sandbox limit reached ({})",
                        config.max_sandboxes
                    )));
                }
            }

            let language = payload
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or(&config.default_language)
                .to_string();

            let timeout = payload
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(config.default_timeout);

            let id = format!("sbx-{}", uuid::Uuid::new_v4());
            let template_ref = template.clone();

            let vm = tokio::task::spawn_blocking(move || fork_from_template(&template_ref))
                .await
                .map_err(|e| IIIError::Handler(format!("spawn_blocking join error: {e}")))?
                .map_err(|e| IIIError::Handler(format!("fork failed: {e}")))?;

            let fork_time_us = vm.fork_time_us;
            let created_at = now_secs();
            let expires_at = created_at + timeout;

            let sandbox = Sandbox {
                id: id.clone(),
                language: language.clone(),
                status: "running".to_string(),
                created_at,
                expires_at,
            };

            {
                let mut map = registry.write().await;
                map.insert(id.clone(), Arc::new(tokio::sync::Mutex::new(vm)));
            }

            let iii = register_worker(&url, InitOptions::default());
            iii.trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({
                    "scope": "sandbox",
                    "key": id,
                    "value": serde_json::to_value(&sandbox).unwrap()
                }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| IIIError::Handler(format!("state::set failed: {e}")))?;

            Ok(json!({
                "id": sandbox.id,
                "language": sandbox.language,
                "status": sandbox.status,
                "created_at": sandbox.created_at,
                "expires_at": sandbox.expires_at,
                "fork_time_us": fork_time_us,
            }))
        })
    }
}

pub fn build_get_handler(
    url: String,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let url = url.clone();

        Box::pin(async move {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?
                .to_string();

            let iii = register_worker(&url, InitOptions::default());
            let result = iii
                .trigger(TriggerRequest {
                    function_id: "state::get".into(),
                    payload: json!({ "scope": "sandbox", "key": id }),
                    action: None,
                    timeout_ms: None,
                })
                .await
                .map_err(|e| IIIError::Handler(format!("state::get failed: {e}")))?;

            Ok(result)
        })
    }
}

pub fn build_list_handler(
    url: String,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let url = url.clone();

        Box::pin(async move {
            let iii = register_worker(&url, InitOptions::default());
            let result = iii
                .trigger(TriggerRequest {
                    function_id: "state::list".into(),
                    payload: json!({ "scope": "sandbox" }),
                    action: None,
                    timeout_ms: None,
                })
                .await
                .map_err(|e| IIIError::Handler(format!("state::list failed: {e}")))?;

            Ok(json!({ "sandboxes": result }))
        })
    }
}

pub fn build_kill_handler(
    url: String,
    registry: VmRegistry,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let url = url.clone();
        let registry = registry.clone();

        Box::pin(async move {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?
                .to_string();

            let removed = {
                let mut map = registry.write().await;
                map.remove(&id)
            };

            if removed.is_none() {
                return Err(IIIError::Handler(format!("sandbox not found: {id}")));
            }

            let iii = register_worker(&url, InitOptions::default());
            iii.trigger(TriggerRequest {
                function_id: "state::delete".into(),
                payload: json!({ "scope": "sandbox", "key": id }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| IIIError::Handler(format!("state::delete failed: {e}")))?;

            Ok(json!({ "killed": true, "id": id }))
        })
    }
}
