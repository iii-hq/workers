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
    let triggers = iii.list_triggers(false).await?;

    let entries: Vec<Value> = triggers
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "trigger_type": t.trigger_type,
                "function_id": t.function_id,
                "config": t.config,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "triggers": entries,
        "count": entries.len()
    }))
}
