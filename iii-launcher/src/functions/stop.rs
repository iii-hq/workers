use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::adapter::RuntimeAdapter;
use crate::state::LauncherState;

pub fn build_stop_handler(
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

            let container_id = {
                let st = state.lock().await;
                let worker = st
                    .get_worker(&name)
                    .ok_or_else(|| {
                        IIIError::Handler(format!("no managed worker named '{}'", name))
                    })?;
                worker.container_id.clone()
            };

            adapter
                .stop(&container_id, 30)
                .await
                .map_err(|e| IIIError::Handler(format!("stop failed: {e}")))?;

            adapter
                .remove(&container_id)
                .await
                .map_err(|e| IIIError::Handler(format!("remove failed: {e}")))?;

            {
                let mut st = state.lock().await;
                st.remove_worker(&name);
                st.save()
                    .map_err(|e| IIIError::Handler(format!("failed to save state: {e}")))?;
            }

            Ok(serde_json::json!({
                "name": name,
                "stopped": true,
            }))
        })
    }
}
