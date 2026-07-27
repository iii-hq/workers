//! Bridges engine trigger bindings into the fan-out table. Mirrors
//! http/src/trigger.rs: table keyed by trigger id because the SDK only
//! populates `id` on unregister.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Trigger config schema — field-parity with the engine's StateTriggerConfig
/// (engine/src/workers/state/trigger.rs:18-23).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateTriggerSpec {
    /// Only fire for writes in this scope (any scope when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Only fire for writes to this key (any key when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Optional function evaluated per event; only an explicit `false` skips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_function_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StateTriggerEntry {
    pub config: StateTriggerSpec,
    pub function_id: String,
    /// Registration metadata, replayed as the fire-time sidecar. Consumers
    /// like `harness::react` / `harness::notify_agent` resolve which
    /// reaction/subscription fired from it; dropping it makes every fire a
    /// silent no-op on their side.
    pub metadata: Option<serde_json::Value>,
}

pub type TriggerTable = Arc<RwLock<HashMap<String, StateTriggerEntry>>>;

/// Scope/key pre-filter (port of state.rs `state_trigger_matches`): runs before
/// any condition RPC, so a non-matching write costs nothing.
pub fn matches(config: &StateTriggerSpec, event_scope: &str, event_key: &str) -> bool {
    if let Some(scope) = &config.scope
        && scope != event_scope
    {
        return false;
    }
    if let Some(key) = &config.key
        && key != event_key
    {
        return false;
    }
    true
}

pub struct StateTriggerHandler {
    pub triggers: TriggerTable,
}

impl StateTriggerHandler {
    pub fn new() -> Self {
        Self {
            triggers: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for StateTriggerHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TriggerHandler for StateTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let spec: StateTriggerSpec = serde_json::from_value(config.config.clone())
            .map_err(|e| Error::Handler(format!("Failed to parse trigger config: {e}")))?;
        // Builtin parity: duplicate id replaces silently (HashMap insert).
        self.triggers.write().await.insert(
            config.id,
            StateTriggerEntry {
                config: spec,
                function_id: config.function_id,
                metadata: config.metadata,
            },
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.triggers.write().await.remove(&config.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger_config(id: &str, cfg: serde_json::Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: "backend".to_string(),
            config: cfg,
            metadata: None,
            namespace: None,
        }
    }

    #[tokio::test]
    async fn register_stores_and_unregister_removes_by_id_only() {
        let h = StateTriggerHandler::new();
        h.register_trigger(trigger_config("t1", serde_json::json!({"scope": "orders"})))
            .await
            .unwrap();
        assert_eq!(h.triggers.read().await.len(), 1);
        assert!(h.triggers.read().await["t1"].metadata.is_none());
        // SDK sends a stub on unregister: only `id` is real.
        h.unregister_trigger(trigger_config("t1", serde_json::Value::Null))
            .await
            .unwrap();
        assert!(h.triggers.read().await.is_empty());
    }

    #[tokio::test]
    async fn duplicate_id_replaces_silently_builtin_parity() {
        let h = StateTriggerHandler::new();
        h.register_trigger(trigger_config("t1", serde_json::json!({"scope": "a"})))
            .await
            .unwrap();
        h.register_trigger(trigger_config("t1", serde_json::json!({"scope": "b"})))
            .await
            .unwrap();
        let t = h.triggers.read().await;
        assert_eq!(t.len(), 1);
        assert_eq!(t["t1"].config.scope.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn register_retains_metadata_for_fire_time_sidecar() {
        let h = StateTriggerHandler::new();
        let mut cfg = trigger_config("t1", serde_json::json!({"key": "change"}));
        cfg.metadata = Some(serde_json::json!({"task": "react to it", "session_id": "s1"}));
        h.register_trigger(cfg).await.unwrap();
        let t = h.triggers.read().await;
        assert_eq!(
            t["t1"].metadata.as_ref().unwrap()["task"],
            "react to it",
            "metadata must survive registration — it is the fire-time sidecar"
        );
    }

    #[tokio::test]
    async fn invalid_config_errors() {
        let h = StateTriggerHandler::new();
        let err = h
            .register_trigger(trigger_config("t1", serde_json::json!({"scope": 123})))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to parse trigger config"));
    }

    #[tokio::test]
    async fn unregister_unknown_id_is_ok() {
        let h = StateTriggerHandler::new();
        h.unregister_trigger(trigger_config("ghost", serde_json::Value::Null))
            .await
            .unwrap();
    }

    #[test]
    fn matches_respects_scope_and_key() {
        let cfg = |scope: Option<&str>, key: Option<&str>| StateTriggerSpec {
            scope: scope.map(String::from),
            key: key.map(String::from),
            condition_function_id: None,
        };
        assert!(matches(&cfg(None, None), "orders", "o1"));
        assert!(matches(&cfg(Some("orders"), None), "orders", "o1"));
        assert!(!matches(&cfg(Some("orders"), None), "users", "o1"));
        assert!(matches(&cfg(None, Some("special")), "s", "special"));
        assert!(!matches(&cfg(None, Some("special")), "s", "other"));
        assert!(!matches(&cfg(Some("orders"), Some("o1")), "orders", "o2"));
    }
}
