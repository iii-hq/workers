//! Redis pub/sub transport adapter.
//!
//! 1:1 port of the engine builtin's `RedisAdapter`
//! (`engine/src/workers/queue/adapters/redis_adapter.rs`, whole file),
//! including its limitations: this transport is pub/sub only — it has no
//! DLQ, no retries, no message durability, and delivers each published
//! message to at most one subscriber connection per topic. Seams applied
//! for the standalone worker:
//! - `Arc<Engine>` + `engine.call(...)` inside the pubsub task ->
//!   `Arc<dyn crate::trigger::Invoker>` + `invoker.call(function_id, payload)`.
//! - The engine's `check_condition` helper (`engine/src/condition.rs`) is
//!   reimplemented locally against `Invoker` with identical semantics:
//!   `Ok(Some(v))` -> proceed unless `v` is the JSON boolean `false`,
//!   `Ok(None)` -> proceed (with a warn log), `Err` -> skip the message
//!   (logged), condition_function_id handling is otherwise untouched.
//! - Telemetry spans (the engine's `tracing::info_span!` + OTel
//!   `with_parent_headers`) are dropped; the `__trace` envelope
//!   (`traceparent`/`baggage`) is still built on `enqueue` and unwrapped on
//!   `subscribe` exactly like the engine, so a future consumer could still
//!   read it, it just isn't used to parent a span here.
//! - `QueueAdapter::shutdown` (required by this worker's trait, absent from
//!   the engine's) aborts every subscription task, mirroring the engine's
//!   "abort consumer tasks in place" behavior at process teardown.
//!
//! Testability note: [`RedisAdapter::new`]/[`RedisAdapter::from_config`]
//! connect to Redis eagerly (engine parity), so unit tests can't construct
//! a live instance without a running Redis. Two pieces of pure logic are
//! therefore extracted so they're covered without a connection:
//! - [`RedisAdapter::resolve_redis_url`] — config parsing.
//! - [`unsubscribe_locked`] — the unsubscribe decision table, taking the
//!   already-locked map directly.
//!
//! The DLQ methods need no such extraction: they return a constant error
//! ([`DLQ_NOT_SUPPORTED`]) without touching `self` at all, so the constant
//! itself is the unit under test.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use redis::{aio::ConnectionManager, AsyncCommands, Client};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::adapter::{QueueAdapter, QueueMessage, TopicInfo};
use crate::subscriber_config::SubscriberQueueConfig;
use crate::trigger::Invoker;

/// `engine/src/workers/redis.rs:9`.
const DEFAULT_REDIS_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
/// `engine/src/workers/queue/adapters/redis_adapter.rs:84` (`make_adapter`).
const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";
/// Verbatim engine error string
/// (`engine/src/workers/queue/adapters/redis_adapter.rs:338` etc.).
const DLQ_NOT_SUPPORTED: &str = "RedisAdapter does not support DLQ operations (pub/sub only)";

pub struct RedisAdapter {
    publisher: Arc<Mutex<ConnectionManager>>,
    subscriber: Arc<Client>,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
    invoker: Arc<dyn Invoker>,
}

struct SubscriptionInfo {
    id: String,
    task_handle: JoinHandle<()>,
}

impl RedisAdapter {
    /// Connects to Redis eagerly, exact engine parity: same timeout
    /// constant and same error messages
    /// (`engine/src/workers/queue/adapters/redis_adapter.rs:48-63`).
    pub async fn new(redis_url: String, invoker: Arc<dyn Invoker>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url.as_str())?;

        let manager = timeout(
            DEFAULT_REDIS_CONNECTION_TIMEOUT,
            client.get_connection_manager(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Redis connection timed out after {:?}. Please ensure Redis is running at: {}",
                DEFAULT_REDIS_CONNECTION_TIMEOUT,
                redis_url
            )
        })?
        .map_err(|e| anyhow::anyhow!("Failed to connect to Redis at {}: {}", redis_url, e))?;

        Ok(Self {
            publisher: Arc::new(Mutex::new(manager)),
            subscriber: Arc::new(client),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            invoker,
        })
    }

    /// Pure config-parsing logic split out of [`Self::from_config`] so it's
    /// unit-testable without connecting to Redis. Engine parity: config key
    /// `redis_url`, default `redis://localhost:6379`
    /// (`engine/src/workers/queue/adapters/redis_adapter.rs:77-88`).
    fn resolve_redis_url(config: Option<&Value>) -> String {
        config
            .and_then(|v| v.get("redis_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| DEFAULT_REDIS_URL.to_string())
    }

    /// Factory entry point: parses `config` (`{redis_url}`) and connects.
    /// Called by the adapter factory that wires `config.adapter.name ==
    /// "redis"` into a live `RedisAdapter`.
    pub async fn from_config(
        config: Option<&Value>,
        invoker: Arc<dyn Invoker>,
    ) -> anyhow::Result<Self> {
        let redis_url = Self::resolve_redis_url(config);
        Self::new(redis_url, invoker).await
    }
}

/// Port of the engine's `check_condition` (`engine/src/condition.rs`)
/// against `Invoker` instead of `EngineTrait`. Same semantics: `Ok(true)`
/// proceed, `Ok(false)` skip (only when the condition function explicitly
/// returned JSON `false`), `Err` propagates the invocation failure.
async fn check_condition(
    invoker: &dyn Invoker,
    condition_function_id: &str,
    data: Value,
) -> Result<bool, String> {
    match invoker.call(condition_function_id, data).await {
        Ok(Some(result)) => Ok(result.as_bool() != Some(false)),
        Ok(None) => {
            tracing::warn!(
                condition_function_id = %condition_function_id,
                "Condition function returned no result"
            );
            Ok(true)
        }
        Err(e) => Err(e),
    }
}

/// The unsubscribe decision table, given the already write-locked map.
/// Extracted so it's unit-testable without a live Redis connection (a
/// `RedisAdapter` can't be constructed without one). Mirrors
/// `engine/src/workers/queue/adapters/redis_adapter.rs:314-334`.
fn unsubscribe_locked(subs: &mut HashMap<String, SubscriptionInfo>, topic: &str, id: &str) {
    if let Some(sub_info) = subs.remove(topic) {
        if sub_info.id == id {
            tracing::debug!(topic = %topic, id = %id, "Unsubscribing from Redis channel");
            sub_info.task_handle.abort();
        } else {
            tracing::warn!(topic = %topic, id = %id, "Subscription ID mismatch, not unsubscribing");
            subs.insert(topic.to_string(), sub_info);
        }
    } else {
        tracing::warn!(topic = %topic, id = %id, "No active subscription found for topic");
    }
}

#[async_trait]
impl QueueAdapter for RedisAdapter {
    async fn enqueue(
        &self,
        topic: &str,
        data: Value,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) {
        let topic = topic.to_string();
        let publisher = Arc::clone(&self.publisher);

        tracing::debug!(topic = %topic, data = %data, "Publishing to Redis queue");

        // Wrap data in an envelope that includes trace context — kept
        // exactly like the engine even though nothing here parents a span
        // off of it (see module doc).
        let envelope = serde_json::json!({
            "__trace": {
                "traceparent": traceparent,
                "baggage": baggage,
            },
            "data": data,
        });

        let json = match serde_json::to_string(&envelope) {
            Ok(json) => json,
            Err(e) => {
                tracing::error!(error = %e, topic = %topic, "Failed to serialize queue data");
                return;
            }
        };

        let mut conn = publisher.lock().await;

        if let Err(e) = conn.publish::<_, _, ()>(&topic, &json).await {
            tracing::error!(error = %e, topic = %topic, "Failed to publish to Redis queue");
        } else {
            tracing::debug!(topic = %topic, "Published to Redis queue");
        }
    }

    async fn subscribe(
        &self,
        topic: &str,
        id: &str,
        function_id: &str,
        metadata: Option<Value>,
        condition_function_id: Option<String>,
        _queue_config: Option<SubscriberQueueConfig>,
    ) {
        let topic = topic.to_string();
        let id = id.to_string();
        let function_id = function_id.to_string();
        let subscriber = Arc::clone(&self.subscriber);
        let invoker = Arc::clone(&self.invoker);
        let subscriptions = Arc::clone(&self.subscriptions);

        // Check if already subscribed — one subscription per topic per
        // adapter instance, same guard as the engine.
        let already_subscribed = {
            let subs = subscriptions.read().await;
            subs.contains_key(&topic)
        };

        if already_subscribed {
            tracing::warn!(topic = %topic, id = %id, "Already subscribed to topic");
            return;
        }

        let topic_for_task = topic.clone();
        let id_for_task = id.clone();
        let function_id_for_task = function_id.clone();
        let metadata_for_task = metadata.clone();
        let condition_function_id_for_task = condition_function_id.clone();

        tracing::debug!(topic = %topic_for_task, id = %id_for_task, function_id = %function_id_for_task, "Subscribing to Redis channel");

        let task_handle = tokio::spawn(async move {
            let mut pubsub = match subscriber.get_async_pubsub().await {
                Ok(pubsub) => pubsub,
                Err(e) => {
                    tracing::error!(error = %e, topic = %topic_for_task, "Failed to get async pubsub connection");
                    return;
                }
            };

            if let Err(e) = pubsub.subscribe(&topic_for_task).await {
                tracing::error!(error = %e, topic = %topic_for_task, "Failed to subscribe to Redis channel");
                return;
            }

            tracing::debug!(
                topic = %topic_for_task,
                id = %id_for_task,
                function_id = %function_id_for_task,
                condition_function_id = ?condition_function_id_for_task,
                "Subscribed to Redis channel"
            );

            let mut msg = pubsub.into_on_message();

            while let Some(msg) = msg.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(payload) => payload,
                    Err(e) => {
                        tracing::error!(error = %e, topic = %topic_for_task, "Failed to get message payload");
                        continue;
                    }
                };

                tracing::debug!(payload = %payload, "Received message from Redis");

                let parsed: Value = match serde_json::from_str(&payload) {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::error!(error = %e, topic = %topic_for_task, "Failed to parse message as JSON");
                        continue;
                    }
                };

                // Extract trace context from envelope, with backward
                // compatibility. Only treat as envelope when "__trace" is an
                // object AND "data" is present, so payloads that merely
                // contain a "__trace" key are not silently clobbered — same
                // guard as the engine.
                let (data, traceparent, baggage) =
                    if parsed.get("__trace").and_then(|v| v.as_object()).is_some()
                        && parsed.get("data").is_some()
                    {
                        let trace = parsed.get("__trace").unwrap();
                        let data = parsed.get("data").cloned().unwrap();
                        let traceparent = trace
                            .get("traceparent")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let baggage = trace
                            .get("baggage")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        (data, traceparent, baggage)
                    } else {
                        // Legacy format or non-envelope payload: use entire
                        // payload as data.
                        (parsed, None, None)
                    };

                tracing::debug!(topic = %topic_for_task, function_id = %function_id_for_task, traceparent = ?traceparent, baggage = ?baggage, "Received message from Redis queue, invoking function");

                if let Some(ref condition_id) = condition_function_id_for_task {
                    tracing::debug!(
                        condition_function_id = %condition_id,
                        "Checking trigger conditions"
                    );
                    match check_condition(invoker.as_ref(), condition_id, data.clone()).await {
                        Ok(true) => {}
                        Ok(false) => {
                            tracing::debug!(
                                function_id = %function_id_for_task,
                                "Condition check failed, skipping handler"
                            );
                            continue;
                        }
                        Err(err) => {
                            tracing::error!(
                                condition_function_id = %condition_id,
                                error = %err,
                                "Error invoking condition function"
                            );
                            continue;
                        }
                    }
                }

                let invoker = Arc::clone(&invoker);
                let function_id = function_id_for_task.clone();
                let metadata = metadata_for_task.clone();
                let topic_for_call = topic_for_task.clone();

                tokio::spawn(async move {
                    if let Err(e) = invoker.call_delivery(&function_id, data, metadata).await {
                        tracing::error!(
                            error = %e,
                            function_id = %function_id,
                            topic = %topic_for_call,
                            "Failed to invoke function for queue job"
                        );
                    }
                });
            }

            tracing::debug!(topic = %topic_for_task, id = %id_for_task, "Subscription task ended");
        });

        tracing::debug!("Subscription task spawned");

        // Store the subscription.
        let mut subs = subscriptions.write().await;
        subs.insert(topic, SubscriptionInfo { id, task_handle });
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        tracing::debug!(topic = %topic, id = %id, "Unsubscribing from Redis channel");
        let mut subs = self.subscriptions.write().await;
        unsubscribe_locked(&mut subs, topic, id);
    }

    async fn redrive_dlq(&self, _topic: &str) -> anyhow::Result<u64> {
        Err(anyhow::anyhow!(DLQ_NOT_SUPPORTED))
    }

    async fn redrive_dlq_message(&self, _topic: &str, _message_id: &str) -> anyhow::Result<bool> {
        Err(anyhow::anyhow!(DLQ_NOT_SUPPORTED))
    }

    async fn discard_dlq_message(&self, _topic: &str, _message_id: &str) -> anyhow::Result<bool> {
        Err(anyhow::anyhow!(DLQ_NOT_SUPPORTED))
    }

    async fn dlq_count(&self, _topic: &str) -> anyhow::Result<u64> {
        Err(anyhow::anyhow!(DLQ_NOT_SUPPORTED))
    }

    // `topic_stats` and `dlq_peek` are intentionally NOT overridden: the
    // engine's redis adapter doesn't override them either, so both fall
    // back to the trait's default bodies (`TopicStats::default()` /
    // `Ok(vec![])`) — same as the engine falling back to its own trait
    // defaults for this adapter.

    #[allow(clippy::too_many_arguments)]
    async fn publish_to_function_queue(
        &self,
        queue_name: &str,
        function_id: &str,
        data: Value,
        message_id: &str,
        _max_retries: u32,
        _backoff_ms: u64,
        traceparent: Option<String>,
        baggage: Option<String>,
        // RabbitMQ-only feature; the redis pub/sub adapter ignores it, same
        // as the engine.
        _priority: Option<u8>,
    ) -> anyhow::Result<()> {
        let channel = format!("__queue::{}", queue_name);
        let publisher = Arc::clone(&self.publisher);

        let envelope = serde_json::json!({
            "__trace": {
                "traceparent": traceparent,
                "baggage": baggage,
            },
            "function_id": function_id,
            "message_id": message_id,
            "data": data,
        });

        let json = serde_json::to_string(&envelope).map_err(|err| {
            anyhow::anyhow!("failed to serialize function queue message for '{queue_name}': {err}")
        })?;

        tracing::debug!(queue = %queue_name, function_id = %function_id, "Publishing to Redis function queue channel");

        let mut conn = publisher.lock().await;

        conn.publish::<_, _, ()>(&channel, &json)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "failed to publish to Redis function queue channel '{channel}': {err}"
                )
            })
    }

    async fn setup_function_queue(
        &self,
        _queue_name: &str,
        _config: &crate::adapter::FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        anyhow::bail!("Redis does not support durable function queues")
    }

    async fn consume_function_queue(
        &self,
        _queue_name: &str,
        _prefetch: u32,
    ) -> anyhow::Result<mpsc::Receiver<QueueMessage>> {
        anyhow::bail!("Redis does not support durable function queues")
    }

    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        // Redis adapter keys subscriptions by topic name directly, and only
        // ever allows one subscription per topic (the guard in
        // `subscribe`), so each present topic contributes exactly one to
        // its own count. This worker's `TopicInfo` has no
        // `broker_type`/`subscriber_count` fields (unlike the engine's), so
        // the subscriber count is carried in `depth` — the closest
        // available field — for lack of a better one.
        let subs = self.subscriptions.read().await;
        Ok(subs
            .keys()
            .map(|topic| TopicInfo {
                name: topic.clone(),
                depth: 1,
            })
            .collect())
    }

    async fn shutdown(&self) {
        let mut subs = self.subscriptions.write().await;
        for (_, sub) in subs.drain() {
            sub.task_handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_redis_url_defaults_when_absent() {
        assert_eq!(RedisAdapter::resolve_redis_url(None), DEFAULT_REDIS_URL);
    }

    #[test]
    fn resolve_redis_url_defaults_when_config_has_no_redis_url_key() {
        let config = json!({"other": "value"});
        assert_eq!(
            RedisAdapter::resolve_redis_url(Some(&config)),
            DEFAULT_REDIS_URL
        );
    }

    #[test]
    fn resolve_redis_url_uses_explicit_url() {
        let config = json!({"redis_url": "redis://example.com:1234"});
        assert_eq!(
            RedisAdapter::resolve_redis_url(Some(&config)),
            "redis://example.com:1234"
        );
    }

    #[test]
    fn dlq_not_supported_message_is_exact_engine_string() {
        // Single source of truth for all four DLQ-family trait methods —
        // see the module doc's testability note.
        assert_eq!(
            DLQ_NOT_SUPPORTED,
            "RedisAdapter does not support DLQ operations (pub/sub only)"
        );
    }

    #[tokio::test]
    async fn unsubscribe_unknown_topic_is_a_noop() {
        let mut subs: HashMap<String, SubscriptionInfo> = HashMap::new();
        unsubscribe_locked(&mut subs, "missing-topic", "some-id");
        assert!(subs.is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_mismatched_id_keeps_subscription() {
        let mut subs: HashMap<String, SubscriptionInfo> = HashMap::new();
        subs.insert(
            "demo".to_string(),
            SubscriptionInfo {
                id: "owner".to_string(),
                task_handle: tokio::spawn(async {
                    std::future::pending::<()>().await;
                }),
            },
        );
        unsubscribe_locked(&mut subs, "demo", "someone-else");
        assert!(
            subs.contains_key("demo"),
            "mismatched id must not remove the subscription"
        );
        subs.remove("demo").unwrap().task_handle.abort();
    }

    #[tokio::test]
    async fn unsubscribe_matching_id_removes_and_aborts() {
        let mut subs: HashMap<String, SubscriptionInfo> = HashMap::new();
        subs.insert(
            "demo".to_string(),
            SubscriptionInfo {
                id: "owner".to_string(),
                task_handle: tokio::spawn(async {
                    std::future::pending::<()>().await;
                }),
            },
        );
        unsubscribe_locked(&mut subs, "demo", "owner");
        assert!(!subs.contains_key("demo"));
    }
}
