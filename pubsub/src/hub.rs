//! The backend in use, and its hot-swap. Port of the builtin PubSubWorker's
//! apply_config swap (engine/src/workers/pubsub/pubsub.rs:508-577). Observers
//! live with the engine, so a swap only replaces the backend that carries
//! events to other engines.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::adapters::PubSubAdapter;

pub struct Hub {
    adapter: RwLock<Arc<dyn PubSubAdapter>>,
}

impl Hub {
    pub fn new(adapter: Arc<dyn PubSubAdapter>) -> Self {
        Self {
            adapter: RwLock::new(adapter),
        }
    }

    async fn adapter_snapshot(&self) -> Arc<dyn PubSubAdapter> {
        self.adapter.read().await.clone()
    }

    /// Publish `data` to `topic`. Parity with the builtin `publish` function
    /// (pubsub.rs:89-107): empty topic fails with `topic_not_set`. iii
    /// delivers the call to the observers on this engine; the backend carries
    /// the event to other engines unless it was `relayed` from one.
    pub async fn publish(&self, topic: &str, data: Value, relayed: bool) -> Result<(), String> {
        if topic.is_empty() {
            return Err("topic_not_set: Topic is not set".to_string());
        }
        if relayed {
            return Ok(());
        }
        tracing::debug!(topic = %topic, "Publishing event");
        self.adapter_snapshot().await.publish(topic, data).await;
        Ok(())
    }

    /// Hot-swap the backend: new publishes route through `new_adapter`, then
    /// the previous one stops receiving from other engines.
    pub async fn swap_adapter(&self, new_adapter: Arc<dyn PubSubAdapter>) {
        let previous = {
            let mut adapter = self.adapter.write().await;
            std::mem::replace(&mut *adapter, new_adapter)
        };
        previous.shutdown().await;
        tracing::info!("pubsub: adapter hot-swapped");
    }

    /// Stop the backend (parity with the builtin destroy, pubsub.rs:294-330:
    /// the redis listener task must be aborted, not leaked).
    pub async fn shutdown(&self) {
        self.adapter_snapshot().await.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingAdapter {
        published: StdMutex<Vec<(String, serde_json::Value)>>,
        shut_down: AtomicBool,
    }

    #[async_trait]
    impl crate::adapters::PubSubAdapter for RecordingAdapter {
        async fn publish(&self, topic: &str, data: serde_json::Value) {
            self.published
                .lock()
                .unwrap()
                .push((topic.to_string(), data));
        }
        async fn shutdown(&self) {
            self.shut_down.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn publish_rejects_empty_topic() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        let err = hub
            .publish("", serde_json::json!({"x": 1}), false)
            .await
            .unwrap_err();
        assert!(err.contains("topic_not_set"));
        assert!(err.contains("Topic is not set"));
        assert!(adapter.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_delegates_to_adapter() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.publish("orders", serde_json::json!({"id": 1}), false)
            .await
            .unwrap();
        let published = adapter.published.lock().unwrap();
        assert_eq!(
            published.as_slice(),
            &[("orders".to_string(), serde_json::json!({"id": 1}))]
        );
    }

    #[tokio::test]
    async fn a_relayed_event_is_not_carried_again() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.publish("orders", serde_json::json!({"id": 1}), true)
            .await
            .unwrap();
        assert!(adapter.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn swap_routes_new_publishes_and_stops_the_old_backend() {
        let old = Arc::new(RecordingAdapter::default());
        let new = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(old.clone());

        hub.swap_adapter(new.clone()).await;
        assert!(old.shut_down.load(Ordering::SeqCst));
        assert!(!new.shut_down.load(Ordering::SeqCst));

        hub.publish("orders", serde_json::json!(1), false)
            .await
            .unwrap();
        assert_eq!(new.published.lock().unwrap().len(), 1);
        assert!(old.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shutdown_stops_the_backend() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.shutdown().await;
        assert!(adapter.shut_down.load(Ordering::SeqCst));
    }
}
