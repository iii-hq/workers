//! The `secrets::changed` trigger type.
//!
//! Fires after every create, rotation, deletion and allowlist change with
//! metadata only, never a value. Config `{names?: string[]}` narrows it to
//! some secrets (bare names or `secret://NAME`). Bindings live in an
//! in-process map keyed by trigger id; delivery is fire-and-forget (`Void`)
//! in the binding's namespace, with the binding's metadata as the sidecar.
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::ids::CHANGED_TRIGGER;
use crate::names::{parse_reference, reference_for};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Created,
    Rotated,
    Deleted,
    AccessChanged,
}

/// A `secrets::changed` delivery. Never carries a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedEvent {
    pub name: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub action: ChangeAction,
    /// The current fingerprint; absent after a deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub updated_at: String,
}

impl ChangedEvent {
    pub fn new(
        name: &str,
        action: ChangeAction,
        fingerprint: Option<String>,
        updated_at: &str,
    ) -> Self {
        Self {
            name: name.to_owned(),
            reference: reference_for(name),
            action,
            fingerprint,
            updated_at: updated_at.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangedConfig {
    /// Secret names (or `secret://NAME` references) to fire for. Omit or
    /// leave empty for every secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<Vec<String>>,
    /// Metadata attached to every invocation this binding receives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

pub type Bindings = Arc<RwLock<HashMap<String, TriggerConfig>>>;

#[derive(Clone, Default)]
pub struct Subscribers(pub Bindings);

impl Subscribers {
    pub fn len(&self) -> usize {
        self.0.read().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Remember a binding, forget it on unregister. A malformed config is
/// accepted and simply never matches.
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

pub fn register_trigger_type(iii: &IIIClient, subscribers: &Subscribers) {
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGED_TRIGGER,
            "Fires after a secret is created, rotated, deleted or its consumers change. Carries name, ref, action, fingerprint and updated_at, never the value. Config: { names? } to narrow to some secrets.",
            BindingTable(subscribers.0.clone()),
        )
        .trigger_request_format::<ChangedConfig>()
        .call_request_format::<ChangedEvent>(),
    );
}

/// Whether a binding's config wants `name`. A missing or empty `names` list
/// matches everything; a `names` that is not a list matches nothing.
pub fn matches(config: &Value, name: &str) -> bool {
    match config.get("names") {
        None | Some(Value::Null) => true,
        Some(Value::Array(entries)) => {
            let mut wanted = entries
                .iter()
                .filter_map(Value::as_str)
                .filter(|entry| !entry.trim().is_empty())
                .peekable();
            if wanted.peek().is_none() {
                return true;
            }
            wanted.any(|entry| parse_reference(entry).is_ok_and(|wanted| wanted == name))
        }
        Some(_) => false,
    }
}

fn subscription_metadata(binding: &TriggerConfig) -> Option<Value> {
    binding
        .config
        .get("metadata")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| binding.metadata.clone())
}

pub fn emit(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &ChangedEvent) {
    let bindings: Vec<TriggerConfig> = subscribers
        .0
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|binding| matches(&binding.config, &event.name))
        .cloned()
        .collect();
    if bindings.is_empty() {
        return;
    }
    let payload = match serde_json::to_value(event) {
        Ok(payload) => payload,
        Err(_) => return,
    };
    let iii = iii.clone();
    tokio::spawn(async move {
        for binding in bindings {
            let mut request: TriggerRequestWithMetadata = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: payload.clone(),
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
            if let Err(error) = iii.trigger(request).await {
                tracing::warn!(
                    function_id = binding.function_id,
                    error = %error,
                    "secrets::changed delivery failed"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_filter() {
        assert!(matches(&json!({}), "A"));
        assert!(matches(&json!({"names": null}), "A"));
        assert!(matches(&json!({"names": []}), "A"));
        assert!(matches(&json!({"names": ["", "  "]}), "A"));
        assert!(matches(&json!({"names": ["A", "B"]}), "A"));
        assert!(matches(&json!({"names": ["secret://A"]}), "A"));
        assert!(!matches(&json!({"names": ["B"]}), "A"));
        assert!(!matches(&json!({"names": "A"}), "A"));
    }

    #[test]
    fn event_shape_never_has_a_value() {
        let event = ChangedEvent::new("A", ChangeAction::AccessChanged, Some("00ff".into()), "t");
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({"name":"A","ref":"secret://A","action":"access_changed","fingerprint":"00ff","updated_at":"t"})
        );
        let deleted = ChangedEvent::new("A", ChangeAction::Deleted, None, "t");
        assert!(serde_json::to_value(deleted)
            .unwrap()
            .get("fingerprint")
            .is_none());
    }
}
