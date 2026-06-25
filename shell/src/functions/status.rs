use crate::exec::error::ExecError;
use crate::functions::types::{StatusRequest, StatusResponse};
use crate::jobs;

pub async fn handle(req: StatusRequest) -> Result<StatusResponse, ExecError> {
    // Return the TYPED ExecError (not its JSON string): main.rs's
    // `.map_err(Error::from)` lifts it to `Error::Remote`, so the S-code
    // (S211 for job-not-found) lands as the top-level wire `code` — an agent
    // runs one error handler across every shell:: call instead of branching on
    // a plain-string contract for status/kill alone.
    let handle = jobs::get(&req.job_id)
        .await
        .ok_or_else(|| ExecError::new("S211", format!("no such job: {}", req.job_id)))?;
    let h = handle.lock().await;
    Ok(StatusResponse {
        job: h.record.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::types::StatusRequest;

    #[tokio::test]
    async fn status_of_missing_job_returns_typed_s211() {
        let err = handle(StatusRequest {
            job_id: "job-does-not-exist".into(),
        })
        .await
        .expect_err("missing job must error");
        // Assert the TYPED ExecError carries the S-code (so `From<ExecError>`
        // can lift it to the wire `code`), not a stringified-JSON payload.
        assert_eq!(err.code, "S211");
        assert!(err.message.contains("no such job"));
    }

    /// Pin the wire contract: the handler's `Err` lifts to
    /// `Error::Remote { code: "S211", .. }`, which the engine SDK maps to
    /// the wire `code` verbatim — NOT the `invocation_failed`/Handler collapse.
    #[tokio::test]
    async fn status_missing_job_lifts_to_remote_s211() {
        let err = handle(StatusRequest {
            job_id: "job-does-not-exist".into(),
        })
        .await
        .expect_err("missing job must error");
        match iii_sdk::errors::Error::from(err) {
            iii_sdk::errors::Error::Remote { code, message, .. } => {
                assert_eq!(code, "S211");
                assert!(message.contains("no such job"));
            }
            other => panic!("expected Error::Remote, got {other:?}"),
        }
    }
}
