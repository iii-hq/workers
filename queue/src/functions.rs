//! Queue and DLQ service function registration.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{QueueAdapter, SwappableAdapter};
use crate::store::Job;

pub const PUBLISH_FN_ID: &str = "iii::durable::publish";
pub const REDRIVE_FN_ID: &str = "iii::queue::redrive";
pub const REDRIVE_MESSAGE_FN_ID: &str = "iii::queue::redrive_message";
pub const DISCARD_MESSAGE_FN_ID: &str = "iii::queue::discard_message";
pub const LIST_TOPICS_FN_ID: &str = "engine::queue::list_topics";
pub const TOPIC_STATS_FN_ID: &str = "engine::queue::topic_stats";
pub const DLQ_TOPICS_FN_ID: &str = "engine::queue::dlq_topics";
pub const DLQ_MESSAGES_FN_ID: &str = "engine::queue::dlq_messages";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PublishInput {
    /// Topic to publish to. `queue` is accepted for the migration worker API.
    #[serde(alias = "queue")]
    pub topic: String,
    pub data: Value,
}

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

pub fn register_all(iii: &Arc<IIIClient>, adapter: Arc<SwappableAdapter>) {
    let publish_adapter = adapter.clone();
    iii.register_function(
        PUBLISH_FN_ID,
        RegisterFunction::new_async(move |input: PublishInput| {
            let adapter = publish_adapter.clone();
            async move { publish(adapter, input).await }
        })
        .description("Enqueue a message"),
    );

    let redrive_adapter = adapter.clone();
    iii.register_function(
        REDRIVE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveInput| {
            let adapter = redrive_adapter.clone();
            async move { redrive(adapter, input).await }
        })
        .description("Redrive all DLQ messages back to the main queue"),
    );

    let redrive_message_adapter = adapter.clone();
    iii.register_function(
        REDRIVE_MESSAGE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveSingleInput| {
            let adapter = redrive_message_adapter.clone();
            async move { redrive_message(adapter, input).await }
        })
        .description("Redrive a single DLQ message by ID back to the main queue"),
    );

    let discard_adapter = adapter.clone();
    iii.register_function(
        DISCARD_MESSAGE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveSingleInput| {
            let adapter = discard_adapter.clone();
            async move { discard_message(adapter, input).await }
        })
        .description("Discard (purge) a single DLQ message by ID"),
    );

    let list_adapter = adapter.clone();
    iii.register_function(
        LIST_TOPICS_FN_ID,
        RegisterFunction::new_async(move |_input: ListTopicsInput| {
            let adapter = list_adapter.clone();
            async move { list_topics(adapter).await }
        })
        .description("List all queue topics"),
    );

    let stats_adapter = adapter.clone();
    iii.register_function(
        TOPIC_STATS_FN_ID,
        RegisterFunction::new_async(move |input: TopicStatsInput| {
            let adapter = stats_adapter.clone();
            async move { topic_stats(adapter, input).await }
        })
        .description("Get stats for a queue topic"),
    );

    let dlq_topics_adapter = adapter.clone();
    iii.register_function(
        DLQ_TOPICS_FN_ID,
        RegisterFunction::new_async(move |_input: DlqTopicsInput| {
            let adapter = dlq_topics_adapter.clone();
            async move { dlq_topics(adapter).await }
        })
        .description("List DLQ topics with counts"),
    );

    let dlq_messages_adapter = adapter;
    iii.register_function(
        DLQ_MESSAGES_FN_ID,
        RegisterFunction::new_async(move |input: DlqMessagesInput| {
            let adapter = dlq_messages_adapter.clone();
            async move { dlq_messages(adapter, input).await }
        })
        .description("Browse DLQ messages"),
    );
}

pub async fn publish(
    adapter: Arc<SwappableAdapter>,
    input: PublishInput,
) -> Result<Option<PublishAck>, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("Topic is not set".to_string()));
    }
    adapter.enqueue(&input.topic, input.data, None, None).await;
    // Always `None` → `null` on the wire, matching the engine builtin's
    // `Success(None)`. The `PublishAck` type only exists to keep the
    // registered response schema typed.
    Ok(None)
}

pub async fn redrive(
    adapter: Arc<SwappableAdapter>,
    input: RedriveInput,
) -> Result<RedriveResult, Error> {
    if input.queue.is_empty() {
        return Err(Error::Handler("Queue name is required".to_string()));
    }
    let redriven = adapter
        .redrive_dlq(&input.queue)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(RedriveResult {
        queue: input.queue,
        redriven,
    })
}

pub async fn redrive_message(
    adapter: Arc<SwappableAdapter>,
    input: RedriveSingleInput,
) -> Result<RedriveSingleResult, Error> {
    validate_single_input(&input)?;
    let found = adapter
        .redrive_dlq_message(&input.queue, &input.message_id)
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
    input: RedriveSingleInput,
) -> Result<RedriveSingleResult, Error> {
    validate_single_input(&input)?;
    let found = adapter
        .discard_dlq_message(&input.queue, &input.message_id)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(RedriveSingleResult {
        queue: input.queue,
        message_id: input.message_id,
        redriven: u64::from(found),
    })
}

pub async fn list_topics(adapter: Arc<SwappableAdapter>) -> Result<Vec<TopicInfo>, Error> {
    let broker_type = adapter.current_name().await;
    let topics = adapter
        .list_topics()
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(topics
        .into_iter()
        .map(|topic| TopicInfo {
            name: topic.name,
            broker_type: broker_type.clone(),
            subscriber_count: 0,
        })
        .collect())
}

pub async fn topic_stats(
    adapter: Arc<SwappableAdapter>,
    input: TopicStatsInput,
) -> Result<TopicStatsOutput, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("topic is required".to_string()));
    }
    let stats = adapter
        .topic_stats(&input.topic)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(TopicStatsOutput {
        depth: stats.depth,
        consumer_count: 0,
        dlq_depth: stats.dlq_depth,
        config: None,
    })
}

/// Ported from the engine builtin's `console_dlq_topics`
/// (`engine/src/workers/queue/queue.rs:524-560`): iterate every known topic
/// and pair it with its DLQ depth, keeping only topics that actually have
/// dead-lettered messages. A per-topic `dlq_count` failure is treated as
/// zero (matching the engine's `unwrap_or(0)`) rather than failing the
/// whole call.
pub async fn dlq_topics(adapter: Arc<SwappableAdapter>) -> Result<Vec<DlqTopicInfo>, Error> {
    let broker_type = adapter.current_name().await;
    let topics = adapter
        .list_topics()
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;

    let mut dlq_topics = Vec::new();
    for topic in topics {
        let dlq_count = adapter.dlq_count(&topic.name).await.unwrap_or(0);
        if dlq_count > 0 {
            dlq_topics.push(DlqTopicInfo {
                topic: topic.name,
                broker_type: broker_type.clone(),
                message_count: dlq_count,
            });
        }
    }
    Ok(dlq_topics)
}

pub async fn dlq_messages(
    adapter: Arc<SwappableAdapter>,
    input: DlqMessagesInput,
) -> Result<Vec<DlqMessage>, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("topic is required".to_string()));
    }
    let count = input.offset.saturating_add(input.limit) as usize;
    let values = adapter
        .dlq_messages(&input.topic, count)
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
        Enqueue { topic: String, data: Value },
        RedriveDlq { topic: String },
        RedriveDlqMessage { topic: String, message_id: String },
        DiscardDlqMessage { topic: String, message_id: String },
        DlqCount { topic: String },
        ListTopics,
        TopicStats { topic: String },
        DlqMessages { topic: String, count: usize },
    }

    /// Records every call and lets a test script canned return values (an
    /// `Err` here is how a real adapter surfaces a transport-specific
    /// failure, e.g. redis's "does not support DLQ" -- functions must
    /// propagate it 1:1).
    #[derive(Default)]
    struct MockAdapter {
        calls: StdMutex<Vec<Call>>,
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
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) {
            self.calls.lock().unwrap().push(Call::Enqueue {
                topic: topic.to_string(),
                data,
            });
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

    fn job_value(id: &str, payload: Value, attempts: u32) -> Value {
        serde_json::to_value(Job {
            id: id.to_string(),
            payload,
            attempts,
            enqueued_at_ms: 0,
            ready_at_ms: 0,
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
            }]
        );
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
    async fn redrive_returns_adapter_count() {
        let (adapter, mock) = adapter();
        *mock.redrive_dlq_result.lock().unwrap() = Some(Ok(3));
        let result = redrive(
            adapter,
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
    async fn redrive_propagates_adapter_error() {
        let (adapter, mock) = adapter();
        *mock.redrive_dlq_result.lock().unwrap() =
            Some(Err(anyhow::anyhow!("redis does not support DLQ")));
        let err = redrive(
            adapter,
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
        let (adapter, _mock) = adapter();
        {
            let mock = _mock.clone();
            *mock.list_topics_result.lock().unwrap() = Some(Ok(vec![AdapterTopicInfo {
                name: "demo".to_string(),
                depth: 1,
            }]));
        }
        let topics = list_topics(adapter.clone()).await.unwrap();
        assert_eq!(
            topics[0],
            TopicInfo {
                name: "demo".to_string(),
                broker_type: "mock".to_string(),
                subscriber_count: 0,
            }
        );

        *_mock.topic_stats_result.lock().unwrap() = Some(Ok(TopicStats {
            depth: 1,
            dlq_depth: 1,
            delivered: 0,
            failed: 1,
        }));
        let stats = topic_stats(
            adapter,
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
        let result = dlq_topics(adapter.clone()).await.unwrap();
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
