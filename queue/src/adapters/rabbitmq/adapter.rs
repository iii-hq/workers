//! RabbitMQ transport adapter: implements
//! [`crate::adapter::QueueAdapter`] on top of the fanout-exchange +
//! per-function-queue topology (`topology.rs`), the job envelope
//! (`types.rs`), and the retry/DLQ decision table (`retry.rs`).
//!
//! Port of `engine/src/workers/queue/adapters/rabbitmq/adapter.rs`. Seams
//! applied for the standalone worker (mirrors every other adapter in this
//! crate, e.g. `adapters/redis.rs`):
//! - `Arc<Engine>` -> `Arc<dyn crate::trigger::Invoker>`.
//! - The engine's `make_adapter`/`register_adapter!` factory-registration
//!   macro (which plugs into the engine's own adapter registry) has no
//!   equivalent here; instead this module exposes
//!   [`RabbitMQAdapter::from_config`], matching the
//!   `RedisAdapter::from_config(config, invoker)` convention already
//!   established by `adapters/redis.rs`.
//! - No `Clone` impl on the adapter itself (the engine's exists so the
//!   factory can hand owned clones around before wrapping in `Arc`); this
//!   worker always shares the adapter via `Arc<dyn QueueAdapter>` /
//!   `SwappableAdapter`, so it isn't needed.
//! - Telemetry (OTel spans) dropped, same as every other adapter port in
//!   this crate.
//! - Return-type shape deviations, forced by this worker's simpler
//!   `crate::adapter::TopicInfo` (`{name, depth}` vs the engine's `{name,
//!   broker_type, subscriber_count}`) and `crate::store::TopicStats`
//!   (`{depth, dlq_depth, delivered, failed}` vs the engine's `{depth,
//!   consumer_count, dlq_depth, config}`) and the trait's `dlq_peek`
//!   returning `Vec<Value>` instead of the engine's richer `Vec<DlqMessage>`:
//!   - `list_topics`: `depth` carries the per-topic subscriber count (same
//!     repurposing `adapters/redis.rs::list_topics` already documents for
//!     its own `TopicInfo`, since this worker's struct has no
//!     `subscriber_count` field).
//!   - `topic_stats`: `consumer_count` is dropped (no field to put it in);
//!     `delivered`/`failed` are not tracked by this adapter (RabbitMQ is the
//!     source of truth for queue depth, not per-message delivery counters)
//!     and are always `0`, same as the engine's rabbitmq `topic_stats` never
//!     populating equivalent fields either.
//!   - `dlq_peek`: builds the same fields the engine's `DlqMessage` carries
//!     (`id`, `payload`, `error`, `failed_at`, `retries`, `size_bytes`) as a
//!     JSON object per message, merging the engine's separate
//!     `dlq_messages`/`dlq_peek` methods into this trait's single paginated
//!     `dlq_peek(topic, offset, limit)` (the engine's `dlq_messages` is
//!     effectively `dlq_peek(topic, 0, count)` with a lighter-weight
//!     rawvalue-only return; this worker's trait only exposes the
//!     paginated shape and defaults `dlq_messages` to calling it).

#![cfg(feature = "rabbitmq")]

use std::{
    collections::HashMap,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use lapin::{message::Delivery, options::*, Channel, Connection, ConnectionProperties};
use serde_json::Value;
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
use uuid::Uuid;

use crate::adapter::{FunctionQueueConfig, QueueAdapter, QueueMessage, TopicInfo};
use crate::store::TopicStats;
use crate::subscriber_config::SubscriberQueueConfig;
use crate::trigger::Invoker;

use super::naming::{FnQueueNames, RabbitNames};
use super::publisher::Publisher;
use super::retry::RetryHandler;
use super::topology::TopologyManager;
use super::types::{priority_from_data, Job, QueueMode, RabbitMQConfig};
use super::worker::Worker;

pub struct RabbitMQAdapter {
    connection: Arc<Connection>,
    publisher: Arc<Publisher>,
    retry_handler: Arc<RetryHandler>,
    topology: Arc<TopologyManager>,
    channel: Arc<Channel>,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
    invoker: Arc<dyn Invoker>,
    config: RabbitMQConfig,
    delivery_map: Arc<RwLock<HashMap<u64, Arc<FunctionQueueDelivery>>>>,
    delivery_counter: Arc<AtomicU64>,
    function_consumer_counter: AtomicU64,
    function_consumer_tasks: RwLock<Vec<FunctionConsumerHandle>>,
    function_queue_configs: RwLock<HashMap<String, FunctionQueueConfig>>,
}

struct FunctionQueueDelivery {
    delivery_id: u64,
    consumer_id: u64,
    queue_name: String,
    delivery: Delivery,
    operation: Mutex<()>,
    settled: AtomicBool,
}

struct FunctionConsumerHandle {
    queue_name: String,
    consumer_id: u64,
    consumer_tag: String,
    cancelled: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

struct SubscriptionInfo {
    id: String,
    consumer_tag: String,
    task_handle: JoinHandle<()>,
}

impl RabbitMQAdapter {
    /// Resolve the DLQ queue name for a given topic. Handles both function
    /// queues (`__fn_queue::` prefix) and topic-based queues.
    fn resolve_dlq_name(topic: &str) -> (String, bool) {
        if let Some(queue_name) = topic.strip_prefix("__fn_queue::") {
            (FnQueueNames::new(queue_name).dlq(), true)
        } else {
            (RabbitNames::new(topic).dlq(), false)
        }
    }

    /// Scan a DLQ for a specific message by ID, applying `on_found` when the
    /// target is located. Non-target messages are nacked back to the queue.
    ///
    /// CONCURRENCY NOTE: this uses `basic_get` + `nack(requeue)`, which is
    /// not atomic. Under concurrent DLQ operations, messages may be
    /// reordered. The iteration is bounded by the queue depth snapshot to
    /// prevent infinite loops from requeued messages, but concurrent
    /// producers may cause messages to be missed.
    async fn find_dlq_message<F, Fut>(
        &self,
        topic: &str,
        message_id: &str,
        on_found: F,
    ) -> anyhow::Result<bool>
    where
        F: FnOnce(&Delivery, bool, &str) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        let (dlq_name, is_fn_queue) = Self::resolve_dlq_name(topic);

        let queue_info = self
            .channel
            .queue_declare(
                &dlq_name,
                lapin::options::QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                lapin::types::FieldTable::default(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get DLQ info: {}", e))?;

        let count = queue_info.message_count();

        for _ in 0..count {
            let get_result = self
                .channel
                .basic_get(&dlq_name, BasicGetOptions { no_ack: false })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get message from DLQ: {}", e))?;

            let Some(delivery) = get_result else { break };

            // Determine if this is the target message.
            // Function queue DLQ: stable ID from AMQP message_id or body hash
            // Topic DLQ: ID is in the JSON payload at job.id
            let is_target = if is_fn_queue {
                delivery_stable_id(&delivery.delivery) == message_id
            } else {
                let dlq_payload: Value =
                    serde_json::from_slice(&delivery.delivery.data).unwrap_or(Value::Null);
                dlq_payload
                    .get("job")
                    .and_then(|j| j.get("id"))
                    .and_then(|id| id.as_str())
                    == Some(message_id)
            };

            if is_target {
                let queue_name = topic.strip_prefix("__fn_queue::").unwrap_or(topic);
                on_found(&delivery.delivery, is_fn_queue, queue_name).await?;

                delivery
                    .delivery
                    .ack(BasicAckOptions { multiple: false })
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to ack target message: {}", e))?;

                return Ok(true);
            }

            // Not the target -- put it back
            delivery
                .delivery
                .nack(BasicNackOptions {
                    requeue: true,
                    multiple: false,
                })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to nack non-target message: {}", e))?;
        }

        Ok(false)
    }

    pub async fn new(config: RabbitMQConfig, invoker: Arc<dyn Invoker>) -> anyhow::Result<Self> {
        let connection = Arc::new(
            Connection::connect(&config.amqp_url, ConnectionProperties::default())
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to connect to RabbitMQ at {}: {}",
                        config.amqp_url,
                        e
                    )
                })?,
        );

        let channel = connection
            .create_channel()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create RabbitMQ channel: {}", e))?;

        channel
            .confirm_select(ConfirmSelectOptions::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to enable RabbitMQ publisher confirms: {}", e))?;

        let effective_prefetch = match config.queue_mode {
            QueueMode::Fifo => 1,
            QueueMode::Standard => config.prefetch_count,
        };

        channel
            .basic_qos(effective_prefetch, BasicQosOptions::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to set QoS: {}", e))?;

        let channel = Arc::new(channel);
        let publisher = Arc::new(Publisher::new(Arc::clone(&channel)));
        let topology = Arc::new(TopologyManager::new(Arc::clone(&channel)));
        let retry_handler = Arc::new(RetryHandler::new(Arc::clone(&publisher)));

        Ok(Self {
            connection,
            publisher,
            retry_handler,
            topology,
            channel,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            invoker,
            config,
            delivery_map: Arc::new(RwLock::new(HashMap::new())),
            delivery_counter: Arc::new(AtomicU64::new(0)),
            function_consumer_counter: AtomicU64::new(1),
            function_consumer_tasks: RwLock::new(Vec::new()),
            function_queue_configs: RwLock::new(HashMap::new()),
        })
    }

    /// Factory entry point: parses `config` (`{amqp_url, max_attempts,
    /// prefetch_count, queue_mode, priority_field}`, see
    /// [`RabbitMQConfig::from_value`]) and connects. Mirrors
    /// `RedisAdapter::from_config`'s convention for this crate's other
    /// adapters.
    pub async fn from_config(
        config: Option<&Value>,
        invoker: Arc<dyn Invoker>,
    ) -> anyhow::Result<Self> {
        let config = RabbitMQConfig::from_value(config);
        Self::new(config, invoker).await
    }
}

/// Extracts a stable message ID from a DLQ delivery. Tries the AMQP
/// `message_id` property first, then falls back to a hash of the raw body
/// so that `dlq_peek` and `redrive_dlq_message` always agree on the ID.
fn delivery_stable_id(delivery: &Delivery) -> String {
    if let Some(mid) = delivery.properties.message_id() {
        let s = mid.as_str();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    delivery.data.hash(&mut hasher);
    format!("dlq-{:016x}", hasher.finish())
}

#[async_trait]
impl QueueAdapter for RabbitMQAdapter {
    async fn enqueue(
        &self,
        topic: &str,
        data: Value,
        traceparent: Option<String>,
        baggage: Option<String>,
    ) {
        // Topic fanout publishes one message to every bound subscriber queue, so
        // the priority is resolved once here from the adapter-level
        // `priority_field`. Each subscriber queue honors it only if declared with
        // `max_priority`, and clamps it to its own `x-max-priority`.
        let priority = priority_from_data(&data, self.config.priority_field.as_deref());
        let job = Job::new(topic, data, self.config.max_attempts, traceparent, baggage)
            .with_priority(priority);

        if let Err(e) = self.topology.setup_topic(topic).await {
            tracing::error!(
                error = ?e,
                topic = %topic,
                "Failed to setup RabbitMQ topology"
            );
            return;
        }

        if let Err(e) = self.publisher.publish(topic, &job).await {
            tracing::error!(
                error = ?e,
                topic = %topic,
                "Failed to publish to RabbitMQ"
            );
        } else {
            tracing::debug!(
                topic = %topic,
                job_id = %job.id,
                "Published to RabbitMQ queue"
            );
        }
    }

    async fn subscribe(
        &self,
        topic: &str,
        id: &str,
        function_id: &str,
        condition_function_id: Option<String>,
        queue_config: Option<SubscriberQueueConfig>,
    ) {
        let topic = topic.to_string();
        let id = id.to_string();
        let function_id = function_id.to_string();
        let subscriptions = Arc::clone(&self.subscriptions);

        let already_subscribed = {
            let subs = subscriptions.read().await;
            subs.contains_key(&format!("{}:{}", topic, id))
        };

        if already_subscribed {
            tracing::warn!(topic = %topic, id = %id, "Already subscribed to topic");
            return;
        }

        if let Err(e) = self.topology.setup_topic(&topic).await {
            tracing::error!(
                error = ?e,
                topic = %topic,
                "Failed to setup RabbitMQ fanout exchange"
            );
            return;
        }

        let subscriber_max_priority = queue_config.as_ref().and_then(|c| c.max_priority);
        if let Err(e) = self
            .topology
            .setup_subscriber_queue(&topic, &function_id, subscriber_max_priority)
            .await
        {
            tracing::error!(
                error = ?e,
                topic = %topic,
                function_id = %function_id,
                "Failed to setup RabbitMQ per-function queue"
            );
            return;
        }

        let names = RabbitNames::new(&topic);
        let per_function_queue = names.function_queue(&function_id);
        let consumer_tag = format!("consumer-{}", Uuid::new_v4());

        let effective_queue_mode = queue_config
            .as_ref()
            .and_then(|c| c.queue_mode.as_ref())
            .map(|mode| QueueMode::from_str(mode).unwrap_or_default())
            .unwrap_or_else(|| self.config.queue_mode.clone());

        let effective_prefetch_count = queue_config
            .as_ref()
            .and_then(|c| c.concurrency)
            .map(|c| c as u16)
            .unwrap_or(self.config.prefetch_count);

        let worker = Arc::new(Worker::new(
            Arc::clone(&self.channel),
            Arc::clone(&self.retry_handler),
            Arc::clone(&self.invoker),
            effective_queue_mode,
            effective_prefetch_count,
        ));

        let topic_clone = topic.clone();
        let function_id_clone = function_id.clone();
        let consumer_tag_clone = consumer_tag.clone();
        let queue_name_clone = per_function_queue.clone();

        let task_handle = tokio::spawn(async move {
            worker
                .run(
                    topic_clone,
                    function_id_clone,
                    condition_function_id,
                    consumer_tag_clone,
                    queue_name_clone,
                )
                .await;
        });

        let mut subs = subscriptions.write().await;
        subs.insert(
            format!("{}:{}", topic, id),
            SubscriptionInfo {
                id,
                consumer_tag,
                task_handle,
            },
        );

        tracing::debug!(
            topic = %topic,
            function_id = %function_id,
            queue = %per_function_queue,
            "Subscribed to RabbitMQ per-function queue"
        );
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        let subscriptions = Arc::clone(&self.subscriptions);
        let key = format!("{}:{}", topic, id);

        let mut subs = subscriptions.write().await;

        if let Some(sub_info) = subs.remove(&key) {
            if sub_info.id == id {
                tracing::debug!(
                    topic = %topic,
                    id = %id,
                    "Unsubscribing from RabbitMQ queue"
                );

                if let Err(e) = self
                    .channel
                    .basic_cancel(&sub_info.consumer_tag, BasicCancelOptions::default())
                    .await
                {
                    tracing::error!(
                        error = ?e,
                        topic = %topic,
                        consumer_tag = %sub_info.consumer_tag,
                        "Failed to cancel consumer"
                    );
                }

                sub_info.task_handle.abort();
            } else {
                tracing::warn!(
                    topic = %topic,
                    id = %id,
                    "Subscription ID mismatch, not unsubscribing"
                );
                subs.insert(key, sub_info);
            }
        } else {
            tracing::warn!(
                topic = %topic,
                id = %id,
                "No active subscription found for topic"
            );
        }
    }

    async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64> {
        let (dlq_name, is_fn_queue) = Self::resolve_dlq_name(topic);
        let mut count: u64 = 0;

        loop {
            let get_result = self
                .channel
                .basic_get(&dlq_name, BasicGetOptions { no_ack: false })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get message from DLQ: {}", e))?;

            match get_result {
                Some(delivery) => {
                    let republish_result: anyhow::Result<()> = if is_fn_queue {
                        // Function queue DLQ: raw data payload, republish directly
                        let queue_name = topic.strip_prefix("__fn_queue::").unwrap();
                        let names = FnQueueNames::new(queue_name);

                        let mut headers = delivery
                            .delivery
                            .properties
                            .headers()
                            .clone()
                            .unwrap_or_default();
                        headers.insert("x-attempt".into(), lapin::types::AMQPValue::LongUInt(0));

                        let properties = lapin::BasicProperties::default()
                            .with_content_type("application/json".into())
                            .with_delivery_mode(2)
                            .with_headers(headers);

                        let properties =
                            if let Some(mid) = delivery.delivery.properties.message_id() {
                                properties.with_message_id(mid.clone())
                            } else {
                                properties
                            };

                        self.channel
                            .basic_publish(
                                &names.exchange(),
                                queue_name,
                                BasicPublishOptions::default(),
                                &delivery.delivery.data,
                                properties,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to republish: {}", e))?
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to confirm: {}", e))?;
                        Ok(())
                    } else {
                        // Topic DLQ: wrapped payload with job/error/exhausted_at
                        let dlq_payload: Value = serde_json::from_slice(&delivery.delivery.data)
                            .map_err(|e| anyhow::anyhow!("Failed to parse DLQ message: {}", e))?;

                        let job: Job = serde_json::from_value(
                            dlq_payload
                                .get("job")
                                .ok_or_else(|| anyhow::anyhow!("DLQ message missing 'job' field"))?
                                .clone(),
                        )
                        .map_err(|e| anyhow::anyhow!("Failed to parse job: {}", e))?;

                        let mut redriven = job;
                        redriven.attempts_made = 0;

                        self.publisher
                            .publish(topic, &redriven)
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to republish: {}", e))?;
                        Ok(())
                    };

                    if let Err(e) = republish_result {
                        tracing::error!(error = ?e, "Failed to republish DLQ message");
                        delivery
                            .delivery
                            .nack(BasicNackOptions {
                                requeue: true,
                                multiple: false,
                            })
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to nack message: {}", e))?;
                        break;
                    }

                    delivery
                        .delivery
                        .ack(BasicAckOptions { multiple: false })
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to ack message: {}", e))?;

                    count += 1;
                }
                None => break,
            }
        }

        Ok(count)
    }

    async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        let publisher = self.publisher.clone();
        let channel = Arc::clone(&self.channel);
        let topic_owned = topic.to_string();

        self.find_dlq_message(topic, message_id, |delivery, is_fn_queue, queue_name| {
            let delivery_data = delivery.data.clone();
            let delivery_props = delivery.properties.clone();
            let queue_name = queue_name.to_string();

            async move {
                if is_fn_queue {
                    // Function queue: republish raw data to the function queue exchange
                    let names = FnQueueNames::new(&queue_name);

                    // Preserve original headers, reset attempt counter
                    let mut headers = delivery_props.headers().clone().unwrap_or_default();
                    headers.insert("x-attempt".into(), lapin::types::AMQPValue::LongUInt(0));

                    let properties = lapin::BasicProperties::default()
                        .with_content_type("application/json".into())
                        .with_delivery_mode(2)
                        .with_headers(headers);

                    // Copy message_id if present
                    let properties = if let Some(mid) = delivery_props.message_id() {
                        properties.with_message_id(mid.clone())
                    } else {
                        properties
                    };

                    channel
                        .basic_publish(
                            &names.exchange(),
                            &queue_name,
                            BasicPublishOptions::default(),
                            &delivery_data,
                            properties,
                        )
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to republish to function queue: {}", e)
                        })?
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to confirm republish: {}", e))?;
                } else {
                    // Topic queue: parse job, reset attempts, republish via Publisher
                    let dlq_payload: Value = serde_json::from_slice(&delivery_data)
                        .map_err(|e| anyhow::anyhow!("Failed to parse DLQ message: {}", e))?;

                    let job_value = dlq_payload
                        .get("job")
                        .ok_or_else(|| anyhow::anyhow!("DLQ message missing 'job' field"))?;

                    let mut job: Job = serde_json::from_value(job_value.clone())
                        .map_err(|e| anyhow::anyhow!("Failed to parse job: {}", e))?;

                    job.attempts_made = 0;

                    publisher
                        .publish(&topic_owned, &job)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to republish message: {}", e))?;
                }

                Ok(())
            }
        })
        .await
    }

    async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
        // Discard = just ack (handled by find_dlq_message after on_found returns)
        self.find_dlq_message(
            topic,
            message_id,
            |_delivery, _is_fn_queue, _queue_name| async { Ok(()) },
        )
        .await
    }

    async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64> {
        // Function queues use FnQueueNames (e.g., __fn_queue::orders -> ::dlq.queue),
        // while topic-based queues use RabbitNames (e.g., user.created -> .dlq).
        let (dlq_name, _is_fn_queue) = Self::resolve_dlq_name(topic);

        let queue = self
            .channel
            .queue_declare(
                &dlq_name,
                lapin::options::QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                lapin::types::FieldTable::default(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get DLQ info: {}", e))?;

        Ok(queue.message_count() as u64)
    }

    /// Merges the engine's separate `dlq_messages`/`dlq_peek` methods (see
    /// the module doc) into this trait's single paginated shape, building
    /// the same fields the engine's `DlqMessage` struct carries as a JSON
    /// object per entry instead of a dedicated type.
    async fn dlq_peek(&self, topic: &str, offset: u64, limit: u64) -> anyhow::Result<Vec<Value>> {
        let (dlq_name, is_fn_queue) = Self::resolve_dlq_name(topic);

        let queue_depth = match self
            .channel
            .queue_declare(
                &dlq_name,
                lapin::options::QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                lapin::types::FieldTable::default(),
            )
            .await
        {
            Ok(info) => info.message_count() as u64,
            Err(_) => return Ok(vec![]),
        };

        let fetch_count = (offset + limit).min(queue_depth) as usize;
        let mut results = Vec::new();
        let mut deliveries_to_nack = Vec::new();

        for i in 0..fetch_count {
            let get_result = self
                .channel
                .basic_get(&dlq_name, BasicGetOptions { no_ack: false })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to get message from DLQ: {}", e))?;

            let Some(delivery) = get_result else { break };

            if (i as u64) >= offset {
                let raw_data = &delivery.delivery.data;
                let payload: Value = serde_json::from_slice(raw_data).unwrap_or(Value::Null);

                let dlq_value = if is_fn_queue {
                    // Function queue DLQ: stable ID from AMQP properties or body hash
                    let id = delivery_stable_id(&delivery.delivery);

                    let function_id = delivery
                        .delivery
                        .properties
                        .headers()
                        .as_ref()
                        .and_then(|h| h.inner().get("function_id"))
                        .and_then(|v| match v {
                            lapin::types::AMQPValue::LongString(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .unwrap_or_default();

                    let retries = delivery
                        .delivery
                        .properties
                        .headers()
                        .as_ref()
                        .and_then(|h| h.inner().get("x-attempt"))
                        .and_then(|v| match v {
                            lapin::types::AMQPValue::LongUInt(n) => Some(*n),
                            _ => None,
                        })
                        .unwrap_or(0);

                    serde_json::json!({
                        "id": id,
                        "payload": payload,
                        "error": format!("Function {} exhausted retries", function_id),
                        "failed_at": 0,
                        "retries": retries,
                        "size_bytes": raw_data.len() as u64,
                    })
                } else {
                    // Topic-based DLQ: wrapped payload with job/error/exhausted_at
                    let id = payload
                        .get("job")
                        .and_then(|j| j.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let job_payload = payload
                        .get("job")
                        .and_then(|j| j.get("data"))
                        .cloned()
                        .unwrap_or(Value::Null);

                    let error = payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let failed_at = payload
                        .get("exhausted_at")
                        .and_then(|v| v.as_u64())
                        .map(|ms| ms / 1000)
                        .unwrap_or(0);

                    let retries = payload
                        .get("job")
                        .and_then(|j| j.get("attempts_made"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    serde_json::json!({
                        "id": id,
                        "payload": job_payload,
                        "error": error,
                        "failed_at": failed_at,
                        "retries": retries,
                        "size_bytes": raw_data.len() as u64,
                    })
                };

                results.push(dlq_value);
            }

            deliveries_to_nack.push(delivery);
        }

        // Nack all back to DLQ (peek, not consume)
        for delivery in deliveries_to_nack {
            let _ = delivery
                .delivery
                .nack(BasicNackOptions {
                    requeue: true,
                    multiple: false,
                })
                .await;
        }

        Ok(results)
    }

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
        priority: Option<u8>,
        namespace: Option<String>,
    ) -> anyhow::Result<()> {
        let names = FnQueueNames::new(queue_name);

        let payload = serde_json::to_vec(&data).map_err(|err| {
            anyhow::anyhow!("failed to serialize function queue message for '{queue_name}': {err}")
        })?;

        let mut headers = lapin::types::FieldTable::default();
        headers.insert(
            "function_id".into(),
            lapin::types::AMQPValue::LongString(function_id.into()),
        );
        if let Some(tp) = &traceparent {
            headers.insert(
                "traceparent".into(),
                lapin::types::AMQPValue::LongString(tp.as_str().into()),
            );
        }
        if let Some(bg) = &baggage {
            headers.insert(
                "baggage".into(),
                lapin::types::AMQPValue::LongString(bg.as_str().into()),
            );
        }
        if let Some(ns) = &namespace {
            headers.insert(
                "namespace".into(),
                lapin::types::AMQPValue::LongString(ns.as_str().into()),
            );
        }

        let mut properties = lapin::BasicProperties::default()
            .with_content_type("application/json".into())
            .with_delivery_mode(2)
            .with_message_id(message_id.into())
            .with_headers(headers);
        if let Some(p) = priority {
            properties = properties.with_priority(p);
        }

        let confirmation = self
            .channel
            .basic_publish(
                &names.exchange(),
                queue_name,
                BasicPublishOptions::default(),
                &payload,
                properties,
            )
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to publish to function queue '{queue_name}': {err}")
            })?
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to confirm publish to function queue '{queue_name}': {err}")
            })?;

        if !confirmation.is_ack() {
            anyhow::bail!("RabbitMQ did not acknowledge publish to function queue '{queue_name}'");
        }

        Ok(())
    }

    async fn setup_function_queue(
        &self,
        queue_name: &str,
        config: &FunctionQueueConfig,
    ) -> anyhow::Result<()> {
        // Queue arguments such as x-max-priority are immutable in RabbitMQ.
        // Provision named queues on a disposable channel so a conflicting
        // declaration closes only that channel, not the shared channel used
        // by every live publisher and consumer in this adapter.
        let channel = self
            .connection
            .create_channel()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create topology channel: {e}"))?;
        TopologyManager::new(Arc::new(channel))
            .setup_function_queue(queue_name, config.backoff_ms, config.max_priority)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to setup function queue topology: {}", e))?;
        self.function_queue_configs
            .write()
            .await
            .insert(queue_name.to_string(), config.clone());
        Ok(())
    }

    async fn consume_function_queue(
        &self,
        queue_name: &str,
        prefetch: u32,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<QueueMessage>> {
        use futures::StreamExt;

        let names = FnQueueNames::new(queue_name);

        self.channel
            .basic_qos(prefetch as u16, BasicQosOptions::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to set QoS: {}", e))?;

        let consumer_tag = format!("fn-queue-{}-{}", queue_name, Uuid::new_v4());
        let mut consumer = self
            .channel
            .basic_consume(
                &names.queue(),
                &consumer_tag,
                BasicConsumeOptions::default(),
                lapin::types::FieldTable::default(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create consumer: {}", e))?;

        let (tx, rx) = tokio::sync::mpsc::channel(prefetch.max(1) as usize);
        let delivery_map = Arc::clone(&self.delivery_map);
        let delivery_counter = Arc::clone(&self.delivery_counter);
        let consumer_id = self
            .function_consumer_counter
            .fetch_add(1, Ordering::Relaxed);
        let queue_name = queue_name.to_string();
        let task_queue_name = queue_name.clone();
        let task_consumer_tag = consumer_tag.clone();
        let channel = Arc::clone(&self.channel);
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = cancelled.clone();

        let task = tokio::spawn(async move {
            loop {
                let delivery_result = tokio::select! {
                    _ = tx.closed() => break,
                    delivery = consumer.next() => delivery,
                };
                let Some(delivery_result) = delivery_result else {
                    break;
                };
                match delivery_result {
                    Ok(delivery) => {
                        let delivery_id = delivery_counter.fetch_add(1, Ordering::SeqCst);

                        let data: Value = match serde_json::from_slice(&delivery.data) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to parse function queue message");
                                let _ = delivery
                                    .nack(BasicNackOptions {
                                        requeue: false,
                                        ..Default::default()
                                    })
                                    .await;
                                continue;
                            }
                        };

                        let headers = delivery.properties.headers().as_ref();

                        let function_id = headers
                            .and_then(|h| h.inner().get("function_id"))
                            .and_then(|v| match v {
                                lapin::types::AMQPValue::LongString(s) => Some(s.to_string()),
                                _ => None,
                            })
                            .unwrap_or_default();

                        let traceparent = headers
                            .and_then(|h| h.inner().get("traceparent"))
                            .and_then(|v| match v {
                                lapin::types::AMQPValue::LongString(s) => Some(s.to_string()),
                                _ => None,
                            });

                        let baggage =
                            headers
                                .and_then(|h| h.inner().get("baggage"))
                                .and_then(|v| match v {
                                    lapin::types::AMQPValue::LongString(s) => Some(s.to_string()),
                                    _ => None,
                                });

                        let namespace = headers
                            .and_then(|h| h.inner().get("namespace"))
                            .and_then(|v| match v {
                                lapin::types::AMQPValue::LongString(s) => Some(s.to_string()),
                                _ => None,
                            });

                        let attempt = headers
                            .and_then(|h| h.inner().get("x-attempt"))
                            .and_then(|v| match v {
                                lapin::types::AMQPValue::LongUInt(n) => Some(*n),
                                _ => None,
                            })
                            .unwrap_or(0);

                        let message_id = delivery
                            .properties
                            .message_id()
                            .as_ref()
                            .map(|s| s.to_string());

                        delivery_map.write().await.insert(
                            delivery_id,
                            Arc::new(FunctionQueueDelivery {
                                delivery_id,
                                consumer_id,
                                queue_name: task_queue_name.clone(),
                                delivery,
                                operation: Mutex::new(()),
                                settled: AtomicBool::new(false),
                            }),
                        );

                        let msg = QueueMessage {
                            delivery_id,
                            function_id,
                            data,
                            attempt,
                            message_id,
                            traceparent,
                            baggage,
                            namespace,
                        };

                        if tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Error receiving delivery from function queue");
                    }
                }
            }
            if task_cancelled
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                if let Err(err) = channel
                    .basic_cancel(&task_consumer_tag, BasicCancelOptions::default())
                    .await
                {
                    task_cancelled.store(false, Ordering::Release);
                    tracing::warn!(queue = %task_queue_name, error = %err, "failed to cancel RabbitMQ function queue consumer");
                }
            }
            if let Err(err) = requeue_rabbitmq_deliveries(&delivery_map, Some(consumer_id)).await {
                tracing::error!(queue = %task_queue_name, error = %err, "failed to requeue RabbitMQ function queue deliveries after consumer stopped");
            }
        });
        self.function_consumer_tasks
            .write()
            .await
            .push(FunctionConsumerHandle {
                queue_name,
                consumer_id,
                consumer_tag,
                cancelled,
                task,
            });

        Ok(rx)
    }

    async fn stop_function_queue_consumer(&self, queue_name: &str) -> anyhow::Result<()> {
        let matching = {
            let mut active = self.function_consumer_tasks.write().await;
            let mut matching = Vec::new();
            let mut retained = Vec::with_capacity(active.len());
            for handle in active.drain(..) {
                if handle.queue_name == queue_name {
                    matching.push(handle);
                } else {
                    retained.push(handle);
                }
            }
            *active = retained;
            matching
        };

        let mut errors = Vec::new();
        for handle in &matching {
            if handle
                .cancelled
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                if let Err(err) = self
                    .channel
                    .basic_cancel(&handle.consumer_tag, BasicCancelOptions::default())
                    .await
                {
                    handle.cancelled.store(false, Ordering::Release);
                    errors.push(format!(
                        "failed to cancel RabbitMQ function queue consumer '{}': {err}",
                        handle.consumer_tag
                    ));
                }
            }
            handle.task.abort();
        }

        for handle in matching {
            let _ = handle.task.await;
            if let Err(err) =
                requeue_rabbitmq_deliveries(&self.delivery_map, Some(handle.consumer_id)).await
            {
                errors.push(err.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(errors.join("; "))
        }
    }

    async fn forget_function_queue(&self, queue_name: &str) -> anyhow::Result<()> {
        self.function_queue_configs.write().await.remove(queue_name);
        Ok(())
    }

    async fn ack_function_queue(&self, queue_name: &str, delivery_id: u64) -> anyhow::Result<()> {
        let delivery = self.delivery_map.read().await.get(&delivery_id).cloned();
        let Some(delivery) = delivery else {
            return Ok(());
        };
        let _operation = delivery.operation.lock().await;
        if delivery.settled.load(Ordering::Acquire) {
            return Ok(());
        }
        if delivery.queue_name != queue_name {
            anyhow::bail!("delivery {delivery_id} does not belong to queue '{queue_name}'");
        }
        delivery
            .delivery
            .ack(BasicAckOptions::default())
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("Failed to ack: {}", e))?;
        delivery.settled.store(true, Ordering::Release);
        self.delivery_map.write().await.remove(&delivery_id);
        Ok(())
    }

    async fn nack_function_queue(
        &self,
        queue_name: &str,
        delivery_id: u64,
        attempt: u32,
        max_retries: u32,
    ) -> anyhow::Result<()> {
        let delivery = self.delivery_map.read().await.get(&delivery_id).cloned();
        let Some(delivery) = delivery else {
            return Ok(());
        };
        let _operation = delivery.operation.lock().await;
        if delivery.settled.load(Ordering::Acquire) {
            return Ok(());
        }
        if delivery.queue_name != queue_name {
            anyhow::bail!("delivery {delivery_id} does not belong to queue '{queue_name}'");
        }
        let broker_delivery = &delivery.delivery;

        if attempt < max_retries {
            let names = FnQueueNames::new(queue_name);

            let mut headers = broker_delivery
                .properties
                .headers()
                .clone()
                .unwrap_or_default();

            // Increment our own attempt counter so classic queues (which do not
            // populate x-delivery-count) can still track retry depth.
            let current_attempt = headers
                .inner()
                .get("x-attempt")
                .and_then(|v| match v {
                    lapin::types::AMQPValue::LongUInt(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(0);
            headers.insert(
                "x-attempt".into(),
                lapin::types::AMQPValue::LongUInt(current_attempt + 1),
            );

            let mut properties = lapin::BasicProperties::default()
                .with_content_type("application/json".into())
                .with_delivery_mode(2)
                .with_headers(headers);

            // Preserve message_id through retries so DLQ messages remain identifiable
            if let Some(mid) = broker_delivery.properties.message_id() {
                properties = properties.with_message_id(mid.clone());
            }
            if let Some(priority) = broker_delivery.properties.priority() {
                properties = properties.with_priority(*priority);
            }

            let base_backoff_ms = self
                .function_queue_configs
                .read()
                .await
                .get(queue_name)
                .map_or(1_000, |config| config.backoff_ms);
            let delay_ms = base_backoff_ms.saturating_mul(2_u64.saturating_pow(attempt));
            properties = properties.with_expiration(delay_ms.to_string().into());

            let publish_result: anyhow::Result<_> = async {
                let confirmation = self
                    .channel
                    .basic_publish(
                        &names.retry_exchange(),
                        queue_name,
                        BasicPublishOptions::default(),
                        &broker_delivery.data,
                        properties,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to publish to retry exchange: {}", e))?
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to confirm retry publish: {}", e))?;
                if !confirmation.is_ack() {
                    anyhow::bail!("RabbitMQ did not acknowledge retry publish");
                }
                Ok(confirmation)
            }
            .await;

            if let Err(publish_err) = publish_result {
                let requeue_result = broker_delivery
                    .nack(BasicNackOptions {
                        requeue: true,
                        ..Default::default()
                    })
                    .await;
                return match requeue_result {
                    Ok(_) => {
                        delivery.settled.store(true, Ordering::Release);
                        self.delivery_map.write().await.remove(&delivery_id);
                        Err(publish_err)
                    }
                    Err(requeue_err) => Err(anyhow::anyhow!(
                        "{publish_err}; also failed to requeue original delivery: {requeue_err}"
                    )),
                };
            }

            // Publish and confirm the retry copy before acknowledging the
            // original. This chooses a possible duplicate over message loss
            // when the broker fails between the two operations.
            broker_delivery
                .ack(BasicAckOptions::default())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to ack for retry: {}", e))?;

            tracing::debug!(
                queue = %queue_name,
                attempt = attempt,
                max_retries = max_retries,
                "Message sent to retry queue"
            );
        } else {
            // Exhausted: nack without requeue. The main queue's DLX points to the
            // DLQ exchange, so RabbitMQ routes the message there automatically.
            broker_delivery
                .nack(BasicNackOptions {
                    requeue: false,
                    ..Default::default()
                })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to nack to DLQ: {}", e))?;

            tracing::warn!(
                queue = %queue_name,
                attempt = attempt,
                max_retries = max_retries,
                "Message exhausted retries, routed to DLQ"
            );
        }

        delivery.settled.store(true, Ordering::Release);
        self.delivery_map.write().await.remove(&delivery_id);
        Ok(())
    }

    /// Deviation from the engine (see module doc): `consumer_count` is
    /// dropped (no field for it in this worker's `TopicStats`);
    /// `delivered`/`failed` are always `0` (not tracked by this adapter).
    ///
    /// Inherits one engine quirk verbatim: the "main queue depth" is looked
    /// up via `RabbitNames::new(topic).queue()` (`iii.{topic}.queue`), a
    /// queue name that topic-fanout subscriptions never actually declare
    /// (`subscribe` declares per-function queues via
    /// `RabbitNames::function_queue`, never the bare `.queue()` name) -- so
    /// for topic-based (non-`__fn_queue::`) topics this always resolves to
    /// depth `0` in both the engine and this port.
    async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats> {
        let (queue_name, dlq_name) = if let Some(name) = topic.strip_prefix("__fn_queue::") {
            let names = FnQueueNames::new(name);
            (names.queue(), names.dlq())
        } else {
            let names = RabbitNames::new(topic);
            (names.queue(), names.dlq())
        };

        let depth = match self
            .channel
            .queue_declare(
                &queue_name,
                lapin::options::QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                lapin::types::FieldTable::default(),
            )
            .await
        {
            Ok(info) => info.message_count() as u64,
            Err(_) => 0,
        };

        let dlq_depth = match self
            .channel
            .queue_declare(
                &dlq_name,
                lapin::options::QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                lapin::types::FieldTable::default(),
            )
            .await
        {
            Ok(info) => info.message_count() as u64,
            Err(_) => 0,
        };

        Ok(TopicStats {
            depth,
            dlq_depth,
            delivered: 0,
            failed: 0,
        })
    }

    /// Deviation from the engine (see module doc): `depth` carries the
    /// per-topic subscriber count (this worker's `TopicInfo` has no
    /// `subscriber_count` field), same repurposing `adapters/redis.rs`
    /// already documents for its own `list_topics`.
    async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
        let subs = self.subscriptions.read().await;
        let mut topics: HashMap<String, u64> = HashMap::new();
        for key in subs.keys() {
            // subscription key format is "topic:id"
            if let Some(topic) = key.split(':').next() {
                *topics.entry(topic.to_string()).or_insert(0u64) += 1;
            }
        }
        drop(subs);
        let function_queues = self
            .function_queue_configs
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for queue_name in function_queues {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                topics.entry(queue_name.clone())
            {
                let depth = self
                    .topic_stats(&format!("__fn_queue::{queue_name}"))
                    .await?
                    .depth;
                entry.insert(depth);
            }
        }
        Ok(topics
            .into_iter()
            .map(|(name, count)| TopicInfo { name, depth: count })
            .collect())
    }

    async fn shutdown(&self) {
        let mut subs = self.subscriptions.write().await;
        for (_, sub) in subs.drain() {
            let _ = self
                .channel
                .basic_cancel(&sub.consumer_tag, BasicCancelOptions::default())
                .await;
            sub.task_handle.abort();
        }
        drop(subs);

        let tasks = self
            .function_consumer_tasks
            .write()
            .await
            .drain(..)
            .collect::<Vec<_>>();
        for handle in &tasks {
            if handle
                .cancelled
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let _ = self
                    .channel
                    .basic_cancel(&handle.consumer_tag, BasicCancelOptions::default())
                    .await;
            }
            handle.task.abort();
        }
        for handle in tasks {
            let _ = handle.task.await;
        }
        if let Err(err) = requeue_rabbitmq_deliveries(&self.delivery_map, None).await {
            tracing::error!(error = %err, "failed to requeue RabbitMQ function queue deliveries during shutdown");
        }
    }
}

async fn requeue_rabbitmq_deliveries(
    delivery_map: &Arc<RwLock<HashMap<u64, Arc<FunctionQueueDelivery>>>>,
    consumer_id: Option<u64>,
) -> anyhow::Result<()> {
    let deliveries = {
        let active = delivery_map.read().await;
        active
            .values()
            .filter(|delivery| {
                consumer_id.is_none_or(|consumer_id| delivery.consumer_id == consumer_id)
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    let mut errors = Vec::new();
    for delivery in deliveries {
        let _operation = delivery.operation.lock().await;
        if delivery.settled.load(Ordering::Acquire) {
            delivery_map.write().await.remove(&delivery.delivery_id);
            continue;
        }
        if let Err(err) = delivery
            .delivery
            .nack(BasicNackOptions {
                requeue: true,
                ..Default::default()
            })
            .await
        {
            tracing::error!(queue = %delivery.queue_name, error = %err, "failed to requeue unacknowledged RabbitMQ function queue delivery");
            errors.push(err.to_string());
            continue;
        }
        delivery.settled.store(true, Ordering::Release);
        delivery_map.write().await.remove(&delivery.delivery_id);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("; "))
    }
}
