use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{error::WorkflowError, state};

use super::Deps;

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NodeResultRequest {
    /// The `run_id` returned by `workflow::start`.
    pub run_id: String,
    /// Node uid: the node id for a plain node, or `"{node_id}#{i}"` for a
    /// fanned-out item. `workflow::status` lists the uids that have a result
    /// under `node_results` (its keys ARE the uids). The canonical arg name is
    /// `node_uid`; the shorter `uid` is accepted as an alias.
    #[serde(alias = "uid")]
    pub node_uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeResultResponse {
    /// The node's stored JSON result, or null if the node has not completed (or
    /// produced no stored result).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(
    deps: &Deps,
    req: NodeResultRequest,
) -> Result<NodeResultResponse, WorkflowError> {
    let result = state::get_node_result(&deps.iii, &req.run_id, &req.node_uid).await?;
    Ok(NodeResultResponse { result })
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
    fn node_result_response_omits_none() {
        let v = serde_json::to_value(NodeResultResponse { result: None }).expect("serialize");
        assert!(v.get("result").is_none(), "result omitted when None");
    }

    #[test]
    fn request_accepts_canonical_node_uid() {
        let req: NodeResultRequest =
            serde_json::from_value(json!({"run_id": "r", "node_uid": "write"})).expect("decode");
        assert_eq!(req.node_uid, "write");
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
