//! Filtered, replayable registration surface for deletion snapshots: the
//! `harness::session-tree-deletion` trigger type and, next to it, the
//! owner-delivered observable function `harness::on-session-tree-deletion`
//! (iii 0.25) whose args are the trigger config. Observers are filed in the
//! same bus as bindings and receive every snapshot a matching binding
//! receives, through `obs.call`; invoking the function is refused with
//! FORBIDDEN.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    Error, IIIClient, ObserveHandler, ObserveRejection, Observer, RegisterTriggerType,
    TriggerAction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Value;

use crate::error::HarnessError;
use crate::functions::delete_session_tree::Snapshot;
use crate::observe::{self, codes, EventObserver};

pub const TYPE: &str = "harness::session-tree-deletion";
/// The observable function next to [`TYPE`].
pub const ON_SESSION_TREE_DELETION: &str = "harness::on-session-tree-deletion";
const ON_SESSION_TREE_DELETION_DESC: &str = "A subtree deletion changed status. Observe it \
    with args { session_id?, operation_id? } before requesting deletion; each observer receives \
    the snapshot { operation_id, attempt, session_id, status, deleted_session_ids, error? }. \
    Only the harness emits it: invoking it directly is refused.";

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

/// One observer of `harness::on-session-tree-deletion` with its parsed args.
#[derive(Clone)]
struct ObserverBinding {
    observer: Arc<dyn EventObserver>,
    filter: DeletionConfig,
}

/// The `harness::on-session-tree-deletion` registration of each client,
/// keyed by the client's address.
static OBSERVABLES: OnceLock<Mutex<HashMap<usize, FunctionRef>>> = OnceLock::new();

#[derive(Clone)]
pub struct DeletionEvents {
    iii: Arc<IIIClient>,
    bindings: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverBinding>>>,
}

impl DeletionEvents {
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let bus = Self {
            iii: iii.clone(),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            observers: Arc::new(Mutex::new(HashMap::new())),
        };
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(RegisterTriggerType::new(TYPE,
            "A subtree deletion changed status; subscribe before requesting deletion and recover once by operation_id.", bus.clone())
            .trigger_request_format::<DeletionConfig>().call_request_format::<Snapshot>());
        // One observable function per client: registering the bus again on
        // the same client (a runtime reset builds fresh `Deps`) replaces the
        // previous one and its observers' bus, as the trigger type's handler
        // is replaced, instead of tripping the SDK's duplicate-id panic.
        let key = Arc::as_ptr(iii) as usize;
        let mut registered = OBSERVABLES
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = registered.remove(&key) {
            previous.unregister();
        }
        let function = observe::register_observable::<DeletionConfig, Snapshot, _>(
            iii,
            ON_SESSION_TREE_DELETION,
            ON_SESSION_TREE_DELETION_DESC,
            DeletionObserveHandler { bus: bus.clone() },
        );
        registered.insert(key, function);
        drop(registered);
        bus
    }

    /// Parse an observer's args (the trigger config) and file it. Malformed
    /// args reject the observer, as they reject a binding.
    fn add_observer(
        &self,
        observer: Arc<dyn EventObserver>,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let filter = serde_json::from_value::<DeletionConfig>(args.clone())
            .map_err(|e| format!("invalid deletion filter: {e}"))?;
        let id = observer.id().to_string();
        self.observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, ObserverBinding { observer, filter });
        Ok(())
    }

    fn remove_observer(&self, id: &str) {
        self.observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    pub async fn emit(&self, snapshot: &Snapshot) -> Result<(), HarnessError> {
        // Observers first: their delivery is fire-and-forget, so a failed
        // binding delivery below never keeps a snapshot from them.
        let observers: Vec<_> = self
            .observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|o| o.filter.matches(snapshot))
            .cloned()
            .collect();
        if !observers.is_empty() {
            let event = serde_json::to_value(snapshot)
                .map_err(|e| HarnessError::Internal(e.to_string()))?;
            for entry in observers {
                entry.observer.deliver(event.clone());
            }
        }
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

/// Files each observer of `harness::on-session-tree-deletion` in the bus the
/// trigger type uses, and takes it out again when the engine removes it.
struct DeletionObserveHandler {
    bus: DeletionEvents,
}

#[async_trait]
impl ObserveHandler for DeletionObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.bus
            .add_observer(Arc::new(obs.clone()), obs.args_value())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), function_id = %obs.function_id(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.bus.remove_observer(obs.id());
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

#[cfg(test)]
mod observable_tests {
    use super::*;
    use crate::functions::delete_session_tree::DeletionStatus;
    use serde_json::json;

    struct Recorder {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn deliver(&self, event: Value) {
            self.seen.lock().unwrap().push(event);
        }
    }

    fn recorder(id: &str) -> Arc<Recorder> {
        Arc::new(Recorder {
            id: id.to_string(),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn bus() -> DeletionEvents {
        DeletionEvents {
            iii: Arc::new(IIIClient::new("ws://127.0.0.1:9")),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            observers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn snapshot(session_id: &str, operation_id: &str) -> Snapshot {
        Snapshot {
            operation_id: operation_id.into(),
            attempt: 1,
            session_id: session_id.into(),
            status: DeletionStatus::Completed,
            deleted_session_ids: vec![session_id.into()],
            error: None,
        }
    }

    #[test]
    fn on_session_tree_deletion_refuses_direct_invocation() {
        assert_eq!(
            ON_SESSION_TREE_DELETION,
            TYPE.replacen("harness::", "harness::on-", 1)
        );
        let err = observe::refuse_invocation(ON_SESSION_TREE_DELETION);
        assert!(
            matches!(&err, Error::Remote { code, message, .. }
                if code == codes::FORBIDDEN && message.contains(ON_SESSION_TREE_DELETION)),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_observer_receives_the_snapshot_a_binding_receives() {
        let bus = bus();
        bus.register_trigger(TriggerConfig {
            id: "b1".into(),
            function_id: "probe::b1".into(),
            config: json!({ "operation_id": "op1" }),
            metadata: None,
            namespace: None,
        })
        .await
        .unwrap();
        let same = recorder("o-same");
        let other = recorder("o-other");
        bus.add_observer(same.clone(), &json!({ "operation_id": "op1" }))
            .unwrap();
        bus.add_observer(other.clone(), &json!({ "operation_id": "op2" }))
            .unwrap();
        let event = snapshot("child", "op1");
        // The binding wants the same snapshot the observer does.
        assert!(bus.bindings.lock().unwrap()["b1"].filter.matches(&event));

        // The binding's fire goes to an engine that is not there; the
        // observer has its snapshot either way.
        let _ = bus.emit(&event).await;
        assert_eq!(
            *same.seen.lock().unwrap(),
            vec![serde_json::to_value(&event).unwrap()]
        );
        assert!(other.seen.lock().unwrap().is_empty());

        bus.remove_observer("o-same");
        let _ = bus.emit(&event).await;
        assert_eq!(same.seen.lock().unwrap().len(), 1);
        assert_eq!(bus.bindings.lock().unwrap().len(), 1, "bindings unchanged");
    }

    #[test]
    fn observer_args_follow_the_binding_config_rules() {
        let bus = bus();
        let err = bus
            .add_observer(recorder("o-bad"), &json!({ "sesion_id": "s" }))
            .unwrap_err();
        assert!(err.contains("invalid deletion filter"), "{err}");
        bus.add_observer(recorder("o-all"), &json!({})).unwrap();
        assert_eq!(bus.observers.lock().unwrap().len(), 1);
        assert!(bus.bindings.lock().unwrap().is_empty());
    }
}
