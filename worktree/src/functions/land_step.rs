//! `worktree::land-step` — internal queue consumer. The engine delivers
//! queued jobs here; the payload's top-level `repo_key` is the FIFO group
//! field. Never agent-callable (denied in iii-permissions.yaml).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::WError;
use crate::functions::Deps;
use crate::land;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// The job to advance.
    pub job_id: String,
    /// FIFO group key; consumed by the engine's queue config, echoed here.
    #[serde(default)]
    pub repo_key: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The advanced job.
    pub job_id: String,
    /// The last phase this invocation completed.
    pub phase_completed: String,
    /// True when the job reached a terminal state (landed or blocked).
    pub done: bool,
}

pub async fn handle(deps: &Deps, req: Request) -> Result<Response, WError> {
    let cfg = deps.cfg().await;
    let land_deps = deps.land_deps(&cfg);
    let result = land::run_step(&land_deps, &req.job_id).await?;
    Ok(Response {
        job_id: req.job_id,
        phase_completed: result.phase_completed.as_str().to_string(),
        done: result.done,
    })
}
