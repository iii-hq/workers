use crate::functions::types::{KillRequest, KillResponse};
use crate::jobs::{self, JobStatus};

pub async fn handle(req: KillRequest) -> Result<KillResponse, String> {
    let handle = jobs::get(&req.job_id)
        .await
        .ok_or_else(|| format!("no such job: {}", req.job_id))?;

    let mut h = handle.lock().await;
    if h.record.status != JobStatus::Running {
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: false,
            status: h.record.status.clone(),
            reason: Some("not running".into()),
        });
    }
    let Some(child) = h.child.as_mut() else {
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: false,
            status: h.record.status.clone(),
            reason: Some("missing child handle".into()),
        });
    };
    child
        .start_kill()
        .map_err(|e| format!("failed to kill job {}: {}", req.job_id, e))?;
    h.record.status = JobStatus::Killed;
    h.record.finished_at_ms = Some(jobs::now_ms());
    Ok(KillResponse {
        job_id: req.job_id,
        killed: true,
        status: h.record.status.clone(),
        reason: None,
    })
}
