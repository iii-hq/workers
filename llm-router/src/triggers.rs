//! Owner side of the router's custom iii trigger types (README § Trigger
//! delivery): keep the subscriber set, dispatch at-least-once, isolate
//! failures. Each type is declared with `iii.register_trigger_type`; the
//! engine replays existing registrations on (re)registration of the type,
//! which rebuilds the set after a restart. Dispatch invokes each subscriber's
//! iii function via `iii.trigger`.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iii_sdk::{IIIError, RegisterTriggerType, TriggerConfig, TriggerHandler, TriggerRequest, III};
use serde_json::Value;

const TYPES: [(&str, &str); 3] = [
    (
        "router::models::changed",
        "Fires when a provider reconciles its catalog slice.",
    ),
    (
        "router::provider::changed",
        "Fires when the provider registry changes (declare / availability flip).",
    ),
    (
        "router::ready",
        "Fires once when the router finishes booting; providers re-declare on it.",
    ),
];

/// trigger type -> [(trigger_id, function_id)]
type SubscriberMap = HashMap<String, Vec<(String, String)>>;

/// Maintains one trigger type's subscriber list from the engine's
/// register/unregister callbacks.
struct SubscriberTracker {
    type_id: &'static str,
    subscribers: Arc<Mutex<SubscriberMap>>,
}

#[async_trait::async_trait]
impl TriggerHandler for SubscriberTracker {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let mut map = self.subscribers.lock().unwrap();
        let list = map.entry(self.type_id.to_string()).or_default();
        if !list.iter().any(|(id, _)| id == &config.id) {
            list.push((config.id, config.function_id));
        }
        Ok(())
    }
    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let mut map = self.subscribers.lock().unwrap();
        if let Some(list) = map.get_mut(self.type_id) {
            list.retain(|(id, _)| id != &config.id);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct TriggerEmitter {
    iii: III,
    subscribers: Arc<Mutex<SubscriberMap>>,
}

impl TriggerEmitter {
    pub fn install(iii: &III) -> Self {
        let emitter = TriggerEmitter {
            iii: iii.clone(),
            subscribers: Arc::new(Mutex::new(HashMap::new())),
        };
        for (type_id, description) in TYPES {
            let _ = iii.register_trigger_type(RegisterTriggerType::new(
                type_id,
                description,
                SubscriberTracker {
                    type_id,
                    subscribers: emitter.subscribers.clone(),
                },
            ));
        }
        emitter
    }

    pub fn subscriber_count(&self, type_id: &str) -> usize {
        self.subscribers
            .lock()
            .unwrap()
            .get(type_id)
            .map_or(0, Vec::len)
    }

    /// Fire-and-forget dispatch to every subscriber; failures are isolated.
    pub async fn emit(&self, type_id: &str, payload: Value) {
        let targets: Vec<String> = self
            .subscribers
            .lock()
            .unwrap()
            .get(type_id)
            .map(|l| l.iter().map(|(_, fid)| fid.clone()).collect())
            .unwrap_or_default();
        for fid in targets {
            let _ = self
                .iii
                .trigger(TriggerRequest {
                    function_id: fid,
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: None,
                })
                .await;
        }
    }
}
