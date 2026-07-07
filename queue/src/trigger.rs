//! Durable subscriber trigger handling.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::store::QueueStore;
use crate::subscriber_config::SubscriberQueueConfig;

pub const DEFAULT_MAX_RETRIES: u32 = 3;
pub const DEFAULT_BACKOFF_MS: u64 = 1000;
const IDLE_POLL_MS: u64 = 50;

#[async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String>;
}

#[derive(Clone)]
pub struct IiiInvoker {
    iii: Arc<IIIClient>,
}

impl IiiInvoker {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl Invoker for IiiInvoker {
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscriberSpec {
    /// Queue/topic name to consume.
    #[serde(alias = "topic")]
    pub queue: String,
    #[serde(default, alias = "maxRetries", skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(
        default,
        alias = "backoffDelayMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub backoff_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_function_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_config: Option<SubscriberQueueConfig>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSubscriber {
    pub trigger_id: String,
    pub function_id: String,
    pub spec: SubscriberSpec,
}

struct Consumer {
    registration: RegisteredSubscriber,
    handle: JoinHandle<()>,
}

#[derive(Clone)]
pub struct QueueTriggerHandler {
    store: Arc<dyn QueueStore>,
    invoker: Arc<dyn Invoker>,
    consumers: Arc<Mutex<HashMap<String, Consumer>>>,
}

impl QueueTriggerHandler {
    pub fn new(store: Arc<dyn QueueStore>, invoker: Arc<dyn Invoker>) -> Self {
        Self {
            store,
            invoker,
            consumers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn registrations(&self) -> Vec<RegisteredSubscriber> {
        self.consumers
            .lock()
            .await
            .values()
            .map(|consumer| consumer.registration.clone())
            .collect()
    }

    pub async fn shutdown(&self) {
        let consumers = self.consumers.lock().await.drain().collect::<Vec<_>>();
        for (_, consumer) in consumers {
            consumer.handle.abort();
        }
    }

    pub async fn register_subscriber(
        &self,
        registration: RegisteredSubscriber,
    ) -> Result<(), String> {
        if registration.spec.queue.trim().is_empty() {
            return Err("queue is required for durable:subscriber trigger".to_string());
        }

        let mut consumers = self.consumers.lock().await;
        if consumers.contains_key(&registration.trigger_id) {
            return Err(format!(
                "durable:subscriber trigger '{}' is already registered",
                registration.trigger_id
            ));
        }

        let handle = spawn_consumer_loop(
            self.store.clone(),
            self.invoker.clone(),
            registration.clone(),
        );
        consumers.insert(
            registration.trigger_id.clone(),
            Consumer {
                registration,
                handle,
            },
        );
        Ok(())
    }
}

#[async_trait]
impl TriggerHandler for QueueTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let spec: SubscriberSpec = serde_json::from_value(config.config.clone())
            .map_err(|e| Error::Handler(format!("invalid durable:subscriber config: {e}")))?;
        self.register_subscriber(RegisteredSubscriber {
            trigger_id: config.id,
            function_id: config.function_id,
            spec,
        })
        .await
        .map_err(Error::Handler)
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        if let Some(consumer) = self.consumers.lock().await.remove(&config.id) {
            consumer.handle.abort();
        }
        Ok(())
    }
}

fn spawn_consumer_loop(
    store: Arc<dyn QueueStore>,
    invoker: Arc<dyn Invoker>,
    registration: RegisteredSubscriber,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let queue = registration.spec.queue.clone();
        let max_retries = effective_max_retries(&registration.spec);
        let backoff_ms = effective_backoff_ms(&registration.spec);

        loop {
            let Some(job) = store.dequeue(&queue).await else {
                tokio::time::sleep(Duration::from_millis(IDLE_POLL_MS)).await;
                continue;
            };

            if let Some(condition_id) = registration.spec.condition_function_id.as_deref() {
                match invoker.call(condition_id, job.payload.clone()).await {
                    Ok(Some(Value::Bool(false))) => {
                        store.ack(&queue, &job.id).await;
                        continue;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        store.nack(&queue, job, max_retries, backoff_ms).await;
                        continue;
                    }
                }
            }

            match invoker
                .call(&registration.function_id, job.payload.clone())
                .await
            {
                Ok(_) => store.ack(&queue, &job.id).await,
                Err(_) => store.nack(&queue, job, max_retries, backoff_ms).await,
            }
        }
    })
}

fn effective_max_retries(spec: &SubscriberSpec) -> u32 {
    spec.max_retries
        .or_else(|| spec.queue_config.as_ref().and_then(|c| c.max_retries))
        .unwrap_or(DEFAULT_MAX_RETRIES)
}

fn effective_backoff_ms(spec: &SubscriberSpec) -> u64 {
    spec.backoff_ms
        .or_else(|| spec.queue_config.as_ref().and_then(|c| c.backoff_delay_ms))
        .unwrap_or(DEFAULT_BACKOFF_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{InMemoryStore, QueueStore};
    use serde_json::json;
    use std::future::Future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::time::{sleep, Duration, Instant};

    #[derive(Default)]
    struct FakeInvoker {
        calls: Mutex<Vec<(String, Value)>>,
        fail_backend: AtomicBool,
        condition_value: Mutex<Option<Value>>,
    }

    #[async_trait]
    impl Invoker for FakeInvoker {
        async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
            self.calls
                .lock()
                .await
                .push((function_id.to_string(), payload));
            if function_id == "condition" {
                return Ok(self.condition_value.lock().await.clone());
            }
            if self.fail_backend.load(Ordering::SeqCst) {
                Err("backend failed".to_string())
            } else {
                Ok(Some(json!({"ok": true})))
            }
        }
    }

    fn trigger_config(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
        }
    }

    async fn wait_until<F, Fut>(mut pred: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if pred().await {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert!(pred().await, "condition did not become true before timeout");
    }

    #[tokio::test]
    async fn register_spawns_loop_and_acks_on_success() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let handler = QueueTriggerHandler::new(store.clone(), invoker.clone());
        store
            .enqueue("demo", json!({"hello": "world"}))
            .await
            .unwrap();

        handler
            .register_trigger(trigger_config(
                "t1",
                "backend",
                json!({"queue": "demo", "max_retries": 2, "backoff_ms": 1}),
            ))
            .await
            .unwrap();

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.calls.lock().await.len() == 1 }
        })
        .await;
        assert_eq!(store.topic_stats("demo").await.delivered, 1);
        assert!(store.dequeue("demo").await.is_none());
        handler.shutdown().await;
    }

    #[tokio::test]
    async fn failed_invocation_nacks_until_dlq() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        invoker.fail_backend.store(true, Ordering::SeqCst);
        let handler = QueueTriggerHandler::new(store.clone(), invoker.clone());
        store
            .enqueue("demo", json!({"hello": "world"}))
            .await
            .unwrap();

        handler
            .register_trigger(trigger_config(
                "t1",
                "backend",
                json!({"queue": "demo", "max_retries": 1, "backoff_ms": 1}),
            ))
            .await
            .unwrap();

        wait_until(|| {
            let invoker = invoker.clone();
            async move { !invoker.calls.lock().await.is_empty() }
        })
        .await;
        wait_until(|| {
            let store = store.clone();
            async move { !store.dlq_messages("demo", 10).await.is_empty() }
        })
        .await;
        assert_eq!(store.topic_stats("demo").await.failed, 1);
        handler.shutdown().await;
    }

    #[tokio::test]
    async fn register_rejects_empty_queue() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let handler = QueueTriggerHandler::new(store, invoker);
        let err = handler
            .register_trigger(trigger_config("t1", "backend", json!({"queue": ""})))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("queue"));
    }

    #[tokio::test]
    async fn duplicate_trigger_id_is_rejected() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let handler = QueueTriggerHandler::new(store, invoker);
        let cfg = trigger_config("t1", "backend", json!({"queue": "demo"}));
        handler.register_trigger(cfg.clone()).await.unwrap();
        let err = handler.register_trigger(cfg).await.unwrap_err();
        assert!(err.to_string().contains("already registered"));
        handler.shutdown().await;
    }

    #[tokio::test]
    async fn unregister_by_id_stops_consumption() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        let handler = QueueTriggerHandler::new(store.clone(), invoker.clone());
        handler
            .register_trigger(trigger_config("t1", "backend", json!({"queue": "demo"})))
            .await
            .unwrap();
        handler
            .unregister_trigger(trigger_config("t1", "", Value::Null))
            .await
            .unwrap();
        store
            .enqueue("demo", json!({"after": "stop"}))
            .await
            .unwrap();
        sleep(Duration::from_millis(120)).await;
        assert!(invoker.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn explicit_false_condition_skips_and_acks() {
        let store: Arc<dyn QueueStore> = Arc::new(InMemoryStore::new());
        let invoker = Arc::new(FakeInvoker::default());
        *invoker.condition_value.lock().await = Some(Value::Bool(false));
        let handler = QueueTriggerHandler::new(store.clone(), invoker.clone());
        store
            .enqueue("demo", json!({"hello": "world"}))
            .await
            .unwrap();

        handler
            .register_trigger(trigger_config(
                "t1",
                "backend",
                json!({"queue": "demo", "condition_function_id": "condition"}),
            ))
            .await
            .unwrap();

        wait_until(|| {
            let invoker = invoker.clone();
            async move { invoker.calls.lock().await.len() == 1 }
        })
        .await;
        assert_eq!(invoker.calls.lock().await[0].0, "condition");
        assert_eq!(store.topic_stats("demo").await.delivered, 1);
        assert!(store.dequeue("demo").await.is_none());
        handler.shutdown().await;
    }

    #[test]
    fn supports_builtin_topic_and_queue_config_aliases() {
        let spec: SubscriberSpec = serde_json::from_value(json!({
            "topic": "demo",
            "queue_config": {
                "maxRetries": 9,
                "backoffDelayMs": 25
            }
        }))
        .unwrap();
        assert_eq!(spec.queue, "demo");
        assert_eq!(effective_max_retries(&spec), 9);
        assert_eq!(effective_backoff_ms(&spec), 25);
    }
}
