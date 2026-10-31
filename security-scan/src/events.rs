//! Worker-owned change notifications for the records this worker stores.
//!
//! Runs, reconciliation snapshots and finding actions live in the worker's
//! private `state` scopes and are read through `security-scan::list`,
//! `security-scan::read`, `security-scan::reconciliation` and
//! `security-scan::action-read`. After a compare-and-set commits a change, the
//! worker calls one of the functions below with a small public projection: the
//! record id and its `updated_at` revision hint, never the record itself.
//! Consumers observe the function, do an initial read, re-read on each call,
//! and re-read everything after a reconnect, because calls are not replayed.
//!
//! Each function is private (only this worker calls it), takes no args, and
//! returns the event it is called with: that event is what observers observe.
//! Consumers select calls with filters on `input.repository` and
//! `input.run_id`. The worker calls a function only while something observes
//! it, fire-and-forget (`CallAction::Void`), so the write path never waits on
//! a consumer. Nothing is spawned per event and nothing is queued in this
//! worker.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{RunRecordV1, RunStatusV1, SecurityActionRecordV1, SecurityActionStatusV1};

pub const ON_RUN_CHANGED_FN: &str = "security-scan::on-run-changed";
pub const ON_RECONCILIATION_CHANGED_FN: &str = "security-scan::on-reconciliation-changed";
pub const ON_ACTION_CHANGED_FN: &str = "security-scan::on-action-changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Run,
    Reconciliation,
    Action,
}

impl ChangeKind {
    pub const ALL: [ChangeKind; 3] = [
        ChangeKind::Run,
        ChangeKind::Reconciliation,
        ChangeKind::Action,
    ];

    pub fn function_id(self) -> &'static str {
        match self {
            ChangeKind::Run => ON_RUN_CHANGED_FN,
            ChangeKind::Reconciliation => ON_RECONCILIATION_CHANGED_FN,
            ChangeKind::Action => ON_ACTION_CHANGED_FN,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ChangeKind::Run => {
                "Called after a security scan run was created or changed. Event { run_id, \
                 repository, status, attempt, updated_at, completed_at }; re-read \
                 security-scan::list or security-scan::read. No args; filter on \
                 input.repository or input.run_id."
            }
            ChangeKind::Reconciliation => {
                "Called after a run's GitHub alert reconciliation snapshot was saved. Event \
                 { run_id, repository }; re-read security-scan::reconciliation. No args; filter \
                 on input.repository or input.run_id."
            }
            ChangeKind::Action => {
                "Called after a finding action (issue or fix) was created or changed. Event \
                 { action_id, run_id, repository, status, updated_at }; re-read \
                 security-scan::action-read. No args; filter on input.repository or \
                 input.run_id."
            }
        }
    }
}

/// `security-scan::on-run-changed` event: the public status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunChangedEventV1 {
    pub run_id: String,
    pub repository: String,
    pub status: RunStatusV1,
    pub attempt: u32,
    /// Revision hint: set on every committed write (ms).
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

impl From<&RunRecordV1> for RunChangedEventV1 {
    fn from(run: &RunRecordV1) -> Self {
        Self {
            run_id: run.run_id.clone(),
            repository: run.repository.clone(),
            status: run.status,
            attempt: run.attempt,
            updated_at: run.updated_at,
            completed_at: run.completed_at,
        }
    }
}

/// `security-scan::on-reconciliation-changed` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReconciliationChangedEventV1 {
    pub run_id: String,
    pub repository: String,
}

/// `security-scan::on-action-changed` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionChangedEventV1 {
    pub action_id: String,
    pub run_id: String,
    pub repository: String,
    pub status: SecurityActionStatusV1,
    /// Revision hint: set on every committed write (ms).
    pub updated_at: i64,
}

impl From<&SecurityActionRecordV1> for ActionChangedEventV1 {
    fn from(action: &SecurityActionRecordV1) -> Self {
        Self {
            action_id: action.action_id.clone(),
            run_id: action.run_id.clone(),
            repository: action.repository.clone(),
            status: action.status,
            updated_at: action.updated_at,
        }
    }
}

/// Registers the three change functions: private, no args, each handler
/// returning the event it is called with. Call before the functions and UI
/// that consumers pair with them.
pub fn register_functions(iii: &IIIClient) {
    iii.register_function(
        ON_RUN_CHANGED_FN,
        RegisterFunction::new(|event: RunChangedEventV1| Ok::<_, Error>(event))
            .description(ChangeKind::Run.description())
            .private(),
    );
    iii.register_function(
        ON_RECONCILIATION_CHANGED_FN,
        RegisterFunction::new(|event: ReconciliationChangedEventV1| Ok::<_, Error>(event))
            .description(ChangeKind::Reconciliation.description())
            .private(),
    );
    iii.register_function(
        ON_ACTION_CHANGED_FN,
        RegisterFunction::new(|event: ActionChangedEventV1| Ok::<_, Error>(event))
            .description(ChangeKind::Action.description())
            .private(),
    );
}

/// Calls the change functions over the worker's engine connection.
pub struct ChangeFeed {
    iii: Arc<IIIClient>,
}

impl ChangeFeed {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }

    /// Calls `kind`'s function with `event` when anything observes it. Call
    /// only after the change is committed. Returns whether the call was
    /// made; a failed call is logged and never fails the caller's write.
    pub async fn emit<T: Serialize>(&self, kind: ChangeKind, event: &T) -> bool {
        let function_id = kind.function_id();
        if !self.iii.is_observed(function_id) {
            return false;
        }
        let payload = match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(function_id, %error, "change event failed to serialize");
                return false;
            }
        };
        match self
            .iii
            .call(CallRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(CallAction::Void),
                timeout_ms: None,
            })
            .await
        {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(function_id, %error, "security scan change notification failed");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `IIIClient::new` builds local state only, so the feed runs without an
    /// engine.
    fn offline_feed() -> ChangeFeed {
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        register_functions(&iii);
        ChangeFeed::new(iii)
    }

    #[tokio::test]
    async fn nothing_is_called_while_a_function_is_unobserved() {
        let feed = offline_feed();
        let event = ReconciliationChangedEventV1 {
            run_id: "sec_1".into(),
            repository: "iii".into(),
        };
        for kind in ChangeKind::ALL {
            assert!(!feed.emit(kind, &event).await, "{}", kind.function_id());
        }
    }

    #[test]
    fn events_serialize_to_their_documented_shape() {
        assert_eq!(
            serde_json::to_value(ReconciliationChangedEventV1 {
                run_id: "sec_1".into(),
                repository: "iii".into(),
            })
            .unwrap(),
            json!({ "run_id": "sec_1", "repository": "iii" })
        );
    }

    #[test]
    fn descriptions_name_the_filterable_fields() {
        for kind in ChangeKind::ALL {
            let description = kind.description();
            assert!(description.contains("input.repository"), "{description}");
            assert!(description.contains("input.run_id"), "{description}");
        }
    }

    #[test]
    fn function_ids_are_owned_by_the_worker() {
        assert_eq!(
            ChangeKind::ALL.map(ChangeKind::function_id),
            [
                "security-scan::on-run-changed",
                "security-scan::on-reconciliation-changed",
                "security-scan::on-action-changed",
            ]
        );
    }
}
