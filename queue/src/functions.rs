//! Queue and DLQ service function registration.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{Job, QueueStore};

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

pub fn register_all(iii: &Arc<IIIClient>, store: Arc<dyn QueueStore>) {
    let publish_store = store.clone();
    iii.register_function(
        PUBLISH_FN_ID,
        RegisterFunction::new_async(move |input: PublishInput| {
            let store = publish_store.clone();
            async move { publish(store, input).await }
        })
        .description("Enqueue a message"),
    );

    let redrive_store = store.clone();
    iii.register_function(
        REDRIVE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveInput| {
            let store = redrive_store.clone();
            async move { redrive(store, input).await }
        })
        .description("Redrive all DLQ messages back to the main queue"),
    );

    let redrive_message_store = store.clone();
    iii.register_function(
        REDRIVE_MESSAGE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveSingleInput| {
            let store = redrive_message_store.clone();
            async move { redrive_message(store, input).await }
        })
        .description("Redrive a single DLQ message by ID back to the main queue"),
    );

    let discard_store = store.clone();
    iii.register_function(
        DISCARD_MESSAGE_FN_ID,
        RegisterFunction::new_async(move |input: RedriveSingleInput| {
            let store = discard_store.clone();
            async move { discard_message(store, input).await }
        })
        .description("Discard (purge) a single DLQ message by ID"),
    );

    let list_store = store.clone();
    iii.register_function(
        LIST_TOPICS_FN_ID,
        RegisterFunction::new_async(move |_input: Value| {
            let store = list_store.clone();
            async move { list_topics(store).await }
        })
        .description("List all queue topics"),
    );

    let stats_store = store.clone();
    iii.register_function(
        TOPIC_STATS_FN_ID,
        RegisterFunction::new_async(move |input: TopicStatsInput| {
            let store = stats_store.clone();
            async move { topic_stats(store, input).await }
        })
        .description("Get stats for a queue topic"),
    );

    let dlq_topics_store = store.clone();
    iii.register_function(
        DLQ_TOPICS_FN_ID,
        RegisterFunction::new_async(move |_input: Value| {
            let store = dlq_topics_store.clone();
            async move { dlq_topics(store).await }
        })
        .description("List DLQ topics with counts"),
    );

    let dlq_messages_store = store;
    iii.register_function(
        DLQ_MESSAGES_FN_ID,
        RegisterFunction::new_async(move |input: DlqMessagesInput| {
            let store = dlq_messages_store.clone();
            async move { dlq_messages(store, input).await }
        })
        .description("Browse DLQ messages"),
    );
}

pub async fn publish(
    store: Arc<dyn QueueStore>,
    input: PublishInput,
) -> Result<Option<Value>, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("Topic is not set".to_string()));
    }
    store
        .enqueue(&input.topic, input.data)
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    Ok(None)
}

pub async fn redrive(
    store: Arc<dyn QueueStore>,
    input: RedriveInput,
) -> Result<RedriveResult, Error> {
    if input.queue.is_empty() {
        return Err(Error::Handler("Queue name is required".to_string()));
    }
    let redriven = store.redrive_dlq(&input.queue).await;
    Ok(RedriveResult {
        queue: input.queue,
        redriven,
    })
}

pub async fn redrive_message(
    store: Arc<dyn QueueStore>,
    input: RedriveSingleInput,
) -> Result<RedriveSingleResult, Error> {
    validate_single_input(&input)?;
    let found = store
        .redrive_dlq_message(&input.queue, &input.message_id)
        .await;
    Ok(RedriveSingleResult {
        queue: input.queue,
        message_id: input.message_id,
        redriven: u64::from(found),
    })
}

pub async fn discard_message(
    store: Arc<dyn QueueStore>,
    input: RedriveSingleInput,
) -> Result<RedriveSingleResult, Error> {
    validate_single_input(&input)?;
    let found = store
        .discard_dlq_message(&input.queue, &input.message_id)
        .await;
    Ok(RedriveSingleResult {
        queue: input.queue,
        message_id: input.message_id,
        redriven: u64::from(found),
    })
}

pub async fn list_topics(store: Arc<dyn QueueStore>) -> Result<Vec<TopicInfo>, Error> {
    Ok(store
        .list_topics()
        .await
        .into_iter()
        .map(|name| TopicInfo {
            name,
            broker_type: "builtin".to_string(),
            subscriber_count: 0,
        })
        .collect())
}

pub async fn topic_stats(
    store: Arc<dyn QueueStore>,
    input: TopicStatsInput,
) -> Result<TopicStatsOutput, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("topic is required".to_string()));
    }
    let stats = store.topic_stats(&input.topic).await;
    Ok(TopicStatsOutput {
        depth: stats.depth,
        consumer_count: 0,
        dlq_depth: stats.dlq_depth,
        config: None,
    })
}

pub async fn dlq_topics(store: Arc<dyn QueueStore>) -> Result<Vec<DlqTopicInfo>, Error> {
    Ok(store
        .dlq_topics()
        .await
        .into_iter()
        .map(|(topic, message_count)| DlqTopicInfo {
            topic,
            broker_type: "builtin".to_string(),
            message_count,
        })
        .collect())
}

pub async fn dlq_messages(
    store: Arc<dyn QueueStore>,
    input: DlqMessagesInput,
) -> Result<Vec<DlqMessage>, Error> {
    if input.topic.is_empty() {
        return Err(Error::Handler("topic is required".to_string()));
    }
    let messages = store
        .dlq_messages(&input.topic, input.offset.saturating_add(input.limit))
        .await
        .into_iter()
        .skip(input.offset as usize)
        .take(input.limit as usize)
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
    use crate::store::{InMemoryStore, QueueStore};
    use serde_json::json;

    fn store() -> Arc<dyn QueueStore> {
        Arc::new(InMemoryStore::new())
    }

    async fn move_one_to_dlq(store: Arc<dyn QueueStore>, queue: &str) -> String {
        store.enqueue(queue, json!({"dead": true})).await.unwrap();
        let job = store.dequeue(queue).await.unwrap();
        let id = job.id.clone();
        store.nack(queue, job, 1, 1).await;
        id
    }

    #[tokio::test]
    async fn publish_enqueues_message() {
        let store = store();
        publish(
            store.clone(),
            PublishInput {
                topic: "demo".to_string(),
                data: json!({"hello": "world"}),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            store.dequeue("demo").await.unwrap().payload,
            json!({"hello": "world"})
        );
    }

    #[tokio::test]
    async fn redrive_moves_dlq_back() {
        let store = store();
        move_one_to_dlq(store.clone(), "demo").await;
        let result = redrive(
            store.clone(),
            RedriveInput {
                queue: "demo".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.redriven, 1);
        assert!(store.dequeue("demo").await.is_some());
    }

    #[tokio::test]
    async fn redrive_message_moves_one_dlq_message() {
        let store = store();
        let id = move_one_to_dlq(store.clone(), "demo").await;
        let result = redrive_message(
            store.clone(),
            RedriveSingleInput {
                queue: "demo".to_string(),
                message_id: id.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.redriven, 1);
        assert_eq!(store.dequeue("demo").await.unwrap().id, id);
    }

    #[tokio::test]
    async fn discard_message_purges_one_dlq_message() {
        let store = store();
        let id = move_one_to_dlq(store.clone(), "demo").await;
        let result = discard_message(
            store.clone(),
            RedriveSingleInput {
                queue: "demo".to_string(),
                message_id: id,
            },
        )
        .await
        .unwrap();
        assert_eq!(result.redriven, 1);
        assert!(store.dlq_messages("demo", 10).await.is_empty());
    }

    #[tokio::test]
    async fn list_and_stats_return_store_state() {
        let store = store();
        store.enqueue("demo", json!("ready")).await.unwrap();
        move_one_to_dlq(store.clone(), "demo").await;
        assert_eq!(
            list_topics(store.clone()).await.unwrap()[0],
            TopicInfo {
                name: "demo".to_string(),
                broker_type: "builtin".to_string(),
                subscriber_count: 0,
            }
        );
        let stats = topic_stats(
            store.clone(),
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
    async fn dlq_browse_returns_topics_and_messages() {
        let store = store();
        let id = move_one_to_dlq(store.clone(), "demo").await;
        assert_eq!(
            dlq_topics(store.clone()).await.unwrap(),
            vec![DlqTopicInfo {
                topic: "demo".to_string(),
                broker_type: "builtin".to_string(),
                message_count: 1,
            }]
        );
        let messages = dlq_messages(
            store,
            DlqMessagesInput {
                topic: "demo".to_string(),
                offset: 0,
                limit: 50,
            },
        )
        .await
        .unwrap();
        assert_eq!(messages[0].id, id);
        assert_eq!(messages[0].retries, 1);
    }
}
