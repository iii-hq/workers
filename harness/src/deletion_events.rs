//! Filtered, replayable registration surface for deletion snapshots.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{Error, IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

use crate::error::HarnessError;
use crate::functions::delete_session_tree::Snapshot;

pub const TYPE: &str = "harness::session-tree-deletion";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeletionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

impl DeletionConfig {
    fn matches(&self, event: &Snapshot) -> bool {
        self.session_id
            .as_ref()
            .is_none_or(|id| id == &event.session_id)
            && self
                .operation_id
                .as_ref()
                .is_none_or(|id| id == &event.operation_id)
    }
}

#[derive(Clone)]
struct Binding {
    target: TriggerConfig,
    filter: DeletionConfig,
}

#[derive(Clone)]
pub struct DeletionEvents {
    iii: Arc<IIIClient>,
    bindings: Arc<Mutex<HashMap<String, Binding>>>,
}

impl DeletionEvents {
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let bus = Self {
            iii: iii.clone(),
            bindings: Arc::new(Mutex::new(HashMap::new())),
        };
        iii.register_trigger_type(RegisterTriggerType::new(TYPE,
            "A subtree deletion changed status; subscribe before requesting deletion and recover once by operation_id.", bus.clone())
            .trigger_request_format::<DeletionConfig>().call_request_format::<Snapshot>());
        bus
    }

    pub async fn emit(&self, snapshot: &Snapshot) -> Result<(), HarnessError> {
        let bindings: Vec<_> = self
            .bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        for binding in bindings {
            if !binding.filter.matches(snapshot) {
                continue;
            }
            let request = TriggerRequest {
                function_id: binding.target.function_id,
                payload: serde_json::to_value(snapshot)
                    .map_err(|e| HarnessError::Internal(e.to_string()))?,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let request = match binding.target.metadata {
                Some(metadata) => request.metadata(metadata),
                None => request.into(),
            };
            self.iii
                .trigger(
                    request.namespace(binding.target.namespace.as_deref().unwrap_or("default")),
                )
                .await
                .map_err(|e| HarnessError::Dependency(format!("deletion event delivery: {e}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl TriggerHandler for DeletionEvents {
    async fn register_trigger(&self, target: TriggerConfig) -> Result<(), Error> {
        let filter = serde_json::from_value::<DeletionConfig>(target.config.clone())
            .map_err(|e| Error::Handler(format!("invalid deletion filter: {e}")))?;
        self.bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(target.id.clone(), Binding { target, filter });
        Ok(())
    }
    async fn unregister_trigger(&self, target: TriggerConfig) -> Result<(), Error> {
        self.bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&target.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::delete_session_tree::DeletionStatus;
    #[test]
    fn filters_are_conjunctive_and_snapshot_has_no_extra_fields() {
        let snapshot = Snapshot {
            operation_id: "op".into(),
            attempt: 1,
            session_id: "child2".into(),
            status: DeletionStatus::Completed,
            deleted_session_ids: vec!["grandchild1".into(), "child2".into()],
            error: None,
        };
        let config = DeletionConfig {
            session_id: Some("child2".into()),
            operation_id: Some("op".into()),
        };
        assert!(config.matches(&snapshot));
        assert!(!DeletionConfig {
            operation_id: Some("other".into()),
            ..config
        }
        .matches(&snapshot));
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::json!({"operation_id":"op","attempt":1,"session_id":"child2","status":"completed","deleted_session_ids":["grandchild1","child2"]})
        );
        assert!(serde_json::from_value::<DeletionConfig>(Value::Null).is_err());
    }
}
