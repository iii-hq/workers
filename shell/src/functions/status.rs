use crate::functions::types::{StatusRequest, StatusResponse};
use crate::jobs;

pub async fn handle(req: StatusRequest) -> Result<StatusResponse, String> {
    let handle = jobs::get(&req.job_id)
        .await
        .ok_or_else(|| format!("no such job: {}", req.job_id))?;
    let h = handle.lock().await;
    Ok(StatusResponse {
        job: h.record.clone(),
    })
}
