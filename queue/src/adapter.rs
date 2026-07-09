//! `QueueAdapter` trait — the transport abstraction every queue backend
//! (builtin in-process store, RabbitMQ, SQS, ...) implements.
//!
//! Ported from the engine builtin `iii-queue` worker
//! (`engine/src/workers/queue/mod.rs:40-160`). Seams applied for the
//! standalone worker: the engine trait is invoked from `Arc<Engine>`
//! call sites; here adapters are driven by the worker's own trigger loop
//! and invoke functions through `Arc<dyn crate::trigger::Invoker>` instead.
//! Trace headers travel through adapter jobs and are restored by the worker's
//! invoker; adapters also emit their own `tracing` events at call sites.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, RwLock};

use crate::store::TopicStats;
use crate::subscriber_config::SubscriberQueueConfig;

/// Summary of a known queue topic, returned by [`QueueAdapter::list_topics`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicInfo {
    pub name: String,
    pub depth: u64,
}

/// A message handed from a function-queue adapter to the consumer loop.
///
/// Ported from the engine builtin's `QueueMessage`
/// (`engine/src/workers/queue/message.rs`). Only used by the
/// function-queue transport path (`consume_function_queue` and friends),
/// which is call-site-less until the engine `QueueEnqueuer` cut lands.
#[derive(Debug, Clone)]
pub struct QueueMessage {
    /// Adapter-specific tag for ack/nack (e.g. RabbitMQ delivery tag).
    pub delivery_id: u64,
    /// The function to invoke.
    pub function_id: String,
    /// The payload to pass to the function.
    pub data: Value,
    /// Current attempt number (derived from transport-native retry count).
    pub attempt: u32,
    /// Publisher-assigned message ID for tracking and cancellation.
    pub message_id: Option<String>,
    /// W3C traceparent header for distributed tracing.
    pub traceparent: Option<String>,
    /// W3C baggage header for context propagation.
    pub baggage: Option<String>,
}

/// Per-function-queue transport settings, passed to
/// [`QueueAdapter::setup_function_queue`].
///
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionQueueConfig {
    /// Maximum delivery attempts before a message is sent to the
    /// dead-letter queue.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Number of messages processed concurrently.
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// Base delay in milliseconds for the exponential retry backoff.
    #[serde(default = "default_backoff_ms")]
    pub backoff_ms: u64,
    /// Delivery mode: `standard` or `fifo`.
    #[serde(default = "default_queue_type", rename = "type")]
    pub r#type: String,
    /// For FIFO queues, the payload field whose value identifies the ordering
    /// group. Required when `type` is `fifo`.
    #[serde(default)]
    pub message_group_field: Option<String>,
    /// Declares the queue as a RabbitMQ priority queue with this many
    /// priority levels (`x-max-priority`). `None` means not a priority
    /// queue. RabbitMQ-only; other adapters ignore it. Added (rather than
    /// part of the original minimal port) so
    /// [`QueueAdapter::setup_function_queue`] can pass it through to the
    /// RabbitMQ adapter's topology setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_priority: Option<u8>,
    /// Payload field whose integer value supplies the RabbitMQ priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_field: Option<String>,
    /// Poll interval for polling adapters such as the builtin store.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_max_retries() -> u32 {
    3
}

fn default_concurrency() -> u32 {
    10
}

fn default_backoff_ms() -> u64 {
    1_000
}

fn default_queue_type() -> String {
    "standard".to_string()
}

fn default_poll_interval_ms() -> u64 {
    100
}

impl Default for FunctionQueueConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            concurrency: default_concurrency(),
            backoff_ms: default_backoff_ms(),
            r#type: default_queue_type(),
            message_group_field: None,
            max_priority: None,
            priority_field: None,
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

impl FunctionQueueConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.concurrency == 0 {
            anyhow::bail!("function queue concurrency must be at least 1");
        }
        if self.r#type != "standard" && self.r#type != "fifo" {
            anyhow::bail!(
                "function queue type '{}' is not supported; expected 'standard' or 'fifo'",
                self.r#type
            );
        }
        if self.r#type == "fifo"
            && self
                .message_group_field
                .as_deref()
                .is_none_or(|field| field.trim().is_empty())
        {
            anyhow::bail!("FIFO function queues require a non-empty 'message_group_field'");
        }
        if self.max_priority == Some(0) {
            anyhow::bail!("function queue max_priority must be at least 1");
        }
        Ok(())
    }

    /// Validate payload fields used by FIFO ordering and priority delivery,
    /// returning the clamped RabbitMQ priority when configured.
    pub fn validate_payload(&self, data: &Value) -> anyhow::Result<Option<u8>> {
        self.validate()?;
        if self.r#type == "fifo" {
            let field = self
                .message_group_field
                .as_deref()
                .expect("FIFO config validation requires a group field");
            let value = data.get(field).ok_or_else(|| {
                anyhow::anyhow!("FIFO function queue requires field '{field}' in data")
            })?;
            if value.is_null() {
                anyhow::bail!("FIFO function queue field '{field}' must not be null");
            }
        }

        let Some(field) = self.priority_field.as_deref() else {
            return Ok(None);
        };
        let Some(raw) = data.get(field).and_then(Value::as_u64) else {
            return Ok(None);
        };
        let maximum = self.max_priority.unwrap_or(u8::MAX) as u64;
        Ok(Some(raw.min(maximum) as u8))
    }
}

/// Transport abstraction implemented by every queue backend adapter
/// (builtin in-process store, RabbitMQ, SQS, ...).
///
/// Ported from the engine builtin's `QueueAdapter`
/// (`engine/src/workers/queue/mod.rs:52-159`).
#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait QueueAdapter: Send + Sync + 'static {
    /// Enqueue a message onto `topic`.
    async fn enqueue(
        &self,
        topic: &str,
        data: Value,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) -> anyhow::Result<()>;

    /// Register a subscriber (`id`) on `topic` that invokes `function_id`
    /// for each delivered message.
    async fn subscribe(
        &self,
        topic: &str,
        id: &str,
        function_id: &str,
        condition_function_id: Option<String>,
        queue_config: Option<SubscriberQueueConfig>,
    );

    /// Remove a previously registered subscriber.
    async fn unsubscribe(&self, topic: &str, id: &str);

    /// Redrive every dead-lettered message on `topic` back onto the live
    /// queue. Returns the number of messages redriven.
    async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64>;

    /// Redrive a single dead-lettered message by ID. Returns `true` if the
    /// message was found and redriven.
    async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool>;

    /// Discard a single dead-lettered message by ID. Returns `true` if the
    /// message was found and discarded.
    async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool>;

    /// Count of messages currently in `topic`'s dead-letter queue.
    async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64>;

    /// Peek at up to `count` dead-lettered messages on `topic`,
    /// non-destructively. Default implementation delegates to
    /// [`Self::dlq_peek`] starting at offset 0.
    async fn dlq_messages(&self, topic: &str, count: usize) -> anyhow::Result<Vec<Value>> {
        self.dlq_peek(topic, 0, count as u64).await
    }

    /// List all known queue topics.
    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        Ok(vec![])
    }

    /// Get stats for a specific topic.
    async fn topic_stats(&self, _topic: &str) -> anyhow::Result<TopicStats> {
        Ok(TopicStats::default())
    }

    /// Peek at dead-lettered messages on `topic` non-destructively, paginated
    /// by `offset`/`limit`.
    async fn dlq_peek(
        &self,
        _topic: &str,
        _offset: u64,
        _limit: u64,
    ) -> anyhow::Result<Vec<Value>> {
        Ok(vec![])
    }

    /// Gracefully tear down this adapter (stop consumers, close
    /// connections). The worker calls this deterministically on shutdown;
    /// the engine builtin instead aborts consumer tasks in place.
    async fn shutdown(&self);

    // -- Function queue transport methods --
    //
    // Call-site-less until the engine `QueueEnqueuer` cut lands (see
    // `engine/src/workers/queue/mod.rs:78-158`). Ported here so adapters
    // implementing this trait have the same surface as the engine builtin;
    // default bodies match the engine's no-op/unimplemented defaults.

    /// Publish a message directly onto a function's dedicated queue.
    #[allow(clippy::too_many_arguments)]
    async fn publish_to_function_queue(
        &self,
        _queue_name: &str,
        _function_id: &str,
        _data: Value,
        _message_id: &str,
        _max_retries: u32,
        _backoff_ms: u64,
        _traceparent: Option<String>,
        _baggage: Option<String>,
        // AMQP message priority (`None` = default). Honored by adapters whose
        // queues are declared as priority queues; others ignore it.
        _priority: Option<u8>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("function queues are not supported by this adapter")
    }

    /// Set up transport topology for a function queue (exchanges, queues,
    /// bindings). Called once per queue during initialization.
    async fn setup_function_queue(
        &self,
        _queue_name: &str,
        _config: &FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        Ok(()) // no-op by default (schemaless stores like builtin)
    }

    /// Start consuming from a function queue. Returns a receiver that yields
    /// `QueueMessage`s.
    async fn consume_function_queue(
        &self,
        _queue_name: &str,
        _prefetch: u32,
    ) -> anyhow::Result<mpsc::Receiver<QueueMessage>> {
        unimplemented!("consume_function_queue not implemented for this adapter")
    }

    /// Acknowledge successful processing of a message.
    async fn ack_function_queue(&self, _queue_name: &str, _delivery_id: u64) -> anyhow::Result<()> {
        Ok(()) // no-op by default
    }

    /// Negative-acknowledge a message. When `attempt < max_retries`, the
    /// adapter should route to retry. When `attempt >= max_retries`, the
    /// adapter should route to the DLQ.
    async fn nack_function_queue(
        &self,
        _queue_name: &str,
        _delivery_id: u64,
        _attempt: u32,
        _max_retries: u32,
    ) -> anyhow::Result<()> {
        Ok(()) // no-op by default
    }
}

/// Hot-swappable [`QueueAdapter`]: holds the currently active adapter behind
/// an `RwLock` and forwards every trait method to it. A configuration change
/// replaces the inner adapter (e.g. builtin -> file-backed builtin) without
/// the trigger handler or service functions needing to know a swap happened.
#[derive(Clone)]
pub struct SwappableAdapter {
    inner: Arc<RwLock<Arc<dyn QueueAdapter>>>,
    /// The active adapter's identity (e.g. `"builtin"`, `"redis"`,
    /// `"rabbitmq"`), set by the factory (`crate::boot::build_adapter`'s
    /// caller) at construction/replace time. Exposed via [`Self::current_name`]
    /// for service functions (`list_topics`) that report which transport
    /// backs a topic.
    name: Arc<RwLock<String>>,
}

impl SwappableAdapter {
    pub fn new(adapter: Arc<dyn QueueAdapter>, name: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(adapter)),
            name: Arc::new(RwLock::new(name.into())),
        }
    }

    pub async fn current(&self) -> Arc<dyn QueueAdapter> {
        self.inner.read().await.clone()
    }

    pub async fn current_name(&self) -> String {
        self.name.read().await.clone()
    }

    pub async fn replace(&self, adapter: Arc<dyn QueueAdapter>, name: impl Into<String>) {
        *self.inner.write().await = adapter;
        *self.name.write().await = name.into();
    }
}

#[async_trait]
impl QueueAdapter for SwappableAdapter {
    async fn enqueue(
        &self,
        topic: &str,
        data: Value,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) -> anyhow::Result<()> {
        self.current()
            .await
            .enqueue(topic, data, traceparent, baggage)
            .await
    }

    async fn subscribe(
        &self,
        topic: &str,
        id: &str,
        function_id: &str,
        condition_function_id: Option<String>,
        queue_config: Option<SubscriberQueueConfig>,
    ) {
        self.current()
            .await
            .subscribe(topic, id, function_id, condition_function_id, queue_config)
            .await;
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        self.current().await.unsubscribe(topic, id).await;
    }

    async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64> {
        self.current().await.redrive_dlq(topic).await
    }

    async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        self.current()
            .await
            .redrive_dlq_message(topic, message_id)
            .await
    }

    async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        self.current()
            .await
            .discard_dlq_message(topic, message_id)
            .await
    }

    async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64> {
        self.current().await.dlq_count(topic).await
    }

    async fn dlq_messages(&self, topic: &str, count: usize) -> anyhow::Result<Vec<Value>> {
        self.current().await.dlq_messages(topic, count).await
    }

    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        self.current().await.list_topics().await
    }

    async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats> {
        self.current().await.topic_stats(topic).await
    }

    async fn dlq_peek(&self, topic: &str, offset: u64, limit: u64) -> anyhow::Result<Vec<Value>> {
        self.current().await.dlq_peek(topic, offset, limit).await
    }

    async fn shutdown(&self) {
        self.current().await.shutdown().await;
    }

    async fn publish_to_function_queue(
        &self,
        queue_name: &str,
        function_id: &str,
        data: Value,
        message_id: &str,
        max_retries: u32,
        backoff_ms: u64,
        traceparent: Option<String>,
        baggage: Option<String>,
        priority: Option<u8>,
    ) -> anyhow::Result<()> {
        self.current()
            .await
            .publish_to_function_queue(
                queue_name,
                function_id,
                data,
                message_id,
                max_retries,
                backoff_ms,
                traceparent,
                baggage,
                priority,
            )
            .await
    }

    async fn setup_function_queue(
        &self,
        queue_name: &str,
        config: &FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        self.current()
            .await
            .setup_function_queue(queue_name, config)
            .await
    }

    async fn consume_function_queue(
        &self,
        queue_name: &str,
        prefetch: u32,
    ) -> anyhow::Result<mpsc::Receiver<QueueMessage>> {
        self.current()
            .await
            .consume_function_queue(queue_name, prefetch)
            .await
    }

    async fn ack_function_queue(&self, queue_name: &str, delivery_id: u64) -> anyhow::Result<()> {
        self.current()
            .await
            .ack_function_queue(queue_name, delivery_id)
            .await
    }

    async fn nack_function_queue(
        &self,
        queue_name: &str,
        delivery_id: u64,
        attempt: u32,
        max_retries: u32,
    ) -> anyhow::Result<()> {
        self.current()
            .await
            .nack_function_queue(queue_name, delivery_id, attempt, max_retries)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Default)]
    struct CountingAdapter {
        enqueue_calls: AtomicU64,
    }

    #[async_trait]
    impl QueueAdapter for CountingAdapter {
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
            _queue_config: Option<SubscriberQueueConfig>,
        ) {
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

        async fn shutdown(&self) {}
    }

    #[tokio::test]
    async fn current_reflects_replace() {
        let first = Arc::new(CountingAdapter::default());
        let swappable = SwappableAdapter::new(first.clone(), "first");
        swappable
            .enqueue("demo", Value::Null, None, None)
            .await
            .unwrap();
        assert_eq!(first.enqueue_calls.load(Ordering::SeqCst), 1);

        let second = Arc::new(CountingAdapter::default());
        swappable.replace(second.clone(), "second").await;
        swappable
            .enqueue("demo", Value::Null, None, None)
            .await
            .unwrap();

        assert_eq!(first.enqueue_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.enqueue_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn current_name_reflects_replace() {
        let first = Arc::new(CountingAdapter::default());
        let swappable = SwappableAdapter::new(first, "builtin");
        assert_eq!(swappable.current_name().await, "builtin");

        let second = Arc::new(CountingAdapter::default());
        swappable.replace(second, "redis").await;
        assert_eq!(swappable.current_name().await, "redis");
    }

    #[test]
    fn function_queue_config_parses_the_full_engine_shape() {
        let config: FunctionQueueConfig = serde_json::from_value(serde_json::json!({
            "type": "fifo",
            "concurrency": 4,
            "max_retries": 7,
            "backoff_ms": 250,
            "poll_interval_ms": 25,
            "message_group_field": "session_id",
            "max_priority": 10,
            "priority_field": "priority"
        }))
        .unwrap();

        assert_eq!(config.r#type, "fifo");
        assert_eq!(config.concurrency, 4);
        assert_eq!(config.max_retries, 7);
        assert_eq!(config.backoff_ms, 250);
        assert_eq!(config.poll_interval_ms, 25);
        assert_eq!(config.message_group_field.as_deref(), Some("session_id"));
        assert_eq!(config.max_priority, Some(10));
        assert_eq!(config.priority_field.as_deref(), Some("priority"));
    }

    #[test]
    fn function_queue_config_validation_matches_engine_constraints() {
        let zero_concurrency = FunctionQueueConfig {
            concurrency: 0,
            ..Default::default()
        };
        assert!(zero_concurrency.validate().is_err());

        let unknown_type = FunctionQueueConfig {
            r#type: "priority".to_string(),
            ..Default::default()
        };
        assert!(unknown_type.validate().is_err());

        let fifo_without_group = FunctionQueueConfig {
            r#type: "fifo".to_string(),
            ..Default::default()
        };
        assert!(fifo_without_group.validate().is_err());

        let empty_group = FunctionQueueConfig {
            r#type: "fifo".to_string(),
            message_group_field: Some("  ".to_string()),
            ..Default::default()
        };
        assert!(empty_group.validate().is_err());

        let zero_priority = FunctionQueueConfig {
            max_priority: Some(0),
            ..Default::default()
        };
        assert!(zero_priority.validate().is_err());
    }

    #[test]
    fn function_queue_config_schema_is_closed_and_complete() {
        let schema = schemars::schema_for!(FunctionQueueConfig);
        let properties = schema
            .schema
            .object
            .as_ref()
            .expect("config schema should be an object")
            .properties
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            properties,
            [
                "backoff_ms",
                "concurrency",
                "max_priority",
                "max_retries",
                "message_group_field",
                "poll_interval_ms",
                "priority_field",
                "type"
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
    }

    #[test]
    fn function_queue_payload_validation_extracts_group_and_priority() {
        let config = FunctionQueueConfig {
            r#type: "fifo".to_string(),
            message_group_field: Some("session_id".to_string()),
            max_priority: Some(10),
            priority_field: Some("priority".to_string()),
            ..Default::default()
        };
        assert_eq!(
            config
                .validate_payload(&serde_json::json!({
                    "session_id": "s1",
                    "priority": 99
                }))
                .unwrap(),
            Some(10)
        );
    }

    #[test]
    fn function_queue_payload_validation_rejects_missing_fifo_group() {
        let config = FunctionQueueConfig {
            r#type: "fifo".to_string(),
            message_group_field: Some("session_id".to_string()),
            ..Default::default()
        };
        let error = config
            .validate_payload(&serde_json::json!({"priority": 1}))
            .expect_err("missing FIFO group should be rejected");
        assert!(error.to_string().contains("session_id"));
    }
}
