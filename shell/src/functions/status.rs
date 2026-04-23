use std::future::Future;
use std::pin::Pin;

use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::jobs;

pub fn build_handler(
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| Box::pin(async move { handle(payload).await })
}

async fn handle(payload: Value) -> Result<Value, IIIError> {
    let job_id = payload
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing 'job_id'".to_string()))?;

    let handle = jobs::get(job_id)
        .await
        .ok_or_else(|| IIIError::Handler(format!("no such job: {}", job_id)))?;
    let h = handle.lock().await;
    Ok(json!({
        "job": h.record,
    }))
}
