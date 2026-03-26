use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::adapter::RuntimeAdapter;
use crate::state::LauncherState;

pub fn build_status_handler(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let adapter = adapter.clone();
        let state = state.clone();

        Box::pin(async move {
            let st = state.lock().await;
            let mut results = Vec::new();

            for (name, worker) in &st.managed_workers {
                let running = match adapter.status(&worker.container_id).await {
                    Ok(cs) => cs.running,
                    Err(_) => false,
                };

                results.push(serde_json::json!({
                    "name": name,
                    "image": worker.image,
                    "runtime": worker.runtime,
                    "running": running,
                    "started_at": worker.started_at.to_rfc3339(),
                    "status": worker.status,
                    "restart_count": worker.restart_count,
                    "last_failure": worker.last_failure,
                }));
            }

            Ok(Value::Array(results))
        })
    }
}
