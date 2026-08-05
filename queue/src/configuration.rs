//! Configuration worker integration.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::boot::ConfigCell;
use crate::config::QueueConfig;
use crate::runtime::FunctionQueueRuntime;
use crate::trigger::QueueTriggerHandler;
#[cfg(test)]
use crate::{adapter::SwappableAdapter, boot, trigger::Invoker};

pub const DEFAULT_CONFIG_ID: &str = "queue";

/// The configuration entry this worker owns.
///
/// `III_CONFIG_NAME` when a supervisor set it, else the built-in name. A worker
/// that hardcodes its id turns that id into a global scarce name: two instances
/// share one entry and take turns overwriting it, and each write wakes both.
/// Being told which entry is its own is what lets them differ.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}
pub const CONFIG_FN_ID: &str = "queue::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

pub fn new_cell(config: QueueConfig) -> ConfigCell {
    Arc::new(tokio::sync::RwLock::new(Arc::new(config.normalized())))
}

pub async fn register_config(iii: &IIIClient, seed: Option<&QueueConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Queue",
        "description": "Durable queue worker settings: transport persistence and named function queues.",
        "schema": QueueConfig::json_schema(),
    });
    if should_seed_initial_value(iii).await? {
        let seed = seed
            .cloned()
            .unwrap_or_else(QueueConfig::packaged_default)
            .normalized();
        payload["initial_value"] = seed.to_json();
    }
    trigger_with_retry(
        iii,
        "configuration::register",
        payload,
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<QueueConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => QueueConfig::from_json(&value).map(|c| c.normalized()),
        _ => {
            tracing::info!(
                "no `{config_entry}` configuration value stored; using durable packaged default",
                config_entry = config_id()
            );
            Ok(QueueConfig::packaged_default())
        }
    }
}

/// Replace the authoritative queue configuration. `configuration::set`
/// replaces the whole value, so callers must merge against their current
/// snapshot before invoking this helper.
pub async fn persist_config(iii: &IIIClient, config: &QueueConfig) -> Result<(), String> {
    config.validate()?;
    trigger_with_retry(
        iii,
        "configuration::set",
        json!({ "id": config_id(), "value": config.to_json() }),
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await?;
    Ok(())
}

pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    runtime: FunctionQueueRuntime,
    trigger_handler: QueueTriggerHandler,
) -> Result<(), Error> {
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let runtime = runtime.clone();
            let trigger_handler = trigger_handler.clone();
            async move {
                on_config_change(runtime, trigger_handler).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload queue configuration from the authoritative store."),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
        namespace: iii.namespace(),
    })?;
    Ok(())
}

async fn on_config_change(runtime: FunctionQueueRuntime, trigger_handler: QueueTriggerHandler) {
    if let Err(err) = runtime.refresh_config(&trigger_handler).await {
        tracing::error!(error = %err, "queue config-change: apply failed; keeping previous live configuration");
    }
}

pub fn swap_needed(old: &QueueConfig, new: &QueueConfig) -> bool {
    effective_adapter(old) != effective_adapter(new)
}

fn effective_adapter(config: &QueueConfig) -> (String, Value) {
    (
        config.effective_adapter_name().to_string(),
        config
            .adapter
            .as_ref()
            .and_then(|adapter| adapter.config.clone())
            .unwrap_or(Value::Null),
    )
}

#[cfg(test)]
async fn swap_adapter(
    adapter: &Arc<SwappableAdapter>,
    trigger_handler: &QueueTriggerHandler,
    invoker: Arc<dyn Invoker>,
    next: &QueueConfig,
) -> anyhow::Result<()> {
    let new_adapter = boot::build_adapter(next, invoker).await?;
    let old_adapter = adapter.current().await;
    adapter
        .replace(new_adapter, boot::adapter_identity_name(next))
        .await;
    old_adapter.shutdown().await;
    trigger_handler.resubscribe_all().await;
    Ok(())
}

async fn should_seed_initial_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => Ok(false),
        _ => Ok(true),
    }
}

async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(
        iii,
        "configuration::get",
        json!({ "id": config_id() }),
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
        {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_err = err.to_string();
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * u64::from(attempt),
                    ))
                    .await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeAck {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeRequest {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{QueueAdapter, TopicInfo};
    use crate::config::AdapterEntry;
    use crate::store::TopicStats;
    use crate::trigger::{RegisteredSubscriber, SubscriberSpec};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    #[derive(Default)]
    struct MockAdapter {
        subscribe_calls: AtomicUsize,
        enqueue_calls: AtomicUsize,
        shutdown_calls: AtomicU64,
    }

    #[async_trait]
    impl QueueAdapter for MockAdapter {
        async fn enqueue(
            &self,
            _topic: &str,
            _data: Value,
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) {
            self.enqueue_calls.fetch_add(1, Ordering::SeqCst);
        }

        async fn subscribe(
            &self,
            _topic: &str,
            _id: &str,
            _function_id: &str,
            _condition_function_id: Option<String>,
            _queue_config: Option<crate::subscriber_config::SubscriberQueueConfig>,
        ) {
            self.subscribe_calls.fetch_add(1, Ordering::SeqCst);
        }

        async fn unsubscribe(&self, _topic: &str, _id: &str) {}

        async fn redrive_dlq(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn redrive_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn discard_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn dlq_count(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
            Ok(vec![])
        }

        async fn topic_stats(&self, _topic: &str) -> anyhow::Result<TopicStats> {
            Ok(TopicStats::default())
        }

        async fn shutdown(&self) {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct NoopInvoker;

    #[async_trait]
    impl Invoker for NoopInvoker {
        async fn call(&self, _function_id: &str, _payload: Value) -> Result<Option<Value>, String> {
            Ok(None)
        }
    }

    fn noop_invoker() -> Arc<dyn Invoker> {
        Arc::new(NoopInvoker)
    }

    #[tokio::test]
    async fn swap_adapter_keeps_old_adapter_running_when_build_fails() {
        let mock = Arc::new(MockAdapter::default());
        let dyn_adapter: Arc<dyn QueueAdapter> = mock.clone();
        let adapter = Arc::new(SwappableAdapter::new(dyn_adapter, "mock"));
        let trigger_handler = QueueTriggerHandler::new(adapter.clone());
        trigger_handler
            .register_subscriber(RegisteredSubscriber {
                trigger_id: "t1".to_string(),
                function_id: "backend".to_string(),
                spec: SubscriberSpec {
                    queue: "demo".to_string(),
                    max_retries: None,
                    backoff_ms: None,
                    condition_function_id: None,
                    queue_config: None,
                },
            })
            .await
            .unwrap();
        assert_eq!(mock.subscribe_calls.load(Ordering::SeqCst), 1);

        let bad_config = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "not-a-real-adapter".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };

        let err = swap_adapter(&adapter, &trigger_handler, noop_invoker(), &bad_config)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not implemented"));

        // The old adapter must never be shut down, and it must still be the
        // one live behind the `SwappableAdapter`.
        assert_eq!(mock.shutdown_calls.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.current_name().await, "mock");
        adapter.enqueue("demo", Value::Null, None, None).await;
        assert_eq!(mock.enqueue_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn swap_adapter_replaces_transport_and_resubscribes_on_success() {
        let old_mock = Arc::new(MockAdapter::default());
        let old_dyn: Arc<dyn QueueAdapter> = old_mock.clone();
        let adapter = Arc::new(SwappableAdapter::new(old_dyn, "mock"));
        let trigger_handler = QueueTriggerHandler::new(adapter.clone());
        trigger_handler
            .register_subscriber(RegisteredSubscriber {
                trigger_id: "t1".to_string(),
                function_id: "backend".to_string(),
                spec: SubscriberSpec {
                    queue: "demo".to_string(),
                    max_retries: None,
                    backoff_ms: None,
                    condition_function_id: None,
                    queue_config: None,
                },
            })
            .await
            .unwrap();
        assert_eq!(old_mock.subscribe_calls.load(Ordering::SeqCst), 1);

        // A real (dependency-free) config change: builtin -> in-memory,
        // which `build_adapter` resolves successfully.
        let next = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "in_memory".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };

        swap_adapter(&adapter, &trigger_handler, noop_invoker(), &next)
            .await
            .unwrap();

        assert_eq!(
            old_mock.shutdown_calls.load(Ordering::SeqCst),
            1,
            "old adapter must be shut down once the new one is live"
        );
        assert_eq!(adapter.current_name().await, "builtin");
        // The registration is still tracked and was re-subscribed onto the
        // new adapter (a builtin adapter has no externally observable
        // subscribe counter, so this is asserted end-to-end in
        // `trigger::tests::resubscribe_all_attaches_every_registration_to_the_new_adapter`
        // -- here we only assert the registration survived the swap).
        assert_eq!(trigger_handler.registrations().await.len(), 1);
    }

    #[test]
    fn swap_needed_ignores_implicit_builtin_default() {
        let old = QueueConfig::default();
        let new = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "builtin".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        assert!(!swap_needed(&old, &new));
    }

    #[test]
    fn swap_needed_when_adapter_config_changes() {
        let old = QueueConfig::default();
        let new = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "builtin".to_string(),
                config: Some(json!({
                    "store_method": "file_based",
                    "file_path": "./data/queue"
                })),
            }),
            ..QueueConfig::default()
        };
        assert!(swap_needed(&old, &new));
    }
}
