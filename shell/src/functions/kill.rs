use std::future::Future;
use std::pin::Pin;

use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::jobs::{self, JobStatus};

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

    let mut h = handle.lock().await;
    if h.record.status != JobStatus::Running {
        return Ok(json!({
            "job_id": job_id,
            "killed": false,
            "status": h.record.status,
            "reason": "not running",
        }));
    }
    let Some(child) = h.child.as_mut() else {
        return Ok(json!({
            "job_id": job_id,
            "killed": false,
            "status": h.record.status,
            "reason": "missing child handle",
        }));
    };
    child
        .start_kill()
        .map_err(|e| IIIError::Handler(format!("failed to kill job {}: {}", job_id, e)))?;
    h.record.status = JobStatus::Killed;
    h.record.finished_at_ms = Some(jobs::now_ms());
    Ok(json!({
        "job_id": job_id,
        "killed": true,
        "status": h.record.status,
    }))
}
