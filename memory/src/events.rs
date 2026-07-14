//! The two custom trigger types this worker emits — `memory::item-changed`
//! and `memory::bank-changed` — using the session-manager reactivity model:
//! consumers bind handlers with the standard two-step pattern; the engine
//! routes each registration to our [`TriggerHandler`]s; delivery is
//! fire-and-forget (`TriggerAction::Void`), at-least-once, unordered. The
//! console memory page live-refreshes off these.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::types::Fact;

pub const ITEM_CHANGED: &str = "memory::item-changed";
pub const ITEM_CHANGED_DESC: &str =
    "A fact was created, updated, superseded, or deleted in a memory bank. \
     Payload: { event_type, bank, fact }. Bind console/live views here.";

pub const BANK_CHANGED: &str = "memory::bank-changed";
pub const BANK_CHANGED_DESC: &str =
    "A memory bank was created or trashed. Payload: { event_type, bank }.";

/// Config accepted by both trigger types.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this bank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank: Option<String>,
}

/// `null` configs are treated as `{}`; unknown fields are rejected so a
/// misspelled filter key fails at registration, not silently.
pub fn parse_binding_config(raw: &Value) -> Result<BindingConfig, String> {
    if raw.is_null() {
        return Ok(BindingConfig::default());
    }
    serde_json::from_value(raw.clone()).map_err(|e| format!("invalid binding config: {e}"))
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type.
#[derive(Clone)]
pub struct SubscriberSet {
    trigger_type: &'static str,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    fn new(trigger_type: &'static str) -> Self {
        Self {
            trigger_type,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = parse_binding_config(&config.config)?;
        self.lock().insert(
            config.id,
            Binding {
                function_id: config.function_id,
                filter,
            },
        );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[derive(Clone)]
pub struct TriggerSets {
    pub items: SubscriberSet,
    pub banks: SubscriberSet,
}

struct MemoryTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for MemoryTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = self.set.trigger_type, id = %id, "trigger subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// Register both trigger types with the engine. Must run **before**
/// `functions::register_all` so handlers capture the subscriber sets they
/// fan out to.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets {
        items: SubscriberSet::new(ITEM_CHANGED),
        banks: SubscriberSet::new(BANK_CHANGED),
    };
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            ITEM_CHANGED,
            ITEM_CHANGED_DESC,
            MemoryTriggerHandler {
                set: sets.items.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            BANK_CHANGED,
            BANK_CHANGED_DESC,
            MemoryTriggerHandler {
                set: sets.banks.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );
    tracing::info!(trigger_types = ?[ITEM_CHANGED, BANK_CHANGED], "registered trigger types");
    sets
}

/// Fact event kinds carried in `event_type`.
#[derive(Debug, Clone, Copy)]
pub enum ItemEvent {
    Created,
    Updated,
    Superseded,
    Deleted,
}

impl ItemEvent {
    fn as_str(self) -> &'static str {
        match self {
            ItemEvent::Created => "created",
            ItemEvent::Updated => "updated",
            ItemEvent::Superseded => "superseded",
            ItemEvent::Deleted => "deleted",
        }
    }
}

/// Filtered fire-and-forget fan-out to every matching binding.
pub struct Emitter {
    sets: TriggerSets,
    iii: Arc<IIIClient>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, iii: Arc<IIIClient>) -> Self {
        Self { sets, iii }
    }

    pub async fn item(&self, event: ItemEvent, bank: &str, fact: &Fact) {
        let payload = json!({
            "event_type": event.as_str(),
            "bank": bank,
            "fact": fact,
        });
        self.fan_out(&self.sets.items, bank, payload).await;
    }

    pub async fn bank(&self, event_type: &str, bank: &str) {
        let payload = json!({ "event_type": event_type, "bank": bank });
        self.fan_out(&self.sets.banks, bank, payload).await;
    }

    async fn fan_out(&self, set: &SubscriberSet, bank: &str, payload: Value) {
        for binding in set.snapshot() {
            let matches = binding
                .filter
                .bank
                .as_deref()
                .is_none_or(|want| want == bank);
            if !matches {
                continue;
            }
            let res = self
                .iii
                .trigger(TriggerRequest {
                    function_id: binding.function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(function_id = %binding.function_id, error = %e, "void fan-out failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_config_is_match_all() {
        assert!(parse_binding_config(&Value::Null).unwrap().bank.is_none());
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        assert!(parse_binding_config(&json!({ "bnak": "typo" })).is_err());
    }

    #[test]
    fn bank_filter_parses() {
        let cfg = parse_binding_config(&json!({ "bank": "blog" })).unwrap();
        assert_eq!(cfg.bank.as_deref(), Some("blog"));
    }
}
