//! `sentinel::group-changed` — the surface siblings and the console watch.
//!
//! The event is a doorbell, not a record: a subscriber re-reads the group it
//! names rather than trusting the payload, which is what keeps a page correct
//! when two events cross. Occurrence events are coalesced per group over a
//! second, because an incident with a thousand occurrences must not become a
//! thousand deliveries — while a state change goes out immediately, since
//! that is the one an alert cares about.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::triggers::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{GroupChangedConfigV1, GroupChangedEventV1, GroupChangedOpV1, GroupStatusV1};

pub const GROUP_CHANGED: &str = "sentinel::group-changed";
pub const INVESTIGATION_CHANGED: &str = "sentinel::investigation-changed";

/// How long occurrence events for one group are gathered before delivery.
const COALESCE: Duration = Duration::from_secs(1);

type Bindings = Arc<RwLock<HashMap<String, TriggerConfig>>>;

#[derive(Clone, Default)]
pub struct Subscribers {
    groups: Bindings,
    investigations: Bindings,
}

impl Subscribers {
    pub fn counts(&self) -> (usize, usize) {
        (
            self.groups.read().unwrap_or_else(|p| p.into_inner()).len(),
            self.investigations
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .len(),
        )
    }
}

/// Remember a binding, forget it on unregister. A config that does not parse
/// is kept and simply never matches, rather than refusing the registration.
struct BindingTable(Bindings);

#[async_trait]
impl TriggerHandler for BindingTable {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

pub fn register_trigger_types(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            GROUP_CHANGED,
            "Fires when an error group is created, takes a new occurrence, or changes state. \
             Config: { group_id?, service_name?, ops? } — ops narrows to created, occurrence or \
             status. Occurrence events are coalesced per group over one second; re-read the group \
             rather than trusting the payload.",
            BindingTable(subscribers.groups.clone()),
        )
        .trigger_request_format::<GroupChangedConfigV1>()
        .call_request_format::<GroupChangedEventV1>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            INVESTIGATION_CHANGED,
            "Fires when an investigation is created, starts, or finishes. \
             Config: { group_id? }.",
            BindingTable(subscribers.investigations.clone()),
        )
        .trigger_request_format::<GroupChangedConfigV1>()
        .call_request_format::<Value>(),
    );
}

/// Delivers events to the bindings that asked for them.
pub struct Emitter {
    iii: Arc<IIIClient>,
    subscribers: Subscribers,
    /// Occurrence events waiting out their coalescing window, newest payload
    /// per group.
    pending: Arc<Mutex<HashMap<String, GroupChangedEventV1>>>,
}

impl Emitter {
    pub fn new(iii: Arc<IIIClient>, subscribers: Subscribers) -> Arc<Self> {
        let emitter = Arc::new(Self {
            iii,
            subscribers,
            pending: Arc::new(Mutex::new(HashMap::new())),
        });
        let flusher = emitter.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(COALESCE);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                flusher.flush().await;
            }
        });
        emitter
    }

    /// Send a group event, coalescing the noisy kind.
    pub async fn group_changed(&self, event: GroupChangedEventV1) {
        if event.op == GroupChangedOpV1::Occurrence {
            self.pending
                .lock()
                .await
                .insert(event.group_id.clone(), event);
            return;
        }
        // A creation or a state change supersedes anything gathered for that
        // group: the later event carries the newer count anyway.
        self.pending.lock().await.remove(&event.group_id);
        self.deliver(&self.subscribers.groups, &event).await;
    }

    pub async fn investigation_changed(&self, payload: Value) {
        self.deliver_value(&self.subscribers.investigations, payload, None)
            .await;
    }

    async fn flush(&self) {
        let ready: Vec<GroupChangedEventV1> = {
            let mut pending = self.pending.lock().await;
            pending.drain().map(|(_, event)| event).collect()
        };
        for event in ready {
            self.deliver(&self.subscribers.groups, &event).await;
        }
    }

    async fn deliver(&self, bindings: &Bindings, event: &GroupChangedEventV1) {
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        self.deliver_value(bindings, payload, Some(event)).await;
    }

    async fn deliver_value(
        &self,
        bindings: &Bindings,
        payload: Value,
        event: Option<&GroupChangedEventV1>,
    ) {
        let targets: Vec<TriggerConfig> = bindings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|binding| event.is_none_or(|event| matches(&binding.config, event)))
            .cloned()
            .collect();
        for binding in targets {
            let mut request: TriggerRequestWithMetadata = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: payload.clone(),
                // Fire and forget: a subscriber that is slow or broken must
                // not hold up the pipeline that noticed the error.
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            }
            .into();
            if let Some(metadata) = subscription_metadata(&binding) {
                request = request.metadata(metadata);
            }
            if let Some(namespace) = &binding.namespace {
                request = request.namespace(namespace.clone());
            }
            if let Err(error) = self.iii.trigger(request).await {
                tracing::warn!(
                    function_id = binding.function_id,
                    %error,
                    "sentinel event delivery failed"
                );
            }
        }
    }
}

/// The console puts its per-tab metadata on the binding config; a worker puts
/// it on the registration. Either is passed back on delivery.
fn subscription_metadata(binding: &TriggerConfig) -> Option<Value> {
    binding
        .config
        .get("metadata")
        .cloned()
        .filter(|value| !value.is_null())
        .or_else(|| binding.metadata.clone())
}

/// Whether one binding asked for this event.
fn matches(config: &Value, event: &GroupChangedEventV1) -> bool {
    if let Some(group_id) = config.get("group_id").and_then(Value::as_str) {
        if !group_id.is_empty() && group_id != event.group_id {
            return false;
        }
    }
    if let Some(ops) = config.get("ops").and_then(Value::as_array) {
        let wanted = op_name(event.op);
        if !ops.is_empty() && !ops.iter().any(|op| op.as_str() == Some(wanted)) {
            return false;
        }
    }
    true
}

fn op_name(op: GroupChangedOpV1) -> &'static str {
    match op {
        GroupChangedOpV1::Created => "created",
        GroupChangedOpV1::Occurrence => "occurrence",
        GroupChangedOpV1::Status => "status",
    }
}

/// Build the event for a group the ingest just touched.
pub fn group_event(
    op: GroupChangedOpV1,
    group_id: &str,
    status: GroupStatusV1,
    occurrence_count: u64,
    reason: Option<crate::GroupChangeReasonV1>,
) -> GroupChangedEventV1 {
    GroupChangedEventV1 {
        op,
        group_id: group_id.to_string(),
        status,
        previous_status: None,
        occurrence_count,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(group_id: &str, op: GroupChangedOpV1) -> GroupChangedEventV1 {
        GroupChangedEventV1 {
            op,
            group_id: group_id.into(),
            status: GroupStatusV1::New,
            previous_status: None,
            occurrence_count: 1,
            reason: None,
        }
    }

    #[test]
    fn an_empty_config_subscribes_to_everything() {
        assert!(matches(
            &json!({}),
            &event("grp_1", GroupChangedOpV1::Status)
        ));
    }

    #[test]
    fn a_binding_can_follow_one_group() {
        let config = json!({ "group_id": "grp_1" });
        assert!(matches(&config, &event("grp_1", GroupChangedOpV1::Status)));
        assert!(!matches(&config, &event("grp_2", GroupChangedOpV1::Status)));
    }

    #[test]
    fn a_binding_can_ask_for_state_changes_only() {
        let config = json!({ "ops": ["status", "created"] });
        assert!(matches(&config, &event("grp_1", GroupChangedOpV1::Status)));
        assert!(matches(&config, &event("grp_1", GroupChangedOpV1::Created)));
        assert!(
            !matches(&config, &event("grp_1", GroupChangedOpV1::Occurrence)),
            "an alert does not want a delivery per occurrence"
        );
    }

    #[test]
    fn the_consoles_per_tab_metadata_wins_over_the_registration() {
        let binding = TriggerConfig {
            id: "b1".into(),
            function_id: "iii::console::group-changed".into(),
            config: json!({ "metadata": { "tab": "a" } }),
            metadata: Some(json!({ "tab": "b" })),
            namespace: None,
        };
        assert_eq!(subscription_metadata(&binding), Some(json!({ "tab": "a" })));

        let worker = TriggerConfig {
            id: "b2".into(),
            function_id: "alerts::on-error".into(),
            config: json!({}),
            metadata: Some(json!({ "channel": "#ops" })),
            namespace: None,
        };
        assert_eq!(
            subscription_metadata(&worker),
            Some(json!({ "channel": "#ops" }))
        );
    }
}
