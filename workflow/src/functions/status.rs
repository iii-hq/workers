use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    error::WorkflowError,
    state,
    types::{NodeState, RunStatus},
};

use super::Deps;

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

/// Read a single snapshot of a workflow run's status. For a long-running
/// pipeline, prefer `workflow::start` + a `notify` callback (pushed the outcome
/// once it's terminal) over polling this in a loop — each poll costs one of your
/// turns and a poll loop can exhaust your turn budget.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusRequest {
    /// The `run_id` returned by `workflow::start`.
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusResponse {
    pub run_id: String,
    pub status: RunStatus,
    /// Per-node state keyed by node uid.
    pub nodes: BTreeMap<String, NodeState>,
    /// Errors for nodes that failed, keyed by node uid — e.g. "no provider
    /// registered for model …". Present so a caller can diagnose a `failed` run
    /// without digging into stored state. Omitted when no node carries an error.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_errors: BTreeMap<String, String>,
    /// Nodes that have a stored result: node uid → result_ref (its key in state).
    /// Fetch the value with `workflow::node-result { run_id, node_uid }`. Lets a
    /// caller recover partial work from a run that failed partway (the run-level
    /// `result` is only set on a Completed run). Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_results: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Run-level failure summary (set when `status == failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(
    deps: &Deps,
    req: StatusRequest,
) -> Result<Option<StatusResponse>, WorkflowError> {
    match state::get_run(&deps.iii, &req.run_id).await? {
        None => Ok(None),
        Some(record) => {
            let nodes: BTreeMap<String, NodeState> = record
                .nodes
                .iter()
                .map(|(node_uid, cp)| (node_uid.clone(), cp.state))
                .collect();

            let node_errors: BTreeMap<String, String> = record
                .nodes
                .iter()
                .filter_map(|(node_uid, cp)| {
                    cp.result_error
                        .as_ref()
                        .map(|e| (node_uid.clone(), e.clone()))
                })
                .collect();

            let node_results: BTreeMap<String, String> = record
                .nodes
                .iter()
                .filter_map(|(node_uid, cp)| {
                    cp.result_ref
                        .as_ref()
                        .map(|r| (node_uid.clone(), r.clone()))
                })
                .collect();

            Ok(Some(StatusResponse {
                run_id: record.run_id,
                status: record.status,
                nodes,
                node_errors,
                node_results,
                result: record.result,
                result_error: record.result_error,
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_response_serde_round_trip() {
        let mut nodes = BTreeMap::new();
        nodes.insert("plan".to_string(), NodeState::Done);
        nodes.insert("read".to_string(), NodeState::Running);

        let mut node_errors = BTreeMap::new();
        node_errors.insert("read".to_string(), "boom".to_string());

        let mut node_results = BTreeMap::new();
        node_results.insert(
            "plan".to_string(),
            "workflow_node_result/r_abc123/plan".to_string(),
        );

        let resp = StatusResponse {
            run_id: "r_abc123".to_string(),
            status: RunStatus::AwaitingNodes,
            nodes,
            node_errors,
            node_results,
            result: Some(json!({"summary": "hello"})),
            result_error: Some("node 'read': boom".to_string()),
        };

        let serialized = serde_json::to_string(&resp).expect("serialize StatusResponse");
        let decoded: StatusResponse =
            serde_json::from_str(&serialized).expect("deserialize StatusResponse");

        assert_eq!(decoded.run_id, resp.run_id);
        assert_eq!(decoded.status, resp.status);
        assert_eq!(decoded.nodes, resp.nodes);
        assert_eq!(decoded.node_errors, resp.node_errors);
        assert_eq!(decoded.node_results, resp.node_results);
        assert_eq!(decoded.result, resp.result);
        assert_eq!(decoded.result_error, resp.result_error);
    }

    #[test]
    fn status_response_omits_empty_diagnostics() {
        let resp = StatusResponse {
            run_id: "r_xyz".to_string(),
            status: RunStatus::Running,
            nodes: BTreeMap::new(),
            node_errors: BTreeMap::new(),
            node_results: BTreeMap::new(),
            result: None,
            result_error: None,
        };

        let serialized = serde_json::to_value(&resp).expect("serialize");
        assert!(
            serialized.get("result").is_none(),
            "result omitted when None"
        );
        assert!(
            serialized.get("result_error").is_none(),
            "result_error omitted when None"
        );
        assert!(
            serialized.get("node_results").is_none(),
            "node_results omitted when empty"
        );
        assert!(
            serialized.get("node_errors").is_none(),
            "node_errors omitted when empty"
        );
    }
}
