//! Boot sequence: builtin guard → build adapter → hub → register the
//! `subscribe` trigger type and the `publish` service function.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::{self, Invoker};
use crate::config::PubSubConfig;
use crate::hub::Hub;
use crate::trigger::{SubscribeTriggerHandler, SubscribeTriggerSpec};
use crate::{PUBLISH_FUNCTION_ID, TRIGGER_TYPE};

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
const BUILTIN_III_PUBSUB_WORKER_ID: &str = "iii-pubsub";

pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;
pub type ConfigCell = Arc<tokio::sync::RwLock<PubSubConfig>>;

/// Input of the `publish` function — exact field parity with the builtin
/// (engine/src/workers/pubsub/pubsub.rs:78-84).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PubSubInput {
    /// Topic to publish to. Subscribers registered for this topic receive the event.
    pub topic: String,
    /// JSON payload delivered to each subscriber.
    pub data: Value,
}

pub struct BootHandle {
    pub hub: Arc<Hub>,
    pub invoker: Arc<dyn Invoker>,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    pub async fn shutdown(&self) {
        self.hub.shutdown().await;
    }
}

/// SDK-backed Invoker: engine.call parity via iii.trigger. Callers (the
/// adapters' fan-out) spawn and ignore the result, matching the builtin's
/// fire-and-forget `tokio::spawn(engine.call(..))`.
struct SdkInvoker {
    iii: Arc<IIIClient>,
}

#[async_trait::async_trait]
impl Invoker for SdkInvoker {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

pub async fn start(iii: Arc<IIIClient>, config: PubSubConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_pubsub(&iii).await?;

    let invoker: Arc<dyn Invoker> = Arc::new(SdkInvoker { iii: iii.clone() });
    let adapter = adapters::build_adapter(&config, invoker.clone()).await?;
    let hub = Arc::new(Hub::new(adapter));

    // The engine delivers every `subscribe` trigger binding through this
    // handler into the hub.
    let handler = SubscribeTriggerHandler::new(hub.clone());
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(TRIGGER_TYPE, "Subscribe to a topic", handler)
            .trigger_request_format::<SubscribeTriggerSpec>(),
    );

    // Service function: the bare id `publish` (see PUBLISH_FUNCTION_ID docs —
    // exact path parity with the builtin is load-bearing).
    let hub_for_publish = hub.clone();
    iii.register_function(
        PUBLISH_FUNCTION_ID,
        RegisterFunction::new_async(move |input: PubSubInput| {
            let hub = hub_for_publish.clone();
            async move {
                hub.publish(&input.topic, input.data)
                    .await
                    .map_err(Error::Handler)?;
                // Builtin returns Success(None) — a null result.
                Ok::<_, Error>(Value::Null)
            }
        })
        .description("Publishes an event"),
    );

    Ok(BootHandle {
        hub,
        invoker,
        config: Arc::new(tokio::sync::RwLock::new(config.normalized())),
        apply_lock: Arc::new(tokio::sync::Mutex::new(())),
    })
}

async fn guard_against_builtin_pubsub(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .trigger(TriggerRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_pubsub_active(&workers_list) {
        anyhow::bail!(
            "cannot start the pubsub worker: the built-in iii-pubsub worker is active and owns \
             the 'subscribe' trigger type and the 'publish' function. Remove iii-pubsub from the \
             engine config (a config.yaml that doesn't list it won't run it), then start this \
             worker."
        );
    }
    Ok(())
}

fn builtin_iii_pubsub_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|w| w.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_PUBSUB_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_PUBSUB_WORKER_ID)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_builtin_by_id() {
        let v = serde_json::json!({"workers": [{"id": "iii-pubsub"}]});
        assert!(builtin_iii_pubsub_active(&v));
    }

    #[test]
    fn detects_builtin_by_name() {
        let v = serde_json::json!({"workers": [{"id": "x", "name": "iii-pubsub"}]});
        assert!(builtin_iii_pubsub_active(&v));
    }

    #[test]
    fn absent_builtin_passes() {
        let v = serde_json::json!({"workers": [{"id": "iii-http"}]});
        assert!(!builtin_iii_pubsub_active(&v));
    }

    #[test]
    fn empty_list_passes() {
        assert!(!builtin_iii_pubsub_active(
            &serde_json::json!({"workers": []})
        ));
    }

    #[test]
    fn missing_key_passes() {
        assert!(!builtin_iii_pubsub_active(&serde_json::json!({})));
    }
}
