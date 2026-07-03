//! Subscription tracking + adapter hot-swap core. Port of the builtin
//! PubSubWorker's subscriptions set and apply_config swap
//! (engine/src/workers/pubsub/pubsub.rs:64-76, 508-577). The subscriptions
//! Mutex is also the swap lock: subscribe/unsubscribe/swap serialize on it so
//! a subscription can never land on a backend being torn down.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::{Mutex, RwLock};

use crate::adapters::PubSubAdapter;

/// `trigger id -> (topic, function_id)`. The authoritative set the adapter
/// hot-swap rebinds, and the topic lookup for id-only unregisters.
type SubscriptionSet = HashMap<String, (String, String)>;

pub struct Hub {
    adapter: RwLock<Arc<dyn PubSubAdapter>>,
    subscriptions: Mutex<SubscriptionSet>,
}

impl Hub {
    pub fn new(adapter: Arc<dyn PubSubAdapter>) -> Self {
        Self {
            adapter: RwLock::new(adapter),
            subscriptions: Mutex::new(SubscriptionSet::new()),
        }
    }

    async fn adapter_snapshot(&self) -> Arc<dyn PubSubAdapter> {
        self.adapter.read().await.clone()
    }

    /// Publish `data` to `topic`. Parity with the builtin `publish` function
    /// (pubsub.rs:89-107): empty topic fails with `topic_not_set`; delivery is
    /// delegated to the backend (fire-and-forget) and always succeeds.
    pub async fn publish(&self, topic: &str, data: Value) -> Result<(), String> {
        if topic.is_empty() {
            return Err("topic_not_set: Topic is not set".to_string());
        }
        tracing::debug!(topic = %topic, "Publishing event");
        self.adapter_snapshot().await.publish(topic, data).await;
        Ok(())
    }

    /// Record and bind a subscription. Empty topic: warn + no-op (parity with
    /// the builtin register_trigger, pubsub.rs:134-139).
    pub async fn subscribe(&self, id: &str, topic: &str, function_id: &str) {
        if topic.is_empty() {
            tracing::warn!(function_id = %function_id, "Topic is not set for trigger");
            return;
        }
        let mut subscriptions = self.subscriptions.lock().await;
        self.adapter_snapshot()
            .await
            .subscribe(topic, id, function_id)
            .await;
        subscriptions.insert(id.to_string(), (topic.to_string(), function_id.to_string()));
    }

    /// Unbind by trigger id alone (the SDK's unregister stub carries only the
    /// id); the topic comes from the tracked set. Unknown id: no-op.
    pub async fn unsubscribe(&self, id: &str) {
        let mut subscriptions = self.subscriptions.lock().await;
        if let Some((topic, _function_id)) = subscriptions.remove(id) {
            self.adapter_snapshot().await.unsubscribe(&topic, id).await;
        }
    }

    pub async fn subscription_count(&self) -> usize {
        self.subscriptions.lock().await.len()
    }

    /// Hot-swap the backend, builtin order (pubsub.rs:552-570): re-subscribe
    /// the tracked set onto the new backend BEFORE the swap (no delivery gap;
    /// the brief double-subscription overlap is accepted for fire-and-forget
    /// pub/sub), swap so new publishes route through the new backend, then
    /// tear the set off the previous backend (aborting redis listener tasks).
    pub async fn swap_adapter(&self, new_adapter: Arc<dyn PubSubAdapter>) {
        let subscriptions = self.subscriptions.lock().await;
        for (id, (topic, function_id)) in subscriptions.iter() {
            new_adapter.subscribe(topic, id, function_id).await;
        }
        let previous = {
            let mut adapter = self.adapter.write().await;
            std::mem::replace(&mut *adapter, new_adapter)
        };
        for (id, (topic, _function_id)) in subscriptions.iter() {
            previous.unsubscribe(topic, id).await;
        }
        tracing::info!(
            subscriptions = subscriptions.len(),
            "pubsub: adapter hot-swapped; subscriptions rebound onto the new backend"
        );
    }

    /// Tear down every tracked subscription (parity with the builtin destroy,
    /// pubsub.rs:294-330: redis listener tasks must be aborted, not leaked).
    pub async fn shutdown(&self) {
        let mut subscriptions = self.subscriptions.lock().await;
        let adapter = self.adapter_snapshot().await;
        for (id, (topic, _function_id)) in subscriptions.drain() {
            adapter.unsubscribe(&topic, &id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingAdapter {
        published: StdMutex<Vec<(String, serde_json::Value)>>,
        subscribed: StdMutex<Vec<(String, String, String)>>,
        unsubscribed: StdMutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl crate::adapters::PubSubAdapter for RecordingAdapter {
        async fn publish(&self, topic: &str, data: serde_json::Value) {
            self.published.lock().unwrap().push((topic.to_string(), data));
        }
        async fn subscribe(&self, topic: &str, id: &str, function_id: &str) {
            self.subscribed.lock().unwrap().push((
                topic.to_string(),
                id.to_string(),
                function_id.to_string(),
            ));
        }
        async fn unsubscribe(&self, topic: &str, id: &str) {
            self.unsubscribed
                .lock()
                .unwrap()
                .push((topic.to_string(), id.to_string()));
        }
    }

    #[tokio::test]
    async fn publish_rejects_empty_topic() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        let err = hub.publish("", serde_json::json!({"x": 1})).await.unwrap_err();
        assert!(err.contains("topic_not_set"));
        assert!(err.contains("Topic is not set"));
        assert!(adapter.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn publish_delegates_to_adapter() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.publish("orders", serde_json::json!({"id": 1})).await.unwrap();
        let published = adapter.published.lock().unwrap();
        assert_eq!(published.as_slice(), &[("orders".to_string(), serde_json::json!({"id": 1}))]);
    }

    #[tokio::test]
    async fn subscribe_records_and_unsubscribe_finds_topic_by_id() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.subscribe("sub-1", "orders", "test::listener").await;
        assert_eq!(hub.subscription_count().await, 1);

        // Only the id is known at unregister time (SDK stub).
        hub.unsubscribe("sub-1").await;
        assert_eq!(hub.subscription_count().await, 0);
        assert_eq!(
            adapter.unsubscribed.lock().unwrap().as_slice(),
            &[("orders".to_string(), "sub-1".to_string())]
        );
    }

    #[tokio::test]
    async fn subscribe_with_empty_topic_is_noop() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.subscribe("sub-1", "", "test::listener").await;
        assert_eq!(hub.subscription_count().await, 0);
        assert!(adapter.subscribed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_unknown_id_is_noop() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.unsubscribe("ghost").await;
        assert!(adapter.unsubscribed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn swap_resubscribes_new_backend_then_unsubscribes_old() {
        let old = Arc::new(RecordingAdapter::default());
        let new = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(old.clone());
        hub.subscribe("sub-1", "orders", "test::listener").await;

        hub.swap_adapter(new.clone()).await;

        assert_eq!(
            new.subscribed.lock().unwrap().as_slice(),
            &[(
                "orders".to_string(),
                "sub-1".to_string(),
                "test::listener".to_string()
            )],
            "swap must re-subscribe the live subscription onto the new backend"
        );
        assert_eq!(
            old.unsubscribed.lock().unwrap().as_slice(),
            &[("orders".to_string(), "sub-1".to_string())],
            "swap must tear the live subscription off the previous backend"
        );

        // New publishes route through the new backend.
        hub.publish("orders", serde_json::json!(1)).await.unwrap();
        assert_eq!(new.published.lock().unwrap().len(), 1);
        assert!(old.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shutdown_unsubscribes_everything() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        hub.subscribe("sub-1", "orders", "fn::a").await;
        hub.subscribe("sub-2", "invoices", "fn::b").await;
        hub.shutdown().await;
        assert_eq!(hub.subscription_count().await, 0);
        assert_eq!(adapter.unsubscribed.lock().unwrap().len(), 2);
    }
}
