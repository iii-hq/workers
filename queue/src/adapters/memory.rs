//! In-process test-only transport, registered under the adapter name
//! `"memory"`.
//!
//! Ported from the engine builtin's `memory_adapter`
//! (`engine/src/workers/queue/adapters/memory_adapter.rs`): the engine's
//! version is mechanically just `builtin::make_adapter` re-registered under
//! a second name so the config hot-swap test suite can swap to a *second*
//! dependency-free backend (the real alternatives, `redis`/`rabbitmq`, need a
//! live server). This port keeps that spirit as a distinct type — rather
//! than re-registering the same constructor under two names (this crate's
//! factory dispatches on a `match` arm per adapter, not a name registry) —
//! wrapping a fresh [`BuiltinAdapter`] over its own [`InMemoryStore`] and
//! delegating every [`QueueAdapter`] method straight through. Only compiled
//! for this crate's own test targets (`#[cfg(feature = "test-adapters")]`,
//! enabled via the dev-dependency self-reference in `Cargo.toml`), never for
//! a normal build.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::adapter::{FunctionQueueConfig, QueueAdapter, QueueMessage, TopicInfo};
use crate::store::{InMemoryStore, TopicStats};
use crate::subscriber_config::SubscriberQueueConfig;
use crate::trigger::Invoker;

use super::builtin::BuiltinAdapter;

/// In-process transport identical to [`BuiltinAdapter`] over a fresh
/// [`InMemoryStore`], registered under its own adapter name so hot-swap
/// tests can exercise a swap between two dependency-free backends.
pub struct MemoryAdapter {
    inner: BuiltinAdapter,
}

impl MemoryAdapter {
    pub fn new(invoker: Arc<dyn Invoker>) -> Self {
        Self {
            inner: BuiltinAdapter::new(Arc::new(InMemoryStore::new()), invoker),
        }
    }
}

#[async_trait]
impl QueueAdapter for MemoryAdapter {
    async fn enqueue(
        &self,
        topic: &str,
        data: Value,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) -> anyhow::Result<()> {
        self.inner.enqueue(topic, data, traceparent, baggage).await
    }

    async fn subscribe(
        &self,
        topic: &str,
        id: &str,
        function_id: &str,
        condition_function_id: Option<String>,
        queue_config: Option<SubscriberQueueConfig>,
    ) {
        self.inner
            .subscribe(topic, id, function_id, condition_function_id, queue_config)
            .await;
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        self.inner.unsubscribe(topic, id).await;
    }

    async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64> {
        self.inner.redrive_dlq(topic).await
    }

    async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        self.inner.redrive_dlq_message(topic, message_id).await
    }

    async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        self.inner.discard_dlq_message(topic, message_id).await
    }

    async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64> {
        self.inner.dlq_count(topic).await
    }

    async fn dlq_peek(&self, topic: &str, offset: u64, limit: u64) -> anyhow::Result<Vec<Value>> {
        self.inner.dlq_peek(topic, offset, limit).await
    }

    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        self.inner.list_topics().await
    }

    async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats> {
        self.inner.topic_stats(topic).await
    }

    async fn shutdown(&self) {
        self.inner.shutdown().await;
    }

    // -- Function queue transport methods: same defaults as `BuiltinAdapter`
    // (unimplemented/no-op) -- delegated for completeness rather than relying
    // on the trait's own defaults, so a future override on `BuiltinAdapter`
    // is inherited here automatically.

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
        self.inner
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
        self.inner.setup_function_queue(queue_name, config).await
    }

    async fn consume_function_queue(
        &self,
        queue_name: &str,
        prefetch: u32,
    ) -> anyhow::Result<mpsc::Receiver<QueueMessage>> {
        self.inner
            .consume_function_queue(queue_name, prefetch)
            .await
    }

    async fn ack_function_queue(&self, queue_name: &str, delivery_id: u64) -> anyhow::Result<()> {
        self.inner.ack_function_queue(queue_name, delivery_id).await
    }

    async fn nack_function_queue(
        &self,
        queue_name: &str,
        delivery_id: u64,
        attempt: u32,
        max_retries: u32,
    ) -> anyhow::Result<()> {
        self.inner
            .nack_function_queue(queue_name, delivery_id, attempt, max_retries)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger::Invoker;
    use async_trait::async_trait;
    use serde_json::json;
    use std::time::Duration;
    use tokio::time::Instant;

    #[derive(Default)]
    struct RecordingInvoker {
        calls: tokio::sync::Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl Invoker for RecordingInvoker {
        async fn call(
            &self,
            function_id: &str,
            payload: Value,
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) -> Result<Option<Value>, String> {
            self.calls
                .lock()
                .await
                .push((function_id.to_string(), payload));
            Ok(None)
        }
    }

    #[tokio::test]
    async fn enqueue_and_subscribe_deliver_like_builtin() {
        let invoker = Arc::new(RecordingInvoker::default());
        let adapter = MemoryAdapter::new(invoker.clone());

        adapter.subscribe("demo", "sub-1", "fn-1", None, None).await;
        adapter
            .enqueue("demo", json!({"hello": "world"}), None, None)
            .await
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if !invoker.calls.lock().await.is_empty() {
                break;
            }
            assert!(Instant::now() < deadline, "message was never delivered");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let calls = invoker.calls.lock().await;
        assert_eq!(calls[0].0, "fn-1");
        assert_eq!(calls[0].1, json!({"hello": "world"}));
    }
}
