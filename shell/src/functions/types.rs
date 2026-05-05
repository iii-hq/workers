//! Typed request/response shapes for the 5 `shell::*` (non-fs) functions.
//! These mirror the inline `json!()` schemas that lived in `main.rs` before
//! the migration to `RegisterFunction::new_async`. The wire format is
//! unchanged from prior versions.

//! Typed responses for the 5 `shell::*` (non-fs) functions. Request shapes
//! for `shell::exec` and `shell::exec_bg` are not typed here because their
//! handlers accept raw `Value` to preserve legacy wire contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::jobs::{JobRecord, JobStatus};

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecResponse {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecBgResponse {
    pub job_id: String,
    pub argv: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KillRequest {
    pub job_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KillResponse {
    pub job_id: String,
    pub killed: bool,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StatusRequest {
    pub job_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub job: JobRecord,
}

/// `shell::list` returns one `JobSummary` per record. argv, stdout, and
/// stderr are deliberately omitted: the global JOBS map is process-wide and
/// has no per-caller scope, so any caller could otherwise read every other
/// caller's command line and captured output (which may embed credentials).
/// Full records remain reachable via `shell::status <job_id>` — the random
/// UUID acts as an unguessable capability for that record.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JobSummary {
    pub id: String,
    pub status: JobStatus,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

impl From<&JobRecord> for JobSummary {
    fn from(r: &JobRecord) -> Self {
        Self {
            id: r.id.clone(),
            status: r.status.clone(),
            started_at_ms: r.started_at_ms,
            finished_at_ms: r.finished_at_ms,
            exit_code: r.exit_code,
            stdout_truncated: r.stdout_truncated,
            stderr_truncated: r.stderr_truncated,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub jobs: Vec<JobSummary>,
    pub count: usize,
}
