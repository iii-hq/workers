use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    dag::last_uid,
    error::WorkflowError,
    ids::{iter_uid, split_iteration},
    state,
    types::{NodeState, WorkflowRunRecord},
};

use super::Deps;

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NodeResultRequest {
    /// The `run_id` returned by `workflow::start`.
    pub run_id: String,
    /// The node id, `"{node_id}#{i}"` for a fanout item, or `"{node_id}#it{k}"` for
    /// loop pass k (`#it0` is the first pass); the bare id of a loop node or body
    /// node returns its last pass. `uid` is accepted as an alias.
    #[serde(alias = "uid")]
    pub node_uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeResultResponse {
    /// The node's stored JSON result, or null if the node has not completed (or
    /// produced no stored result). Always serialized — even when absent it is an
    /// explicit `{ "result": null }`, so callers never special-case a missing key.
    pub result: Option<Value>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(
    deps: &Deps,
    req: NodeResultRequest,
) -> Result<NodeResultResponse, WorkflowError> {
    let Some(record) = state::get_run(&deps.iii, &req.run_id).await? else {
        return Ok(NodeResultResponse { result: None });
    };
    let result = match readable_uid(&record, &req.node_uid) {
        Some(uid) => state::get_node_result(&deps.iii, &req.run_id, &uid).await?,
        None => None,
    };
    Ok(NodeResultResponse { result })
}

/// The uid whose result `node_uid` names, when that checkpoint is done: an explicit
/// `<id>#it<k>` is pass k, any other uid its last pass. A submitted value sits in
/// the slot before the node settles, so only a Done node's result is its result.
fn readable_uid(record: &WorkflowRunRecord, node_uid: &str) -> Option<String> {
    let (id, k) = split_iteration(node_uid);
    let uid = if id == node_uid {
        last_uid(record, id)
    } else {
        iter_uid(id, k)
    };
    (record.nodes.get(&uid)?.state == NodeState::Done).then_some(uid)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn node_result_response_serde_round_trip() {
        let resp = NodeResultResponse {
            result: Some(json!({"blog_post": "..."})),
        };
        let s = serde_json::to_string(&resp).expect("serialize");
        let decoded: NodeResultResponse = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(decoded.result, resp.result);
    }

    #[test]
    fn node_result_response_serializes_explicit_null() {
        let v = serde_json::to_value(NodeResultResponse { result: None }).expect("serialize");
        // Contract: an absent result is an explicit `{ "result": null }`, never a
        // missing key — so callers don't have to special-case the key being gone.
        assert!(v.get("result").is_some(), "result key is always present");
        assert!(v["result"].is_null(), "result is null when None");
    }

    #[test]
    fn request_accepts_canonical_node_uid() {
        let req: NodeResultRequest =
            serde_json::from_value(json!({"run_id": "r", "node_uid": "write"})).expect("decode");
        assert_eq!(req.node_uid, "write");
    }

    #[test]
    fn readable_uid_picks_the_pass_and_needs_it_done() {
        let record: WorkflowRunRecord = serde_json::from_value(json!({
            "run_id": "r", "step": 0, "status": "running", "def_ref": "r", "input": null,
            "created_at": 0, "updated_at": 0,
            "nodes": {
                "v": {"state": "done"}, "v#it1": {"state": "pending"},
                "plain": {"state": "done"}, "read#0": {"state": "done"},
                "running": {"state": "running"},
            },
        }))
        .expect("record");
        let read = |uid| readable_uid(&record, uid);
        assert_eq!(read("v"), None, "the last pass is not done yet");
        assert_eq!(
            read("v#it0").as_deref(),
            Some("v"),
            "#it0 is the first pass"
        );
        assert_eq!(read("v#it1"), None);
        assert_eq!(read("plain").as_deref(), Some("plain"));
        assert_eq!(read("read#0").as_deref(), Some("read#0"));
        assert_eq!(read("running"), None);
        assert_eq!(read("missing"), None);

        let mut record = record;
        record.nodes.get_mut("v#it1").expect("v#it1").state = NodeState::Done;
        assert_eq!(readable_uid(&record, "v").as_deref(), Some("v#it1"));
        assert_eq!(readable_uid(&record, "v#it1").as_deref(), Some("v#it1"));
    }

    #[test]
    fn request_accepts_uid_alias() {
        // The live foot-gun: the contract prose says "uid", an agent passes `uid`,
        // and serde used to reject it with `missing field 'node_uid'`. The alias
        // makes the natural guess work.
        let req: NodeResultRequest =
            serde_json::from_value(json!({"run_id": "r", "uid": "write"})).expect("decode");
        assert_eq!(req.node_uid, "write");
    }
}
