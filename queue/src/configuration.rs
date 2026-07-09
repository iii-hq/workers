//! Configuration worker integration.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::adapter::SwappableAdapter;
use crate::boot::{self, ApplyLock, ConfigCell};
use crate::config::QueueConfig;
use crate::function_queues::FunctionQueueRuntime;
use crate::trigger::{IiiInvoker, Invoker, QueueTriggerHandler};

pub const CONFIG_ID: &str = "queue";
pub const CONFIG_FN_ID: &str = "queue::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

pub fn new_cell(config: QueueConfig) -> ConfigCell {
    Arc::new(tokio::sync::RwLock::new(Arc::new(config.normalized())))
}

pub async fn register_config(iii: &IIIClient, seed: Option<&QueueConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Queue",
        "description": "Durable queue worker settings: in-process/file-backed transport persistence.",
        "schema": QueueConfig::json_schema(),
    });
    if should_seed_initial_value(iii).await? {
        let seed = seed.cloned().unwrap_or_default().normalized();
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
            tracing::info!("no `{CONFIG_ID}` configuration value stored; using built-in default");
            Ok(QueueConfig::default())
        }
    }
}

pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    adapter: Arc<SwappableAdapter>,
    trigger_handler: QueueTriggerHandler,
    config: ConfigCell,
    apply_lock: ApplyLock,
    function_queues: Arc<FunctionQueueRuntime>,
) -> Result<(), Error> {
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let engine = engine.clone();
            let adapter = adapter.clone();
            let trigger_handler = trigger_handler.clone();
            let config = config.clone();
            let apply_lock = apply_lock.clone();
            let function_queues = function_queues.clone();
            async move {
                on_config_change(
                    engine,
                    adapter,
                    trigger_handler,
                    config,
                    apply_lock,
                    function_queues,
                )
                .await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload queue configuration from the authoritative store."),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
    })?;
    Ok(())
}

async fn on_config_change(
    iii: Arc<IIIClient>,
    adapter: Arc<SwappableAdapter>,
    trigger_handler: QueueTriggerHandler,
    config: ConfigCell,
    apply_lock: ApplyLock,
    function_queues: Arc<FunctionQueueRuntime>,
) {
    let _guard = apply_lock.lock().await;

    let next = match fetch_config(&iii).await {
        Ok(config) => config.normalized(),
        Err(err) => {
            tracing::error!(error = %err, "queue config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let old = config.read().await.clone();
    if swap_needed(&old, &next) {
        let invoker = Arc::new(IiiInvoker::new(iii.clone()));
        match swap_adapter(
            &adapter,
            &trigger_handler,
            &function_queues,
            invoker,
            &next,
        )
        .await
        {
            Ok(()) => {
                *config.write().await = Arc::new(next);
                tracing::info!("queue transport hot-swapped after configuration change");
            }
            Err(err) => {
                // Mirrors the engine's policy (`engine/src/workers/queue/queue.rs:1002-1019`
                // builds the replacement adapter first and only swaps in the
                // successful result): a failed hot-swap must never kill a
                // live worker, so the previous adapter and config are left
                // running untouched on error.
                tracing::error!(
                    error = %err,
                    "queue config-change: failed to build replacement adapter; keeping the \
                     previous adapter running"
                );
            }
        }
        return;
    }

    if let Err(err) = function_queues.reconcile(&next.queue_configs).await {
        tracing::error!(
            error = %err,
            "queue config-change: failed to apply named function queues; keeping previous config"
        );
        return;
    }
    *config.write().await = Arc::new(next);
    tracing::info!("queue configuration reloaded without transport swap");
}

/// Fallible core of a transport hot-swap. Builds the new adapter for `next`
/// via [`crate::boot::build_adapter`] *before* touching anything else: only
/// once that succeeds does this replace the live adapter and re-subscribe
/// every tracked trigger registration onto it. A `build_adapter` failure
/// returns `Err` with the old adapter, its subscriptions, and the config
/// cell completely untouched -- same policy as the engine's `apply_config`
/// (`engine/src/workers/queue/queue.rs:1002-1019`): never tear down a live
/// transport for a replacement that didn't pan out.
async fn swap_adapter(
    adapter: &Arc<SwappableAdapter>,
    trigger_handler: &QueueTriggerHandler,
    function_queues: &Arc<FunctionQueueRuntime>,
    invoker: Arc<dyn Invoker>,
    next: &QueueConfig,
) -> anyhow::Result<()> {
    let new_adapter = boot::build_adapter(next, invoker).await?;
    let old_adapter = adapter.current().await;
    adapter
        .replace(new_adapter, boot::adapter_identity_name(next))
        .await;
    function_queues.restart_all(&next.queue_configs).await?;
    trigger_handler.resubscribe_all().await;
    old_adapter.shutdown().await;
    Ok(())
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
        json!({ "id": CONFIG_ID }),
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
        ) -> anyhow::Result<()> {
            self.enqueue_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
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
        async fn call(
            &self,
            _function_id: &str,
            _payload: Value,
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) -> Result<Option<Value>, String> {
            Ok(None)
        }
    }

    fn noop_invoker() -> Arc<dyn Invoker> {
        Arc::new(NoopInvoker)
    }

    fn function_runtime(adapter: Arc<SwappableAdapter>) -> Arc<FunctionQueueRuntime> {
        Arc::new(FunctionQueueRuntime::new(adapter, noop_invoker()))
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
            ..Default::default()
        };

        let runtime = function_runtime(adapter.clone());
        let err = swap_adapter(
            &adapter,
            &trigger_handler,
            &runtime,
            noop_invoker(),
            &bad_config,
        )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not implemented"));

        // The old adapter must never be shut down, and it must still be the
        // one live behind the `SwappableAdapter`.
        assert_eq!(mock.shutdown_calls.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.current_name().await, "mock");
        adapter
            .enqueue("demo", Value::Null, None, None)
            .await
            .unwrap();
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
            ..Default::default()
        };

        let runtime = function_runtime(adapter.clone());
        swap_adapter(&adapter, &trigger_handler, &runtime, noop_invoker(), &next)
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
            ..Default::default()
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
            ..Default::default()
        };
        assert!(swap_needed(&old, &new));
    }
}
