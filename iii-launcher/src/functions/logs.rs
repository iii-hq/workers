use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::adapter::RuntimeAdapter;
use crate::state::LauncherState;

pub fn build_logs_handler(
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

            let follow = payload
                .get("follow")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let container_id = {
                let st = state.lock().await;
                let worker = st
                    .get_worker(&name)
                    .ok_or_else(|| {
                        IIIError::Handler(format!("no managed worker named '{}'", name))
                    })?;
                worker.container_id.clone()
            };

            let logs = adapter
                .logs(&container_id, follow)
                .await
                .map_err(|e| IIIError::Handler(format!("logs failed: {e}")))?;

            Ok(serde_json::json!({
                "name": name,
                "logs": logs,
            }))
        })
    }
}
