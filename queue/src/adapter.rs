//! `QueueAdapter` trait — the transport abstraction every queue backend
//! (builtin in-process store, RabbitMQ, SQS, ...) implements.
//!
//! Ported from the engine builtin `iii-queue` worker
//! (`engine/src/workers/queue/mod.rs:40-160`). Seams applied for the
//! standalone worker: the engine trait is invoked from `Arc<Engine>`
//! call sites; here adapters are driven by the worker's own trigger loop
//! and invoke functions through `Arc<dyn crate::trigger::Invoker>` instead.
//! Telemetry/tracing spans present on the engine's trait methods are
//! dropped — the worker emits its own `tracing` events at call sites.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::store::TopicStats;
// Until Task 2 lands this is the existing 2-field struct in trigger.rs;
// Task 2 replaces it with the full port at crate::subscriber_config.
use crate::trigger::SubscriberQueueConfig;

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
/// Minimal port of the engine builtin's `FunctionQueueConfig`
/// (`engine/src/workers/queue/config.rs`) — only the fields the
/// function-queue adapter methods need to compile against. Call-site-less
/// until the engine `QueueEnqueuer` cut lands; a later task may replace
/// this with the full config-module port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionQueueConfig {
    /// Maximum delivery attempts before a message is sent to the
    /// dead-letter queue.
    pub max_retries: u32,
    /// Number of messages processed concurrently.
    pub concurrency: u32,
    /// Base delay in milliseconds for the exponential retry backoff.
    pub backoff_ms: u64,
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
    );

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
    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>>;

    /// Get stats for a specific topic.
    async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats>;

    /// Peek at dead-lettered messages on `topic` non-destructively, paginated
    /// by `offset`/`limit`.
    async fn dlq_peek(&self, topic: &str, offset: u64, limit: u64) -> anyhow::Result<Vec<Value>>;

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
    ) {
        unimplemented!("publish_to_function_queue not implemented for this adapter")
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
