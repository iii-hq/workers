//! `harness::comm::history` — read the inter-agent communication log for a
//! session family. Internal: registered for the console, kept off the
//! model-facing catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::comm::{collect_events, CommEvent, COMM_LOG_CAP, COMM_LOG_SCOPE};
use crate::deps::Deps;
use crate::error::HarnessError;

pub const COMM_HISTORY_ID: &str = "harness::comm::history";
pub const COMM_HISTORY_DESC: &str =
    "Internal: read the inter-agent communication log for a session family. Not called directly.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CommHistoryRequest {
    pub root_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommHistoryResponse {
    pub events: Vec<CommEvent>,
    /// True when the ring buffer dropped older events.
    pub truncated: bool,
}

pub async fn handle(
    deps: &Deps,
    req: CommHistoryRequest,
) -> Result<CommHistoryResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let record = crate::state::state_get(
        &deps.iii,
        COMM_LOG_SCOPE,
        &req.root_session_id,
        cfg.session_timeout_ms,
    )
    .await?;
    let (events, truncated) = collect_events(&record, COMM_LOG_CAP);
    Ok(CommHistoryResponse { events, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_serializes_events_and_truncated() {
        let resp = CommHistoryResponse {
            events: vec![],
            truncated: false,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v.get("events").unwrap().as_array().unwrap().len(), 0);
        assert_eq!(v.get("truncated").unwrap(), false);
    }
}
