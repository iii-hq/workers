//! The three custom trigger types this worker emits, and the fan-out
//! machinery behind them (README § Events).
//!
//! Reactivity model: consumers bind handlers to these trigger types with the
//! standard two-step pattern; the engine routes each registration to our
//! [`TriggerHandler`]s. Delivery is fire-and-forget (`TriggerAction::Void`);
//! failures are logged and swallowed so a misbehaving subscriber must not
//! break the write path.
//!
//! After a worker restart the engine replays every existing trigger
//! registration of a re-registered type back to its owner, so the
//! subscriber sets rebuild themselves.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fires when a provider reconciles its catalog slice.
pub const MODELS_CHANGED: &str = "router::models::changed";
/// Fires when the provider registry changes (declare / availability flip).
pub const PROVIDER_CHANGED: &str = "router::provider::changed";
/// Fires once when the router finishes booting; providers re-declare on it.
pub const READY: &str = "router::ready";

const READY_DESC: &str =
    "The router finished booting; providers bind here and re-declare after a restart.";
const MODELS_CHANGED_DESC: &str =
    "A provider reconciled its catalog slice. Payload: { provider, count }.";
const PROVIDER_CHANGED_DESC: &str =
    "The provider registry changed (declare / availability flip). Payload: { provider, op }.";

/// Config accepted by all three router event bindings. No filters in v1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventBindingConfig {}

/// Thread-safe subscriber registry keyed by trigger-instance id.
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, TriggerConfig>>>,
}

impl SubscriberSet {
    fn insert(&self, config: TriggerConfig) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.insert(config.id.clone(), config);
    }

    fn remove(&self, id: &str) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(id);
    }

    /// Snapshot so the mutex isn't held across awaits.
    pub fn function_ids(&self) -> Vec<String> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.values().map(|c| c.function_id.clone()).collect()
    }
}

struct RouterEventTriggerHandler {
    name: String,
    subscribers: SubscriberSet,
}

impl RouterEventTriggerHandler {
    fn new(name: &str, subscribers: SubscriberSet) -> Self {
        Self {
            name: name.into(),
            subscribers,
        }
    }
}

#[async_trait]
impl TriggerHandler for RouterEventTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let raw = if config.config.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            config.config.clone()
        };
        serde_json::from_value::<EventBindingConfig>(raw)
            .map_err(|e| Error::Handler(format!("invalid {} config: {e}", self.name)))?;
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            "trigger subscription unregistered"
        );
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

/// Fan-out handle: register once at boot, clone into handlers that emit.
pub struct RouterEvents {
    iii: IIIClient,
    ready: SubscriberSet,
    models_changed: SubscriberSet,
    provider_changed: SubscriberSet,
}

impl RouterEvents {
    /// Register the three custom trigger types. Must run before emitting
    /// `router::ready` so the engine can replay existing bindings first.
    pub fn register(iii: &IIIClient) -> Arc<Self> {
        let ready = SubscriberSet::default();
        let models_changed = SubscriberSet::default();
        let provider_changed = SubscriberSet::default();

        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                READY,
                READY_DESC,
                RouterEventTriggerHandler::new(READY, ready.clone()),
            )
            .trigger_request_format::<EventBindingConfig>(),
        );
        tracing::info!(trigger_type = READY, "registered trigger type");

        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                MODELS_CHANGED,
                MODELS_CHANGED_DESC,
                RouterEventTriggerHandler::new(MODELS_CHANGED, models_changed.clone()),
            )
            .trigger_request_format::<EventBindingConfig>(),
        );
        tracing::info!(trigger_type = MODELS_CHANGED, "registered trigger type");

        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                PROVIDER_CHANGED,
                PROVIDER_CHANGED_DESC,
                RouterEventTriggerHandler::new(PROVIDER_CHANGED, provider_changed.clone()),
            )
            .trigger_request_format::<EventBindingConfig>(),
        );
        tracing::info!(trigger_type = PROVIDER_CHANGED, "registered trigger type");

        Arc::new(Self {
            iii: iii.clone(),
            ready,
            models_changed,
            provider_changed,
        })
    }

    fn subscribers_for(&self, trigger_type: &str) -> Option<&SubscriberSet> {
        match trigger_type {
            READY => Some(&self.ready),
            MODELS_CHANGED => Some(&self.models_changed),
            PROVIDER_CHANGED => Some(&self.provider_changed),
            _ => None,
        }
    }

    /// Fire `payload` to every subscriber bound to `trigger_type`.
    pub async fn emit(&self, trigger_type: &str, payload: Value) {
        let Some(set) = self.subscribers_for(trigger_type) else {
            tracing::warn!(trigger_type, "unknown router event type");
            return;
        };
        let targets = set.function_ids();
        for function_id in targets {
            let res = self
                .iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(
                    trigger_type,
                    function_id = %function_id,
                    error = %e,
                    "router event fan-out failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriber_set_insert_remove_and_snapshot() {
        let set = SubscriberSet::default();
        assert!(set.function_ids().is_empty());
        set.insert(TriggerConfig {
            id: "sub-1".into(),
            function_id: "probe::on_ready".into(),
            config: Value::Object(serde_json::Map::new()),
            metadata: None,
            namespace: None,
        });
        set.insert(TriggerConfig {
            id: "sub-2".into(),
            function_id: "other::recv".into(),
            config: Value::Object(serde_json::Map::new()),
            metadata: None,
            namespace: None,
        });
        let mut fns = set.function_ids();
        fns.sort();
        assert_eq!(
            fns,
            vec!["other::recv".to_string(), "probe::on_ready".to_string()]
        );
        set.remove("sub-1");
        assert_eq!(set.function_ids(), vec!["other::recv".to_string()]);
    }
}
