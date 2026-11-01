//! `harness::on-session-tree-deletion`: every status change of a durable
//! subtree deletion, as a private function whose handler returns the
//! deletion snapshot it is called with. It takes no args: consumers observe
//! it with filters on `input.session_id` and `input.operation_id`, before
//! requesting the deletion, and recover once by operation id.
use std::sync::Arc;

use iii_sdk::{CallAction, CallRequest, Error, IIIClient, RegisterFunction};

use crate::error::HarnessError;
use crate::functions::delete_session_tree::Snapshot;
use crate::surface::schema_value;

pub const ON_SESSION_TREE_DELETION_FN: &str = "harness::on-session-tree-deletion";

pub const DESCRIPTION: &str = "A subtree deletion changed status. Observe it before requesting \
     deletion, with filters on input.session_id or input.operation_id, and recover once by \
     operation_id.";

/// Register the function. Once per worker: [`crate::events::TurnEvents::register`]
/// registers it with the other event functions.
pub fn register(iii: &IIIClient) {
    iii.register_function(
        ON_SESSION_TREE_DELETION_FN,
        RegisterFunction::new(|snapshot: Snapshot| Ok::<Snapshot, Error>(snapshot))
            .description(DESCRIPTION)
            .request_format(schema_value::<Snapshot>())
            .response_format(schema_value::<Snapshot>())
            .private(),
    );
}

/// Emits deletion snapshots by calling the function.
#[derive(Clone)]
pub struct DeletionEvents {
    iii: Arc<IIIClient>,
}

impl DeletionEvents {
    pub fn new(iii: &Arc<IIIClient>) -> Self {
        Self { iii: iii.clone() }
    }

    /// Call the function with the snapshot; iii delivers it to each observer
    /// whose filters match. Delivery does not wait for targets.
    pub async fn emit(&self, snapshot: &Snapshot) -> Result<(), HarnessError> {
        if !self.iii.is_observed(ON_SESSION_TREE_DELETION_FN) {
            return Ok(());
        }
        let request = CallRequest {
            function_id: ON_SESSION_TREE_DELETION_FN.to_string(),
            payload: serde_json::to_value(snapshot)
                .map_err(|e| HarnessError::Internal(e.to_string()))?,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        self.iii
            .call(request)
            .await
            .map(|_| ())
            .map_err(|e| HarnessError::Dependency(format!("deletion event call: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use crate::functions::delete_session_tree::{DeletionStatus, Snapshot};

    /// Consumers filter on `input.session_id` and `input.operation_id`: the
    /// event carries both, and nothing beyond the snapshot.
    #[test]
    fn the_event_is_the_snapshot_with_its_filterable_fields() {
        let snapshot = Snapshot {
            operation_id: "op".into(),
            attempt: 1,
            session_id: "child2".into(),
            status: DeletionStatus::Completed,
            deleted_session_ids: vec!["grandchild1".into(), "child2".into()],
            error: None,
        };
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({"operation_id":"op","attempt":1,"session_id":"child2","status":"completed","deleted_session_ids":["grandchild1","child2"]})
        );
    }
}
