//! `github::run::*` and `github::workflow::*` — GitHub Actions wrappers
//! (`gh run …`, `gh workflow …`).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;

use super::{argv, push_bool, push_opt};

pub const RUN_LIST_ID: &str = "github::run::list";
pub const RUN_LIST_DESC: &str = "List GitHub Actions workflow runs: { repo: \"owner/name\", workflow?, branch?, status?, limit? } -> { value: [{databaseId, displayTitle, workflowName, headBranch, event, status, conclusion, url, ...}] }.";
pub const RUN_JSON: &str = "databaseId,number,displayTitle,name,workflowName,headBranch,headSha,event,status,conclusion,attempt,createdAt,startedAt,updatedAt,url";

pub const RUN_VIEW_ID: &str = "github::run::view";
pub const RUN_VIEW_DESC: &str = "View one workflow run including its jobs and their steps: { repo: \"owner/name\", run_id } -> { value }.";

pub const RUN_RERUN_ID: &str = "github::run::rerun";
pub const RUN_RERUN_DESC: &str =
    "Rerun a workflow run: { repo, run_id, failed? } -> { output }. failed reruns only the failed jobs.";

pub const RUN_CANCEL_ID: &str = "github::run::cancel";
pub const RUN_CANCEL_DESC: &str =
    "Cancel an in-progress workflow run: { repo, run_id } -> { output }.";

pub const WORKFLOW_LIST_ID: &str = "github::workflow::list";
pub const WORKFLOW_LIST_DESC: &str = "List workflows in a repo: { repo: \"owner/name\", all? } -> { value: [{id, name, path, state}] }. all includes disabled workflows.";
pub const WORKFLOW_JSON: &str = "id,name,path,state";

pub const WORKFLOW_RUN_ID: &str = "github::workflow::run";
pub const WORKFLOW_RUN_DESC: &str = "Dispatch a workflow (workflow_dispatch): { repo, workflow: <file name or id>, ref?, inputs? } -> { output }. inputs become -f key=value pairs.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunListRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Filter by workflow (file name, e.g. "ci.yml").
    pub workflow: Option<String>,
    /// Filter by branch.
    pub branch: Option<String>,
    /// Filter by status (e.g. completed, in_progress, queued, failure, success).
    pub status: Option<String>,
    /// Maximum number of results (gh default: 20).
    pub limit: Option<u32>,
}

pub fn run_list_args(r: &RunListRequest) -> Vec<String> {
    let mut a = argv(["run", "list", "-R", r.repo.as_str(), "--json", RUN_JSON]);
    push_opt(&mut a, "--workflow", r.workflow.as_deref());
    push_opt(&mut a, "--branch", r.branch.as_deref());
    push_opt(&mut a, "--status", r.status.as_deref());
    push_opt(&mut a, "--limit", r.limit);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunViewRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Workflow run id (databaseId from github::run::list).
    pub run_id: u64,
}

pub fn run_view_args(r: &RunViewRequest) -> Vec<String> {
    let mut a = argv([
        "run",
        "view",
        r.run_id.to_string().as_str(),
        "-R",
        r.repo.as_str(),
        "--json",
    ]);
    a.push(format!("{RUN_JSON},jobs"));
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunRerunRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Workflow run id.
    pub run_id: u64,
    /// Rerun only the failed jobs.
    pub failed: Option<bool>,
}

pub fn run_rerun_args(r: &RunRerunRequest) -> Vec<String> {
    let mut a = argv([
        "run",
        "rerun",
        r.run_id.to_string().as_str(),
        "-R",
        r.repo.as_str(),
    ]);
    push_bool(&mut a, "--failed", r.failed);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunCancelRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Workflow run id.
    pub run_id: u64,
}

pub fn run_cancel_args(r: &RunCancelRequest) -> Vec<String> {
    argv([
        "run",
        "cancel",
        r.run_id.to_string().as_str(),
        "-R",
        r.repo.as_str(),
    ])
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkflowListRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Include disabled workflows.
    pub all: Option<bool>,
}

pub fn workflow_list_args(r: &WorkflowListRequest) -> Vec<String> {
    let mut a = argv([
        "workflow",
        "list",
        "-R",
        r.repo.as_str(),
        "--json",
        WORKFLOW_JSON,
    ]);
    push_bool(&mut a, "--all", r.all);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkflowRunRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Workflow file name (e.g. "release.yml") or numeric id.
    pub workflow: String,
    /// Branch or tag to run on (gh default: the repo default branch).
    #[serde(rename = "ref")]
    pub r#ref: Option<String>,
    /// workflow_dispatch inputs, each passed as `-f key=value`.
    pub inputs: Option<BTreeMap<String, String>>,
}

pub fn workflow_run_args(r: &WorkflowRunRequest) -> Vec<String> {
    let mut a = argv([
        "workflow",
        "run",
        r.workflow.as_str(),
        "-R",
        r.repo.as_str(),
    ]);
    push_opt(&mut a, "--ref", r.r#ref.as_deref());
    if let Some(inputs) = &r.inputs {
        for (k, v) in inputs {
            a.push("-f".to_string());
            a.push(format!("{k}={v}"));
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_view_requests_jobs_too() {
        let r: RunViewRequest =
            serde_json::from_value(json!({ "repo": "o/r", "run_id": 42 })).unwrap();
        let a = run_view_args(&r);
        assert_eq!(a[..6], ["run", "view", "42", "-R", "o/r", "--json"]);
        assert!(a[6].ends_with(",jobs"));
    }

    #[test]
    fn workflow_run_maps_ref_and_sorted_inputs() {
        let r: WorkflowRunRequest = serde_json::from_value(json!({
            "repo": "o/r", "workflow": "release.yml", "ref": "main",
            "inputs": { "worker": "github", "bump": "minor" }
        }))
        .unwrap();
        assert_eq!(
            workflow_run_args(&r),
            vec![
                "workflow",
                "run",
                "release.yml",
                "-R",
                "o/r",
                "--ref",
                "main",
                "-f",
                "bump=minor",
                "-f",
                "worker=github",
            ]
        );
    }
}
