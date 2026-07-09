//! Queue and DLQ service function registration.

use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{FunctionQueueConfig, QueueAdapter, SwappableAdapter};
use crate::boot::{ApplyLock, ConfigCell};
use crate::function_queue_id::function_queue_adapter_key;
use crate::function_queues::FunctionQueueRuntime;
use crate::store::Job;

pub const PUBLISH_FN_ID: &str = "iii::durable::publish";
pub const REDRIVE_FN_ID: &str = "iii::queue::redrive";
pub const REDRIVE_MESSAGE_FN_ID: &str = "iii::queue::redrive_message";
pub const DISCARD_MESSAGE_FN_ID: &str = "iii::queue::discard_message";
pub const LIST_TOPICS_FN_ID: &str = "engine::queue::list_topics";
pub const TOPIC_STATS_FN_ID: &str = "engine::queue::topic_stats";
pub const DLQ_TOPICS_FN_ID: &str = "engine::queue::dlq_topics";
pub const DLQ_MESSAGES_FN_ID: &str = "engine::queue::dlq_messages";
/// Idempotently creates a named function queue owned by this worker.
pub const ENSURE_FUNCTION_QUEUE_FN_ID: &str = "engine::queue::ensure";
/// Internal provider invoked by the engine for `TriggerAction::Enqueue`.
pub const ENQUEUE_FUNCTION_QUEUE_FN_ID: &str = "engine::queue::enqueue";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PublishInput {
    /// Topic to publish to. `queue` is accepted for the migration worker API.
    #[serde(alias = "queue")]
    pub topic: String,
    pub data: Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnsureFunctionQueueInput {
    pub function_id: String,
    pub config: FunctionQueueConfig,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct EnsureFunctionQueueResult {
    pub function_id: String,
    pub queue: String,
    pub changed: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnqueueFunctionInput {
    pub queue: String,
    pub function_id: String,
    pub data: Value,
    #[serde(alias = "messageReceiptId")]
    pub message_receipt_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct EnqueueFunctionResult {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RedriveInput {
    #[serde(alias = "topic")]
    pub queue: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RedriveResult {
    pub queue: String,
    pub redriven: u64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RedriveSingleInput {
    #[serde(alias = "topic")]
    pub queue: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RedriveSingleResult {
    pub queue: String,
    pub message_id: String,
    pub redriven: u64,
}

/// Acknowledgement type for `iii::durable::publish`. The function always
/// returns `null` on the wire (engine parity — the builtin returns
/// `Success(None)`); this type exists so the registered response schema is
/// typed instead of `AnyValue`, which the registry publish gate rejects.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct PublishAck {}

/// Input for `engine::queue::list_topics`. The function takes no
/// parameters; any provided fields are ignored (engine parity). The empty
/// struct keeps the registered request schema typed for the registry
/// publish gate.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListTopicsInput {}

/// Input for `engine::queue::dlq_topics` — same story as
/// [`ListTopicsInput`].
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct DlqTopicsInput {}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TopicInfo {
    pub name: String,
    pub broker_type: String,
    pub subscriber_count: u64,
    pub function_id: Option<String>,
    pub healthy: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TopicStatsInput {
    #[serde(alias = "queue")]
    pub topic: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct TopicStatsOutput {
    pub depth: u64,
    pub consumer_count: u64,
    pub dlq_depth: u64,
    pub config: Option<Value>,
    pub function_id: Option<String>,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DlqTopicInfo {
    pub topic: String,
    pub broker_type: String,
    pub message_count: u64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DlqMessagesInput {
    #[serde(alias = "queue")]
    pub topic: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default = "default_dlq_limit")]
    pub limit: u64,
}

fn default_dlq_limit() -> u64 {
    50
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct DlqMessage {
    pub id: String,
    pub payload: Value,
    pub error: String,
    pub failed_at: u64,
    pub retries: u32,
    pub size_bytes: u64,
}

pub fn register_all(
    iii: &Arc<IIIClient>,
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    apply_lock: ApplyLock,
    function_queues: Arc<FunctionQueueRuntime>,
) {
    let publish_adapter = adapter.clone();
    iii.register_function(
        PUBLISH_FN_ID,
        RegisterFunction::new_async(move |input: PublishInput| {
            let adapter = publish_adapter.clone();
            async move { publish(adapter, input).await }
        })
        .description("Enqueue a message"),
    );

    let ensure_iii = iii.clone();
    let ensure_config = config.clone();
    let ensure_apply_lock = apply_lock.clone();
    let ensure_runtime = function_queues.clone();
    iii.register_function(
        ENSURE_FUNCTION_QUEUE_FN_ID,
        RegisterFunction::new_async(move |input: EnsureFunctionQueueInput| {
            let iii = ensure_iii.clone();
            let config = ensure_config.clone();
            let apply_lock = ensure_apply_lock.clone();
            let runtime = ensure_runtime.clone();
            async move { ensure_function_queue(iii, config, apply_lock, runtime, input).await }
        })
        .description("Idempotently provision a named function queue"),
    );

    let enqueue_runtime = function_queues.clone();
    iii.register_function(
        ENQUEUE_FUNCTION_QUEUE_FN_ID,
        RegisterFunction::new_async(move |input: EnqueueFunctionInput| {
            let runtime = enqueue_runtime.clone();
            async move { enqueue_function_queue(runtime, input).await }
        })
        .description("Internal engine enqueue provider; persists a named function queue job"),
    );

    let redrive_adapter = adapter.clone();
    let redrive_config = config.clone();
    iii.register_function(
        REDRIVE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveInput| {
            let adapter = redrive_adapter.clone();
            let config = redrive_config.clone();
            async move { redrive(adapter, config, input).await }
        })
        .description("Redrive all DLQ messages back to the main queue"),
    );

    let redrive_message_adapter = adapter.clone();
    let redrive_message_config = config.clone();
    iii.register_function(
        REDRIVE_MESSAGE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveSingleInput| {
            let adapter = redrive_message_adapter.clone();
            let config = redrive_message_config.clone();
            async move { redrive_message(adapter, config, input).await }
        })
        .description("Redrive a single DLQ message by ID back to the main queue"),
    );

    let discard_adapter = adapter.clone();
    let discard_config = config.clone();
    iii.register_function(
        DISCARD_MESSAGE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveSingleInput| {
            let adapter = discard_adapter.clone();
            let config = discard_config.clone();
            async move { discard_message(adapter, config, input).await }
        })
        .description("Discard (purge) a single DLQ message by ID"),
    );

    let list_adapter = adapter.clone();
    let list_config = config.clone();
    let list_runtime = function_queues.clone();
    iii.register_function(
        LIST_TOPICS_FN_ID,
        RegisterFunction::new_async(move |_input: ListTopicsInput| {
            let adapter = list_adapter.clone();
            let config = list_config.clone();
            let runtime = list_runtime.clone();
            async move { list_topics(adapter, config, runtime).await }
        })
        .description("List all queue topics"),
    );

    let stats_adapter = adapter.clone();
    let stats_config = config.clone();
    let stats_runtime = function_queues.clone();
    iii.register_function(
        TOPIC_STATS_FN_ID,
        RegisterFunction::new_async(move |input: TopicStatsInput| {
            let adapter = stats_adapter.clone();
            let config = stats_config.clone();
            let runtime = stats_runtime.clone();
            async move { topic_stats(adapter, config, runtime, input).await }
        })
        .description("Get stats for a queue topic"),
    );

    let dlq_topics_adapter = adapter.clone();
    let dlq_topics_config = config.clone();
    iii.register_function(
        DLQ_TOPICS_FN_ID,
        RegisterFunction::new_async(move |_input: DlqTopicsInput| {
            let adapter = dlq_topics_adapter.clone();
            let config = dlq_topics_config.clone();
            async move { dlq_topics(adapter, config).await }
        })
        .description("List DLQ topics with counts"),
    );

    let dlq_messages_adapter = adapter;
    let dlq_messages_config = config;
    iii.register_function(
        DLQ_MESSAGES_FN_ID,
        RegisterFunction::new_async(move |input: DlqMessagesInput| {
            let adapter = dlq_messages_adapter.clone();
            let config = dlq_messages_config.clone();
            async move { dlq_messages(adapter, config, input).await }
        })
        .description("Browse DLQ messages"),
    );
}

async fn ensure_function_queue(
    iii: Arc<IIIClient>,
    config_cell: ConfigCell,
    apply_lock: ApplyLock,
    runtime: Arc<FunctionQueueRuntime>,
    input: EnsureFunctionQueueInput,
) -> Result<EnsureFunctionQueueResult, Error> {
    if input.function_id.trim().is_empty() {
        return Err(Error::Handler("function_id is required".to_string()));
    }
    input
        .config
        .validate()
        .map_err(|error| Error::Handler(error.to_string()))?;

    let _apply = apply_lock.lock().await;
    let current = config_cell.read().await.as_ref().clone();
    if let Some(existing) = current.queue_configs.get(&input.function_id) {
        if existing == &input.config {
            let healthy = runtime
                .status(&input.function_id)
                .await
                .is_some_and(|status| status.healthy);
            if !healthy {
                runtime
                    .reconcile(&current.queue_configs)
                    .await
                    .map_err(|error| {
                        Error::Handler(format!(
                            "could not restart function queue '{}': {error}",
                            input.function_id
                        ))
                    })?;
            }
            return Ok(EnsureFunctionQueueResult {
                function_id: input.function_id.clone(),
                queue: input.function_id,
                changed: !healthy,
            });
        }
    }

    let mut next = current.clone();
    next.queue_configs
        .insert(input.function_id.clone(), input.config.clone());
    persist_config(&iii, &next).await?;
    if let Err(error) = runtime.reconcile(&next.queue_configs).await {
        let _ = persist_config(&iii, &current).await;
        return Err(Error::Handler(format!(
            "could not start function queue '{}': {error}",
            input.function_id
        )));
    }
    *config_cell.write().await = Arc::new(next);
    Ok(EnsureFunctionQueueResult {
        function_id: input.function_id.clone(),
        queue: input.function_id,
        changed: true,
    })
}

async fn persist_config(iii: &IIIClient, config: &crate::config::QueueConfig) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".to_string(),
        payload: serde_json::json!({ "id": crate::configuration::CONFIG_ID, "value": config }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .map(|_| ())
    .map_err(|error| Error::Handler(format!("persisting queue configuration: {error}")))
}

pub async fn enqueue_function_queue(
    runtime: Arc<FunctionQueueRuntime>,
    input: EnqueueFunctionInput,
) -> Result<Option<EnqueueFunctionResult>, Error> {
    if input.queue.trim().is_empty() {
        return Err(Error::Handler("queue is required".to_string()));
    }
    if input.function_id.trim().is_empty() {
        return Err(Error::Handler("function_id is required".to_string()));
    }
    if input.queue != input.function_id {
        return Err(Error::Handler(format!(
            "standalone function queues require queue to equal function_id: queue='{}', function_id='{}'",
            input.queue, input.function_id
        )));
    }
    if input.message_receipt_id.trim().is_empty() {
        return Err(Error::Handler("messageReceiptId is required".to_string()));
    }
    let traceparent = iii_helpers::observability::inject_traceparent();
    let baggage = iii_helpers::observability::inject_baggage();
    runtime
        .enqueue(
            &input.function_id,
            input.data,
            &input.message_receipt_id,
            traceparent,
            baggage,
        )
        .await
        .map_err(|error| Error::Handler(error.to_string()))?;
    Ok(None)
}

pub async fn publish(
    adapter: Arc<SwappableAdapter>,
    input: PublishInput,
) -> Result<Option<PublishAck>, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("Topic is not set".to_string()));
    }
    let traceparent = iii_helpers::observability::inject_traceparent();
    let baggage = iii_helpers::observability::inject_baggage();
    adapter
        .enqueue(&input.topic, input.data, traceparent, baggage)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    // Always `None` → `null` on the wire, matching the engine builtin's
    // `Success(None)`. The `PublishAck` type only exists to keep the
    // registered response schema typed.
    Ok(None)
}

pub async fn redrive(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    input: RedriveInput,
) -> Result<RedriveResult, Error> {
    if input.queue.is_empty() {
        return Err(Error::Handler("Queue name is required".to_string()));
    }
    let storage_topic = resolve_storage_topic(&config, &input.queue).await;
    let redriven = adapter
        .redrive_dlq(&storage_topic)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(RedriveResult {
        queue: input.queue,
        redriven,
    })
}

pub async fn redrive_message(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    input: RedriveSingleInput,
) -> Result<RedriveSingleResult, Error> {
    validate_single_input(&input)?;
    let storage_topic = resolve_storage_topic(&config, &input.queue).await;
    let found = adapter
        .redrive_dlq_message(&storage_topic, &input.message_id)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(RedriveSingleResult {
        queue: input.queue,
        message_id: input.message_id,
        redriven: u64::from(found),
    })
}

pub async fn discard_message(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    input: RedriveSingleInput,
) -> Result<RedriveSingleResult, Error> {
    validate_single_input(&input)?;
    let storage_topic = resolve_storage_topic(&config, &input.queue).await;
    let found = adapter
        .discard_dlq_message(&storage_topic, &input.message_id)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(RedriveSingleResult {
        queue: input.queue,
        message_id: input.message_id,
        redriven: u64::from(found),
    })
}

pub async fn list_topics(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    runtime: Arc<FunctionQueueRuntime>,
) -> Result<Vec<TopicInfo>, Error> {
    let broker_type = adapter.current_name().await;
    let topics = adapter
        .list_topics()
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    let mut listed = topics
        .into_iter()
        .filter(|topic| !topic.name.starts_with("__fn_queue::"))
        .map(|topic| TopicInfo {
            name: topic.name,
            broker_type: broker_type.clone(),
            subscriber_count: 0,
            function_id: None,
            healthy: true,
        })
        .collect::<Vec<_>>();
    let configs = config.read().await.queue_configs.clone();
    let statuses = runtime
        .statuses()
        .await
        .into_iter()
        .map(|status| (status.function_id.clone(), status))
        .collect::<HashMap<_, _>>();
    for (name, queue_config) in configs {
        let status = statuses.get(&name);
        if let Some(topic) = listed.iter_mut().find(|topic| topic.name == name) {
            topic.function_id = Some(name.clone());
            topic.subscriber_count = status.map_or(u64::from(queue_config.concurrency), |status| {
                u64::from(status.consumer_count)
            });
            topic.healthy = status.is_some_and(|status| status.healthy);
        } else {
            listed.push(TopicInfo {
                name: name.clone(),
                broker_type: broker_type.clone(),
                subscriber_count: status.map_or(u64::from(queue_config.concurrency), |status| {
                    u64::from(status.consumer_count)
                }),
                function_id: Some(name),
                healthy: status.is_some_and(|status| status.healthy),
            });
        }
    }
    listed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(listed)
}

pub async fn topic_stats(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    runtime: Arc<FunctionQueueRuntime>,
    input: TopicStatsInput,
) -> Result<TopicStatsOutput, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("topic is required".to_string()));
    }
    let queue_config = config.read().await.queue_configs.get(&input.topic).cloned();
    let storage_topic = resolve_storage_topic(&config, &input.topic).await;
    let stats = adapter
        .topic_stats(&storage_topic)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    let status = runtime.status(&input.topic).await;
    Ok(TopicStatsOutput {
        depth: stats.depth,
        consumer_count: queue_config.as_ref().map_or(0, |cfg| {
            status
                .as_ref()
                .map_or(u64::from(cfg.concurrency), |status| {
                    u64::from(status.consumer_count)
                })
        }),
        dlq_depth: stats.dlq_depth,
        config: queue_config
            .as_ref()
            .and_then(|cfg| serde_json::to_value(cfg).ok()),
        function_id: queue_config.as_ref().map(|_| input.topic.clone()),
        healthy: queue_config
            .as_ref()
            .is_none_or(|_| status.is_some_and(|status| status.healthy)),
    })
}

/// Ported from the engine builtin's `console_dlq_topics`
/// (`engine/src/workers/queue/queue.rs:524-560`): iterate every known topic
/// and pair it with its DLQ depth, keeping only topics that actually have
/// dead-lettered messages. A per-topic `dlq_count` failure is treated as
/// zero (matching the engine's `unwrap_or(0)`) rather than failing the
/// whole call.
pub async fn dlq_topics(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
) -> Result<Vec<DlqTopicInfo>, Error> {
    let broker_type = adapter.current_name().await;
    let configs = config.read().await.queue_configs.clone();
    let topics = adapter
        .list_topics()
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;

    let mut dlq_topics = Vec::new();
    for topic in topics {
        if topic.name.starts_with("__fn_queue::") {
            continue;
        }
        let dlq_count = adapter.dlq_count(&topic.name).await.unwrap_or(0);
        if dlq_count > 0 {
            dlq_topics.push(DlqTopicInfo {
                topic: topic.name,
                broker_type: broker_type.clone(),
                message_count: dlq_count,
            });
        }
    }
    for function_id in configs.keys() {
        let storage_topic = function_queue_adapter_key(function_id);
        let dlq_count = adapter.dlq_count(&storage_topic).await.unwrap_or(0);
        if dlq_count > 0 {
            dlq_topics.push(DlqTopicInfo {
                topic: function_id.clone(),
                broker_type: broker_type.clone(),
                message_count: dlq_count,
            });
        }
    }
    dlq_topics.sort_by(|left, right| left.topic.cmp(&right.topic));
    Ok(dlq_topics)
}

pub async fn dlq_messages(
    adapter: Arc<SwappableAdapter>,
    config: ConfigCell,
    input: DlqMessagesInput,
) -> Result<Vec<DlqMessage>, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("topic is required".to_string()));
    }
    let count = input.offset.saturating_add(input.limit) as usize;
    let storage_topic = resolve_storage_topic(&config, &input.topic).await;
    let values = adapter
        .dlq_messages(&storage_topic, count)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    let messages = values
        .into_iter()
        .skip(input.offset as usize)
        .take(input.limit as usize)
        .filter_map(|value| serde_json::from_value::<Job>(value).ok())
        .map(dlq_message_from_job)
        .collect();
    Ok(messages)
}

fn validate_single_input(input: &RedriveSingleInput) -> Result<(), Error> {
    if input.queue.is_empty() {
        return Err(Error::Handler("Queue name is required".to_string()));
    }
    if input.message_id.is_empty() {
        return Err(Error::Handler("Message ID is required".to_string()));
    }
    Ok(())
}

async fn resolve_storage_topic(config: &ConfigCell, logical_topic: &str) -> String {
    let config = config.read().await;
    if config.queue_configs.contains_key(logical_topic) {
        function_queue_adapter_key(logical_topic)
    } else {
        logical_topic.to_string()
    }
}

fn dlq_message_from_job(job: Job) -> DlqMessage {
    let size_bytes = serde_json::to_vec(&job.payload).map_or(0, |bytes| bytes.len() as u64);
    DlqMessage {
        id: job.id,
        payload: job.payload,
        error: "function call failed".to_string(),
        failed_at: job.enqueued_at_ms,
        retries: job.attempts,
        size_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::TopicInfo as AdapterTopicInfo;
    use crate::store::TopicStats;
    use crate::subscriber_config::SubscriberQueueConfig;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Enqueue {
            topic: String,
            data: Value,
            traceparent: Option<String>,
            baggage: Option<String>,
        },
        RedriveDlq {
            topic: String,
        },
        RedriveDlqMessage {
            topic: String,
            message_id: String,
        },
        DiscardDlqMessage {
            topic: String,
            message_id: String,
        },
        DlqCount {
            topic: String,
        },
        ListTopics,
        TopicStats {
            topic: String,
        },
        DlqMessages {
            topic: String,
            count: usize,
        },
    }

    /// Records every call and lets a test script canned return values (an
    /// `Err` here is how a real adapter surfaces a transport-specific
    /// failure, e.g. redis's "does not support DLQ" -- functions must
    /// propagate it 1:1).
    #[derive(Default)]
    struct MockAdapter {
        calls: StdMutex<Vec<Call>>,
        enqueue_error: StdMutex<Option<anyhow::Error>>,
        redrive_dlq_result: StdMutex<Option<anyhow::Result<u64>>>,
        redrive_dlq_message_result: StdMutex<Option<anyhow::Result<bool>>>,
        discard_dlq_message_result: StdMutex<Option<anyhow::Result<bool>>>,
        dlq_count_result: StdMutex<Option<anyhow::Result<u64>>>,
        list_topics_result: StdMutex<Option<anyhow::Result<Vec<AdapterTopicInfo>>>>,
        topic_stats_result: StdMutex<Option<anyhow::Result<TopicStats>>>,
        dlq_messages_result: StdMutex<Option<anyhow::Result<Vec<Value>>>>,
    }

    impl MockAdapter {
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl QueueAdapter for MockAdapter {
        async fn enqueue(
            &self,
            topic: &str,
            data: Value,
            traceparent: Option<String>,
            baggage: Option<String>,
        ) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(Call::Enqueue {
                topic: topic.to_string(),
                data,
                traceparent,
                baggage,
            });
            if let Some(error) = self.enqueue_error.lock().unwrap().take() {
                return Err(error);
            }
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

        async fn redrive_dlq(&self, topic: &str) -> anyhow::Result<u64> {
            self.calls.lock().unwrap().push(Call::RedriveDlq {
                topic: topic.to_string(),
            });
            self.redrive_dlq_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(0))
        }

        async fn redrive_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
            self.calls.lock().unwrap().push(Call::RedriveDlqMessage {
                topic: topic.to_string(),
                message_id: message_id.to_string(),
            });
            self.redrive_dlq_message_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(false))
        }

        async fn discard_dlq_message(&self, topic: &str, message_id: &str) -> anyhow::Result<bool> {
            self.calls.lock().unwrap().push(Call::DiscardDlqMessage {
                topic: topic.to_string(),
                message_id: message_id.to_string(),
            });
            self.discard_dlq_message_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(false))
        }

        async fn dlq_count(&self, topic: &str) -> anyhow::Result<u64> {
            self.calls.lock().unwrap().push(Call::DlqCount {
                topic: topic.to_string(),
            });
            self.dlq_count_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(0))
        }

        async fn dlq_messages(&self, topic: &str, count: usize) -> anyhow::Result<Vec<Value>> {
            self.calls.lock().unwrap().push(Call::DlqMessages {
                topic: topic.to_string(),
                count,
            });
            self.dlq_messages_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(vec![]))
        }

        async fn list_topics(&self) -> anyhow::Result<Vec<AdapterTopicInfo>> {
            self.calls.lock().unwrap().push(Call::ListTopics);
            self.list_topics_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(vec![]))
        }

        async fn topic_stats(&self, topic: &str) -> anyhow::Result<TopicStats> {
            self.calls.lock().unwrap().push(Call::TopicStats {
                topic: topic.to_string(),
            });
            self.topic_stats_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(TopicStats::default()))
        }

        async fn shutdown(&self) {}
    }

    fn adapter() -> (Arc<SwappableAdapter>, Arc<MockAdapter>) {
        let mock = Arc::new(MockAdapter::default());
        let dyn_adapter: Arc<dyn QueueAdapter> = mock.clone();
        (Arc::new(SwappableAdapter::new(dyn_adapter, "mock")), mock)
    }

    fn config() -> crate::boot::ConfigCell {
        crate::configuration::new_cell(crate::config::QueueConfig::default())
    }

    fn function_config(function_id: &str) -> crate::boot::ConfigCell {
        let mut queue_config = crate::config::QueueConfig::default();
        queue_config
            .queue_configs
            .insert(function_id.to_string(), FunctionQueueConfig::default());
        crate::configuration::new_cell(queue_config)
    }

    struct NoopInvoker;

    #[async_trait::async_trait]
    impl crate::trigger::Invoker for NoopInvoker {
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

    fn runtime(adapter: &Arc<SwappableAdapter>) -> Arc<FunctionQueueRuntime> {
        Arc::new(FunctionQueueRuntime::new(
            adapter.clone(),
            Arc::new(NoopInvoker),
        ))
    }

    fn job_value(id: &str, payload: Value, attempts: u32) -> Value {
        serde_json::to_value(Job {
            id: id.to_string(),
            payload,
            attempts,
            enqueued_at_ms: 0,
            ready_at_ms: 0,
            traceparent: None,
            baggage: None,
            function_id: None,
            message_id: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn publish_calls_adapter_enqueue() {
        let (adapter, mock) = adapter();
        publish(
            adapter,
            PublishInput {
                topic: "demo".to_string(),
                data: json!({"hello": "world"}),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            mock.calls(),
            vec![Call::Enqueue {
                topic: "demo".to_string(),
                data: json!({"hello": "world"}),
                traceparent: None,
                baggage: None,
            }]
        );
    }

    #[tokio::test]
    async fn publish_captures_current_trace_context() {
        use iii_helpers::observability::opentelemetry::trace::FutureExt as OtelFutureExt;

        let (adapter, mock) = adapter();
        let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let context =
            iii_helpers::observability::extract_context(Some(traceparent), Some("tenant=motia"));

        publish(
            adapter,
            PublishInput {
                topic: "demo".to_string(),
                data: json!({"hello": "world"}),
            },
        )
        .with_context(context)
        .await
        .unwrap();

        assert_eq!(
            mock.calls(),
            vec![Call::Enqueue {
                topic: "demo".to_string(),
                data: json!({"hello": "world"}),
                traceparent: Some(traceparent.to_string()),
                baggage: Some("tenant=motia".to_string()),
            }]
        );
    }

    #[tokio::test]
    async fn publish_propagates_adapter_enqueue_failure() {
        let (adapter, mock) = adapter();
        *mock.enqueue_error.lock().unwrap() = Some(anyhow::anyhow!("transport unavailable"));

        let error = publish(
            adapter,
            PublishInput {
                topic: "demo".to_string(),
                data: json!({"hello": "world"}),
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("transport unavailable"));
    }

    #[tokio::test]
    async fn publish_rejects_empty_topic() {
        let (adapter, mock) = adapter();
        let err = publish(
            adapter,
            PublishInput {
                topic: String::new(),
                data: Value::Null,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Topic"));
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn enqueue_rejects_mismatched_queue_and_function_before_runtime() {
        let (adapter, mock) = adapter();
        let error = enqueue_function_queue(
            runtime(&adapter),
            EnqueueFunctionInput {
                queue: "orders::create".to_string(),
                function_id: "orders::update".to_string(),
                data: json!({"id": 1}),
                message_receipt_id: "receipt-1".to_string(),
            },
        )
        .await
        .expect_err("mismatched function queue should be rejected");
        assert!(error.to_string().contains("queue to equal function_id"));
        assert!(mock.calls().is_empty());
    }

    #[tokio::test]
    async fn redrive_returns_adapter_count() {
        let (adapter, mock) = adapter();
        *mock.redrive_dlq_result.lock().unwrap() = Some(Ok(3));
        let result = redrive(
            adapter,
            config(),
            RedriveInput {
                queue: "demo".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.redriven, 3);
        assert_eq!(
            mock.calls(),
            vec![Call::RedriveDlq {
                topic: "demo".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn function_queue_dlq_operations_use_internal_storage_key() {
        let (adapter, mock) = adapter();
        *mock.redrive_dlq_result.lock().unwrap() = Some(Ok(2));
        let result = redrive(
            adapter,
            function_config("orders::create"),
            RedriveInput {
                queue: "orders::create".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.queue, "orders::create");
        assert_eq!(
            mock.calls(),
            vec![Call::RedriveDlq {
                topic: crate::function_queue_id::function_queue_adapter_key("orders::create")
            }]
        );
    }

    #[tokio::test]
    async fn redrive_propagates_adapter_error() {
        let (adapter, mock) = adapter();
        *mock.redrive_dlq_result.lock().unwrap() =
            Some(Err(anyhow::anyhow!("redis does not support DLQ")));
        let err = redrive(
            adapter,
            config(),
            RedriveInput {
                queue: "demo".to_string(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("does not support DLQ"));
    }

    #[tokio::test]
    async fn redrive_message_moves_one_dlq_message() {
        let (adapter, mock) = adapter();
        *mock.redrive_dlq_message_result.lock().unwrap() = Some(Ok(true));
        let result = redrive_message(
            adapter,
            config(),
            RedriveSingleInput {
                queue: "demo".to_string(),
                message_id: "m1".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.redriven, 1);
        assert_eq!(
            mock.calls(),
            vec![Call::RedriveDlqMessage {
                topic: "demo".to_string(),
                message_id: "m1".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn discard_message_purges_one_dlq_message() {
        let (adapter, mock) = adapter();
        *mock.discard_dlq_message_result.lock().unwrap() = Some(Ok(true));
        let result = discard_message(
            adapter,
            config(),
            RedriveSingleInput {
                queue: "demo".to_string(),
                message_id: "m1".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.redriven, 1);
        assert_eq!(
            mock.calls(),
            vec![Call::DiscardDlqMessage {
                topic: "demo".to_string(),
                message_id: "m1".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn list_and_stats_return_adapter_state() {
        let (adapter, mock) = adapter();
        {
            *mock.list_topics_result.lock().unwrap() = Some(Ok(vec![AdapterTopicInfo {
                name: "demo".to_string(),
                depth: 1,
            }]));
        }
        let topics = list_topics(adapter.clone(), config(), runtime(&adapter))
            .await
            .unwrap();
        assert_eq!(
            topics[0],
            TopicInfo {
                name: "demo".to_string(),
                broker_type: "mock".to_string(),
                subscriber_count: 0,
                function_id: None,
                healthy: true,
            }
        );

        *mock.topic_stats_result.lock().unwrap() = Some(Ok(TopicStats {
            depth: 1,
            dlq_depth: 1,
            delivered: 0,
            failed: 1,
        }));
        let stats = topic_stats(
            adapter.clone(),
            config(),
            runtime(&adapter),
            TopicStatsInput {
                topic: "demo".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(stats.depth, 1);
        assert_eq!(stats.dlq_depth, 1);
    }

    #[tokio::test]
    async fn list_topics_reports_unhealthy_configured_function_queue() {
        let (adapter, mock) = adapter();
        *mock.list_topics_result.lock().unwrap() = Some(Ok(vec![]));
        let topics = list_topics(
            adapter.clone(),
            function_config("orders::create"),
            runtime(&adapter),
        )
        .await
        .unwrap();
        assert_eq!(
            topics,
            vec![TopicInfo {
                name: "orders::create".to_string(),
                broker_type: "mock".to_string(),
                subscriber_count: 10,
                function_id: Some("orders::create".to_string()),
                healthy: false,
            }]
        );
    }

    #[tokio::test]
    async fn topic_stats_reports_configured_function_health_without_leaking_storage_name() {
        let (adapter, mock) = adapter();
        *mock.topic_stats_result.lock().unwrap() = Some(Ok(TopicStats::default()));
        let stats = topic_stats(
            adapter.clone(),
            function_config("orders::create"),
            runtime(&adapter),
            TopicStatsInput {
                topic: "orders::create".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(stats.function_id.as_deref(), Some("orders::create"));
        assert!(!stats.healthy);
        assert_eq!(
            mock.calls(),
            vec![Call::TopicStats {
                topic: crate::function_queue_id::function_queue_adapter_key("orders::create")
            }]
        );
    }

    #[tokio::test]
    async fn dlq_topics_only_includes_topics_with_positive_dlq_depth() {
        let (adapter, mock) = adapter();
        *mock.list_topics_result.lock().unwrap() = Some(Ok(vec![
            AdapterTopicInfo {
                name: "demo".to_string(),
                depth: 0,
            },
            AdapterTopicInfo {
                name: "empty".to_string(),
                depth: 0,
            },
        ]));
        // dlq_count is called once per topic in list order; queue the "demo"
        // answer first, then "empty"'s.
        *mock.dlq_count_result.lock().unwrap() = Some(Ok(1));
        let result = dlq_topics(adapter.clone(), config()).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].topic, "demo");
        assert_eq!(result[0].message_count, 1);
        assert_eq!(result[0].broker_type, "mock");
    }

    #[tokio::test]
    async fn dlq_browse_returns_messages() {
        let (adapter, mock) = adapter();
        *mock.dlq_messages_result.lock().unwrap() =
            Some(Ok(vec![job_value("m1", json!({"dead": true}), 1)]));
        let messages = dlq_messages(
            adapter,
            config(),
            DlqMessagesInput {
                topic: "demo".to_string(),
                offset: 0,
                limit: 50,
            },
        )
        .await
        .unwrap();
        assert_eq!(messages[0].id, "m1");
        assert_eq!(messages[0].retries, 1);
        assert_eq!(
            mock.calls(),
            vec![Call::DlqMessages {
                topic: "demo".to_string(),
                count: 50,
            }]
        );
    }

    #[tokio::test]
    async fn dlq_messages_rejects_empty_topic() {
        let (adapter, _mock) = adapter();
        let err = dlq_messages(
            adapter,
            config(),
            DlqMessagesInput {
                topic: String::new(),
                offset: 0,
                limit: 50,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("topic"));
    }
}
