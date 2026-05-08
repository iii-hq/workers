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
        // Sandbox-backed jobs don't own a host child process; the in-VM
        // process is reachable only through `sandbox::exec`, which has no
        // cancel hook. Mark the record Killed so shell::status / shell::list
        // reflect the cancellation; the late response from the trigger will
        // capture stdout/stderr but won't overwrite this status (see the
        // `already_killed` guard in functions::exec_bg::spawn_sandbox_job).
        h.record.status = JobStatus::Killed;
        h.record.finished_at_ms = Some(jobs::now_ms());
        return Ok(KillResponse {
            job_id: req.job_id,
            killed: true,
            status: h.record.status.clone(),
            reason: Some(
                "sandbox::exec has no cancel hook; the in-VM process will run \
                 until its timeout_ms expires"
                    .into(),
            ),
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

#[cfg(test)]
mod sandbox_kill_tests {
    use super::*;
    use crate::functions::types::KillRequest;
    use crate::jobs::{self, JobHandle, JobRecord};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn killing_sandbox_job_returns_caveat_reason() {
        // Insert a Running sandbox-backed job (child=None) directly into JOBS.
        let id = format!("job-{}", uuid::Uuid::new_v4());
        let record = JobRecord {
            id: id.clone(),
            argv: vec!["echo".into(), "x".into()],
            started_at_ms: jobs::now_ms(),
            finished_at_ms: None,
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        };
        jobs::JOBS.map.lock().await.insert(
            id.clone(),
            Arc::new(Mutex::new(JobHandle {
                record,
                child: None,
            })),
        );

        let resp = handle(KillRequest { job_id: id.clone() }).await.unwrap();
        assert!(resp.killed);
        assert_eq!(resp.status, JobStatus::Killed);
        let reason = resp
            .reason
            .expect("sandbox kill must include caveat reason");
        assert!(reason.contains("sandbox::exec"));
        assert!(reason.contains("timeout_ms"));

        // Cleanup.
        jobs::JOBS.map.lock().await.remove(&id);
    }
}
