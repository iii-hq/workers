use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::adapter::{ContainerSpec, RuntimeAdapter};
use crate::state::{LauncherState, ManagedWorker};

pub fn build_start_handler(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let adapter = adapter.clone();
        let state = state.clone();

        Box::pin(async move {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'name' field".to_string()))?
                .to_string();

            let image = payload
                .get("image")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'image' field".to_string()))?
                .to_string();

            let engine_url = payload
                .get("engine_url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'engine_url' field".to_string()))?
                .to_string();

            let auth_token = payload
                .get("auth_token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let config = payload
                .get("config")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            // Encode config as base64 for the container env var
            let config_json = serde_json::to_string(&config)
                .map_err(|e| IIIError::Handler(format!("failed to serialize config: {e}")))?;
            let config_b64 = data_encoding::BASE64.encode(config_json.as_bytes());

            let mut env = HashMap::new();
            env.insert("III_ENGINE_URL".to_string(), engine_url);
            env.insert("III_AUTH_TOKEN".to_string(), auth_token);
            env.insert("III_WORKER_CONFIG".to_string(), config_b64);

            let spec = ContainerSpec {
                name: name.clone(),
                image: image.clone(),
                env,
                memory_limit: payload
                    .get("memory_limit")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                cpu_limit: payload
                    .get("cpu_limit")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            };

            let container_id = adapter
                .start(&spec)
                .await
                .map_err(|e| IIIError::Handler(format!("start failed: {e}")))?;

            let worker = ManagedWorker {
                image: image.clone(),
                container_id: container_id.clone(),
                runtime: "docker".to_string(),
                started_at: chrono::Utc::now(),
                status: "running".to_string(),
                config,
            };

            {
                let mut st = state.lock().await;
                st.add_worker(name.clone(), worker);
                st.save()
                    .map_err(|e| IIIError::Handler(format!("failed to save state: {e}")))?;
            }

            Ok(serde_json::json!({
                "name": name,
                "container_id": container_id,
                "status": "running",
            }))
        })
    }
}
