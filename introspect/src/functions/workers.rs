use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        Box::pin(async move { handle(&iii).await })
    }
}

pub async fn handle(iii: &III) -> Result<Value, IIIError> {
    let workers = iii.list_workers().await?;

    let entries: Vec<Value> = workers
        .iter()
        .filter(|w| w.name.is_some() || w.function_count > 0)
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "name": w.name,
                "function_count": w.function_count,
                "functions": w.functions,
                "status": w.status,
                "runtime": w.runtime,
                "version": w.version,
                "connected_at_ms": w.connected_at_ms,
                "active_invocations": w.active_invocations,
            })
        })
        .collect();

    let anonymous_count = workers.len() - entries.len();

    Ok(serde_json::json!({
        "workers": entries,
        "count": entries.len(),
        "anonymous_connections": anonymous_count,
    }))
}
