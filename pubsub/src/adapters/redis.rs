//! Cross-instance Redis Pub/Sub backend. Port of the builtin
//! (engine/src/workers/pubsub/adapters/redis_adapter.rs): publish serializes
//! the data to a JSON string and PUBLISHes it; subscribe spawns one listener
//! task per topic. Parity quirks kept: only ONE subscription per topic per
//! instance (a second subscribe warns and is dropped); unsubscribe is
//! id-checked and aborts the topic's listener task.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::{Invoker, PubSubAdapter};

/// Builtin parity: engine/src/workers/redis.rs:9.
const REDIS_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

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
    pub async fn connect(redis_url: &str, invoker: Arc<dyn Invoker>) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)?;
        let manager = timeout(REDIS_CONNECTION_TIMEOUT, client.get_connection_manager())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Redis connection timed out after {REDIS_CONNECTION_TIMEOUT:?}. \
                     Please ensure Redis is running at: {redis_url}"
                )
            })?
            .map_err(|e| anyhow::anyhow!("Failed to connect to Redis at {redis_url}: {e}"))?;

        Ok(Self {
            publisher: Arc::new(Mutex::new(manager)),
            subscriber: Arc::new(client),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            invoker,
        })
    }
}

#[async_trait]
impl PubSubAdapter for RedisAdapter {
    async fn publish(&self, topic: &str, data: Value) {
        let event_json = match serde_json::to_string(&data) {
            Ok(json) => json,
            Err(e) => {
                tracing::error!(error = %e, topic = %topic, "Failed to serialize event data");
                return;
            }
        };
        let mut conn = self.publisher.lock().await;
        if let Err(e) = conn.publish::<_, _, ()>(topic, &event_json).await {
            tracing::error!(error = %e, topic = %topic, "Failed to publish event to Redis");
        }
    }

    async fn subscribe(&self, topic: &str, id: &str, function_id: &str) {
        // Builtin quirk kept: one listener per topic per instance.
        if self.subscriptions.read().await.contains_key(topic) {
            tracing::warn!(topic = %topic, id = %id, "Already subscribed to topic");
            return;
        }

        let topic_owned = topic.to_string();
        let function_id = function_id.to_string();
        let subscriber = self.subscriber.clone();
        let invoker = self.invoker.clone();

        let task_handle = tokio::spawn(async move {
            let mut pubsub = match subscriber.get_async_pubsub().await {
                Ok(pubsub) => pubsub,
                Err(e) => {
                    tracing::error!(error = %e, topic = %topic_owned, "Failed to get async pubsub connection");
                    return;
                }
            };
            if let Err(e) = pubsub.subscribe(&topic_owned).await {
                tracing::error!(error = %e, topic = %topic_owned, "Failed to subscribe to Redis channel");
                return;
            }
            let mut messages = pubsub.into_on_message();
            while let Some(msg) = messages.next().await {
                let payload: String = match msg.get_payload() {
                    Ok(payload) => payload,
                    Err(e) => {
                        tracing::error!(error = %e, topic = %topic_owned, "Failed to get message payload");
                        continue;
                    }
                };
                let data: Value = match serde_json::from_str(&payload) {
                    Ok(data) => data,
                    Err(e) => {
                        tracing::error!(error = %e, topic = %topic_owned, "Failed to parse message as JSON");
                        continue;
                    }
                };
                let invoker = invoker.clone();
                let function_id = function_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = invoker.call(&function_id, data).await {
                        tracing::debug!(function_id = %function_id, error = %e, "pubsub delivery failed");
                    }
                });
            }
            tracing::debug!(topic = %topic_owned, "Subscription task ended");
        });

        self.subscriptions.write().await.insert(
            topic.to_string(),
            SubscriptionInfo {
                id: id.to_string(),
                task_handle,
            },
        );
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        let mut subs = self.subscriptions.write().await;
        if let Some(sub_info) = subs.remove(topic) {
            if sub_info.id == id {
                sub_info.task_handle.abort();
            } else {
                tracing::warn!(topic = %topic, id = %id, "Subscription ID mismatch, not unsubscribing");
                subs.insert(topic.to_string(), sub_info);
            }
        } else {
            tracing::warn!(topic = %topic, id = %id, "No active subscription found for topic");
        }
    }
}
