//! `workflow::list` — the runs a session started, its sub-agents' runs included.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::WorkflowError,
    state,
    types::{RunStatus, WorkflowRunRecord},
};

use super::Deps;

pub const LIST_ID: &str = "workflow::list";
pub const LIST_DESC: &str = "List the workflow runs started from `caller_session_id` or any session in its \
     tree (sub-agents included), newest first: {runs: [{run_id, status, title?, created_at, updated_at}]}. \
     An agent's call lists its own session's runs. Inspect one with workflow::status.";

/// Not `deny_unknown_fields`: the stamp hook may add keys (`caller_fs_root`).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// The session whose runs are listed; the stamp hook overwrites it with an
    /// agent caller's own session.
    pub caller_session_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListResponse {
    /// Newest `created_at` first.
    pub runs: Vec<ListedRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ListedRun {
    pub run_id: String,
    pub status: RunStatus,
    /// The run's display title, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The runs one of `sessions` started, newest first (ties by run id). Pure.
pub fn runs_started_by(records: Vec<WorkflowRunRecord>, sessions: &[String]) -> Vec<ListedRun> {
    let mut runs: Vec<ListedRun> = records
        .into_iter()
        .filter(|r| {
            r.caller_session_id
                .as_ref()
                .is_some_and(|s| sessions.contains(s))
        })
        .map(|r| ListedRun {
            run_id: r.run_id,
            status: r.status,
            title: r.title,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();
    runs.sort_by(|a, b| (b.created_at, &b.run_id).cmp(&(a.created_at, &a.run_id)));
    runs
}

/// The session tree and the run records (listed as the sweep lists them), read together.
async fn list<F, Fut>(session_id: &str, call: &F) -> Result<ListResponse, WorkflowError>
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: std::future::Future<Output = Result<Value, iii_sdk::errors::Error>>,
{
    let scope = json!({ "scope": state::SCOPE_RUN });
    let (mut sessions, records) = tokio::join!(
        super::session_tree(session_id, call),
        call("state::list", scope)
    );
    let records = records
        .map_err(|e| WorkflowError::State(format!("state::list {}: {e}", state::SCOPE_RUN)))?;
    // The tree of a session the harness does not know is empty; the session itself still matches.
    sessions.push(session_id.to_string());
    Ok(ListResponse {
        runs: runs_started_by(state::parse_record_list(&records), &sessions),
    })
}

pub async fn handle(deps: &Deps, req: ListRequest) -> Result<ListResponse, WorkflowError> {
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    let call = |function_id: &'static str, payload: Value| {
        deps.iii.call(iii_sdk::CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
    };
    list(&req.caller_session_id, &call).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(run_id: &str, caller: Option<&str>, created_at: i64) -> Value {
        json!({
            "run_id": run_id, "step": 0, "status": "awaiting_nodes", "def_ref": run_id,
            "input": {}, "caller_session_id": caller, "created_at": created_at, "updated_at": 99
        })
    }

    fn records() -> Vec<Value> {
        let mut titled = record("r_b", Some("child"), 30);
        titled["title"] = json!("Writing tests");
        vec![
            record("r_a", Some("root"), 10),
            titled,
            record("r_c", Some("other"), 20),
            record("r_d", None, 40),
            record("r_e", Some("root"), 30),
        ]
    }

    fn ids(runs: &[ListedRun]) -> Vec<&str> {
        runs.iter().map(|r| r.run_id.as_str()).collect()
    }

    #[test]
    fn runs_started_by_filters_on_the_sessions_newest_first() {
        let parsed = state::parse_record_list(&Value::Array(records()));
        let sessions = ["root".to_string(), "child".to_string()];
        let runs = runs_started_by(parsed.clone(), &sessions);
        // Equal `created_at`: run id descending, so the order never depends on the store.
        assert_eq!(ids(&runs), ["r_e", "r_b", "r_a"]);
        assert_eq!(
            serde_json::to_value(&runs[0]).unwrap(),
            json!({"run_id": "r_e", "status": "awaiting_nodes", "created_at": 30, "updated_at": 99})
        );
        assert_eq!(runs[1].title.as_deref(), Some("Writing tests"));
        assert!(runs_started_by(parsed, &[]).is_empty());
    }

    /// `list` against a scripted harness and state: `tree` is the session-tree reply
    /// (`None`: the read fails), `state_fails` fails `state::list`.
    async fn listed(tree: Option<Value>, state_fails: bool) -> Result<Vec<String>, WorkflowError> {
        let call = |function_id: &'static str, payload: Value| {
            std::future::ready(match function_id {
                "harness::session-tree" => {
                    assert_eq!(payload, json!({"root_session_id": "root"}));
                    tree.clone().ok_or(iii_sdk::errors::Error::Timeout)
                }
                "state::list" if state_fails => Err(iii_sdk::errors::Error::Timeout),
                "state::list" => {
                    assert_eq!(payload, json!({"scope": "workflow_run"}));
                    Ok(json!({ "items": records() }))
                }
                other => panic!("unexpected call {other}"),
            })
        };
        let resp = list("root", &call).await?;
        Ok(resp.runs.into_iter().map(|r| r.run_id).collect())
    }

    #[tokio::test]
    async fn list_reads_the_tree_and_falls_back_to_the_session() {
        let tree = json!({"sessions": [{"session_id": "root"}, {"session_id": "child"}]});
        assert_eq!(
            listed(Some(tree), false).await.unwrap(),
            ["r_e", "r_b", "r_a"]
        );
        // A failed tree read, or an unknown session's empty tree: the session alone.
        assert_eq!(listed(None, false).await.unwrap(), ["r_e", "r_a"]);
        assert_eq!(
            listed(Some(Value::Null), false).await.unwrap(),
            ["r_e", "r_a"]
        );
        let err = listed(None, true).await.unwrap_err();
        assert!(
            err.to_string().contains("state::list workflow_run"),
            "{err}"
        );
    }

    #[test]
    fn request_takes_extra_keys_the_stamp_hook_adds() {
        let req: ListRequest = serde_json::from_value(
            json!({"caller_session_id": "s", "caller_fs_root": "/home/u/proj"}),
        )
        .unwrap();
        assert_eq!(req.caller_session_id, "s");
    }
}
