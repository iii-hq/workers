//! The `secrets::changed` trigger type.
//!
//! Fires after every create, rotation, deletion and allowlist change with
//! metadata only, never a value; for the env store, also when an edit to
//! `.env` sets, changes or removes a shared variable. Config
//! `{names?: string[]}` narrows it to some secrets: a bare name matches both
//! stores, `secret://NAME` / `env://NAME` only one. Bindings live in an
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
use crate::api::StoreKind;
use crate::names::{is_valid_name, reference_for, split_reference};

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
    /// `secret://NAME` or `env://NAME`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub action: ChangeAction,
    /// The current fingerprint; absent after a deletion and for the env store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub updated_at: String,
}

impl ChangedEvent {
    /// A vault secret's event.
    pub fn new(
        name: &str,
        action: ChangeAction,
        fingerprint: Option<String>,
        updated_at: &str,
    ) -> Self {
        Self {
            name: name.to_owned(),
            reference: reference_for(StoreKind::Vault, name),
            action,
            fingerprint,
            updated_at: updated_at.to_owned(),
        }
    }

    /// A shared environment variable's event.
    pub fn env(name: &str, action: ChangeAction, updated_at: &str) -> Self {
        Self {
            name: name.to_owned(),
            reference: reference_for(StoreKind::Env, name),
            action,
            fingerprint: None,
            updated_at: updated_at.to_owned(),
        }
    }

    /// The store the event is about, from its reference.
    pub fn store(&self) -> StoreKind {
        split_reference(&self.reference)
            .0
            .unwrap_or(StoreKind::Vault)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangedConfig {
    /// Secret names to fire for: a bare `NAME` for both stores, or
    /// `secret://NAME` / `env://NAME` for one. Omit or leave empty for every
    /// secret.
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

/// Whether a binding's config wants `name` in `store`. A missing or empty
/// `names` list matches everything; a `names` that is not a list matches
/// nothing.
pub fn matches(config: &Value, store: StoreKind, name: &str) -> bool {
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
            wanted.any(|entry| {
                let (wanted_store, wanted) = split_reference(entry);
                is_valid_name(wanted)
                    && wanted == name
                    && wanted_store.is_none_or(|wanted_store| wanted_store == store)
            })
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
        .filter(|binding| matches(&binding.config, event.store(), &event.name))
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
        let vault = StoreKind::Vault;
        assert!(matches(&json!({}), vault, "A"));
        assert!(matches(&json!({"names": null}), vault, "A"));
        assert!(matches(&json!({"names": []}), vault, "A"));
        assert!(matches(&json!({"names": ["", "  "]}), vault, "A"));
        assert!(matches(&json!({"names": ["A", "B"]}), vault, "A"));
        assert!(matches(&json!({"names": ["secret://A"]}), vault, "A"));
        assert!(!matches(&json!({"names": ["B"]}), vault, "A"));
        assert!(!matches(&json!({"names": "A"}), vault, "A"));
        // A bare name follows both stores; a reference only its own.
        assert!(matches(&json!({"names": ["A"]}), StoreKind::Env, "A"));
        assert!(matches(&json!({"names": ["env://A"]}), StoreKind::Env, "A"));
        assert!(!matches(&json!({"names": ["env://A"]}), vault, "A"));
        assert!(!matches(
            &json!({"names": ["secret://A"]}),
            StoreKind::Env,
            "A"
        ));
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
        let env = ChangedEvent::env("A", ChangeAction::Rotated, "t");
        assert_eq!(env.store(), StoreKind::Env);
        assert_eq!(
            serde_json::to_value(&env).unwrap(),
            json!({"name":"A","ref":"env://A","action":"rotated","updated_at":"t"})
        );
    }
}
