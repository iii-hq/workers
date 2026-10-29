//! Cross-engine Redis Pub/Sub backend. Every pubsub worker on the same Redis
//! PUBLISHes the events published on its engine to one channel,
//! [`REDIS_CHANNEL`], as `{ origin, topic, data }`, and listens on it. An
//! event from another worker (a different `origin`) is handed to the
//! [`Relay`], which calls `publish` on this engine so iii delivers it to the
//! observers here. A worker skips its own events: iii delivered them when
//! `publish` ran.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::{PubSubAdapter, Relay};

/// Builtin parity: engine/src/workers/redis.rs:9.
const REDIS_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// The Redis channel every pubsub worker publishes to and listens on.
pub const REDIS_CHANNEL: &str = "iii:pubsub";

/// One event on [`REDIS_CHANNEL`].
#[derive(Debug, Serialize)]
struct OutgoingEvent<'a> {
    origin: &'a str,
    topic: &'a str,
    data: &'a Value,
}

#[derive(Debug, PartialEq, Deserialize)]
struct IncomingEvent {
    origin: String,
    topic: String,
    data: Value,
}

pub struct RedisAdapter {
    publisher: Arc<Mutex<ConnectionManager>>,
    origin: String,
    listener: JoinHandle<()>,
}

/// A process-unique id for this adapter's events.
fn new_origin() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "{:016x}-{:x}",
        RandomState::new().hash_one(nanos),
        std::process::id()
    )
}

/// The event in `payload` when another worker published it.
fn foreign_event(payload: &str, own_origin: &str) -> Result<Option<IncomingEvent>, String> {
    let event: IncomingEvent = serde_json::from_str(payload).map_err(|e| e.to_string())?;
    Ok((event.origin != own_origin).then_some(event))
}

impl RedisAdapter {
    pub async fn connect(redis_url: &str, relay: Arc<dyn Relay>) -> anyhow::Result<Self> {
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

        let origin = new_origin();
        let listener = tokio::spawn(listen(client, origin.clone(), relay));
        Ok(Self {
            publisher: Arc::new(Mutex::new(manager)),
            origin,
            listener,
        })
    }
}

/// Receives the events of [`REDIS_CHANNEL`] and relays the ones other
/// workers published, while anything on this engine observes `publish`.
async fn listen(client: Client, origin: String, relay: Arc<dyn Relay>) {
    let mut pubsub = match client.get_async_pubsub().await {
        Ok(pubsub) => pubsub,
        Err(e) => {
            tracing::error!(error = %e, channel = REDIS_CHANNEL, "Failed to get async pubsub connection");
            return;
        }
    };
    if let Err(e) = pubsub.subscribe(REDIS_CHANNEL).await {
        tracing::error!(error = %e, channel = REDIS_CHANNEL, "Failed to subscribe to Redis channel");
        return;
    }
    let mut messages = pubsub.into_on_message();
    while let Some(msg) = messages.next().await {
        let payload: String = match msg.get_payload() {
            Ok(payload) => payload,
            Err(e) => {
                tracing::error!(error = %e, channel = REDIS_CHANNEL, "Failed to get message payload");
                continue;
            }
        };
        let event = match foreign_event(&payload, &origin) {
            Ok(Some(event)) => event,
            Ok(None) => continue,
            Err(e) => {
                tracing::error!(error = %e, channel = REDIS_CHANNEL, "Failed to parse pubsub event");
                continue;
            }
        };
        if !relay.is_observed() {
            continue;
        }
        let relay = relay.clone();
        tokio::spawn(async move {
            relay.deliver(&event.topic, event.data, &event.origin).await;
        });
    }
    tracing::debug!(channel = REDIS_CHANNEL, "Redis listener ended");
}

#[async_trait]
impl PubSubAdapter for RedisAdapter {
    /// Failures here are NOT surfaced to the caller: `publish` keeps returning
    /// its success (null) result, and the observers on this engine already
    /// received the event. Each failure is logged at error level, with entry
    /// point and reason only (never the event data).
    async fn publish(&self, topic: &str, data: Value) {
        let event = OutgoingEvent {
            origin: &self.origin,
            topic,
            data: &data,
        };
        let event_json = match serde_json::to_string(&event) {
            Ok(json) => json,
            Err(e) => {
                tracing::error!(
                    entry_point = "publish",
                    adapter = "redis",
                    error = %e,
                    topic = %topic,
                    "Failed to serialize event data; event not carried to other engines"
                );
                return;
            }
        };
        let mut conn = self.publisher.lock().await;
        if let Err(e) = conn.publish::<_, _, ()>(REDIS_CHANNEL, &event_json).await {
            tracing::error!(
                entry_point = "publish",
                adapter = "redis",
                error = %e,
                topic = %topic,
                "Failed to publish event to Redis; event not carried to other engines"
            );
        }
    }

    async fn shutdown(&self) {
        self.listener.abort();
    }
}

impl Drop for RedisAdapter {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_carry_origin_topic_and_data() {
        let data = json!({"id": 7});
        let encoded = serde_json::to_string(&OutgoingEvent {
            origin: "a",
            topic: "orders",
            data: &data,
        })
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&encoded).unwrap(),
            json!({"origin": "a", "topic": "orders", "data": {"id": 7}})
        );
        assert_eq!(
            foreign_event(&encoded, "b").unwrap(),
            Some(IncomingEvent {
                origin: "a".into(),
                topic: "orders".into(),
                data,
            })
        );
    }

    #[test]
    fn a_workers_own_events_are_skipped() {
        let encoded = r#"{"origin":"a","topic":"orders","data":1}"#;
        assert_eq!(foreign_event(encoded, "a").unwrap(), None);
    }

    #[test]
    fn a_malformed_message_is_an_error() {
        assert!(foreign_event("not json", "a").is_err());
        assert!(foreign_event(r#"{"id": 1}"#, "a").is_err());
    }

    #[test]
    fn origins_differ_between_adapters() {
        assert_ne!(new_origin(), new_origin());
    }
}
