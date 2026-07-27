//! Bridges engine `subscribe` trigger bindings into the hub. Mirrors
//! http/src/trigger.rs: keyed by trigger id because the SDK only populates
//! `id` on unregister. Topic parsing is lenient (builtin parity: a missing
//! topic warns and registers nothing — pubsub.rs:115-139).

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::hub::Hub;

/// Advertised trigger config schema — field parity with the engine's
/// `SubscribeTriggerConfig` (engine/src/trigger_formats.rs:135-141).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeTriggerSpec {
    /// Topic to subscribe to
    pub topic: String,
    /// Optional function ID to evaluate before invoking handler.
    /// Accepted for schema parity; the builtin never evaluated it and neither
    /// does this worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_function_id: Option<String>,
}

pub struct SubscribeTriggerHandler {
    pub hub: Arc<Hub>,
}

impl SubscribeTriggerHandler {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self { hub }
    }
}

#[async_trait]
impl TriggerHandler for SubscribeTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        // Lenient like the builtin: missing/non-string topic -> "" -> the hub
        // warns and registers nothing. Never an error.
        let topic = config
            .config
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        tracing::info!(topic = %topic, function_id = %config.function_id, "PubSub subscription registered");
        self.hub
            .subscribe(&config.id, &topic, &config.function_id)
            .await;
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        // Only `config.id` is populated (SDK stub); the hub knows the topic.
        self.hub.unsubscribe(&config.id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopAdapter;

    #[async_trait]
    impl crate::adapters::PubSubAdapter for NoopAdapter {
        async fn publish(&self, _topic: &str, _data: serde_json::Value) {}
        async fn subscribe(&self, _topic: &str, _id: &str, _function_id: &str) {}
        async fn unsubscribe(&self, _topic: &str, _id: &str) {}
    }

    fn handler() -> SubscribeTriggerHandler {
        SubscribeTriggerHandler::new(Arc::new(Hub::new(Arc::new(NoopAdapter))))
    }

    fn trigger_config(id: &str, cfg: serde_json::Value) -> iii_sdk::trigger::TriggerConfig {
        iii_sdk::trigger::TriggerConfig {
            id: id.to_string(),
            function_id: "test::listener".to_string(),
            config: cfg,
            metadata: None,
            namespace: None,
        }
    }

    #[tokio::test]
    async fn register_subscribes_and_unregister_by_id_only() {
        let h = handler();
        h.register_trigger(trigger_config("t1", serde_json::json!({"topic": "orders"})))
            .await
            .unwrap();
        assert_eq!(h.hub.subscription_count().await, 1);

        // SDK sends a stub on unregister: only `id` is real.
        h.unregister_trigger(trigger_config("t1", serde_json::Value::Null))
            .await
            .unwrap();
        assert_eq!(h.hub.subscription_count().await, 0);
    }

    #[tokio::test]
    async fn register_without_topic_is_ok_but_registers_nothing() {
        // Builtin parity: missing/empty topic warns and returns Ok.
        let h = handler();
        h.register_trigger(trigger_config("t1", serde_json::json!({})))
            .await
            .unwrap();
        h.register_trigger(trigger_config("t2", serde_json::json!({"topic": ""})))
            .await
            .unwrap();
        assert_eq!(h.hub.subscription_count().await, 0);
    }

    #[tokio::test]
    async fn unregister_unknown_id_is_ok() {
        let h = handler();
        h.unregister_trigger(trigger_config("ghost", serde_json::Value::Null))
            .await
            .unwrap();
    }
}
