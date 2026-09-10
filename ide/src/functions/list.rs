use std::sync::Arc;

use crate::config::ShellConfig;
use crate::functions::types::{JobSummary, ListResponse};
use crate::jobs;

pub async fn handle(cfg: Arc<ShellConfig>) -> Result<ListResponse, String> {
    jobs::remove_old(cfg.job_retention_secs).await;
    let all = jobs::list_all().await;
    let summaries: Vec<JobSummary> = all.iter().map(JobSummary::from).collect();
    let count = summaries.len();
    Ok(ListResponse {
        jobs: summaries,
        count,
    })
}
