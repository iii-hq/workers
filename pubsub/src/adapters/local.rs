//! In-process backend (default). Port of the builtin local adapter
//! (engine/src/workers/pubsub/adapters/local_adapter.rs): a topic ->
//! {subscription id -> function id} map; publish spawns one fire-and-forget
//! invocation per subscriber with the raw data value. One deliberate fix vs
//! the builtin: unsubscribe removes only the given id instead of dropping the
//! whole topic entry (builtin bug, local_adapter.rs:82-96).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use super::{Invoker, PubSubAdapter};

type TopicName = String;
type SubscriptionId = String;
type FunctionPath = String;

pub struct LocalAdapter {
    subscriptions: Arc<RwLock<HashMap<TopicName, HashMap<SubscriptionId, FunctionPath>>>>,
    invoker: Arc<dyn Invoker>,
}

impl LocalAdapter {
    pub fn new(invoker: Arc<dyn Invoker>) -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            invoker,
        }
    }
}

#[async_trait]
impl PubSubAdapter for LocalAdapter {
    async fn publish(&self, topic: &str, data: Value) {
        let subs = self.subscriptions.read().await;
        let Some(sub_info) = subs.get(topic) else {
            tracing::debug!(topic = %topic, "Event: No subscriptions found");
            return;
        };
        for function_id in sub_info.values() {
            tracing::debug!(function_id = %function_id, topic = %topic, "Event: Invoking function");
            let function_id = function_id.clone();
            let data = data.clone();
            let invoker = self.invoker.clone();
            // Fire-and-forget, parity with the builtin's spawned engine.call.
            tokio::spawn(async move {
                if let Err(e) = invoker.call(&function_id, data).await {
                    tracing::debug!(function_id = %function_id, error = %e, "pubsub delivery failed");
                }
            });
        }
    }

    async fn subscribe(&self, topic: &str, id: &str, function_id: &str) {
        let mut subs = self.subscriptions.write().await;
        subs.entry(topic.to_string())
            .or_default()
            .insert(id.to_string(), function_id.to_string());
    }

    async fn unsubscribe(&self, topic: &str, id: &str) {
        tracing::debug!(topic = %topic, id = %id, "Unsubscribing from PubSub topic");
        let mut subs = self.subscriptions.write().await;
        if let Some(sub_info) = subs.get_mut(topic) {
            sub_info.remove(id);
            if sub_info.is_empty() {
                subs.remove(topic);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex as TokioMutex;

    #[derive(Default)]
    struct RecordingInvoker {
        calls: AtomicUsize,
        received: TokioMutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait]
    impl Invoker for RecordingInvoker {
        async fn call(
            &self,
            function_id: &str,
            payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.received
                .lock()
                .await
                .push((function_id.to_string(), payload));
            Ok(None)
        }
    }

    async fn wait_for_calls(inv: &RecordingInvoker, n: usize) {
        for _ in 0..50 {
            if inv.calls.load(Ordering::SeqCst) >= n {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("invoker never reached {n} call(s)");
    }

    #[tokio::test]
    async fn publish_fans_out_raw_data_to_all_topic_subscribers() {
        let inv = Arc::new(RecordingInvoker::default());
        let a = LocalAdapter::new(inv.clone());
        a.subscribe("orders", "sub-1", "fn::a").await;
        a.subscribe("orders", "sub-2", "fn::b").await;
        a.subscribe("other", "sub-3", "fn::c").await;

        a.publish("orders", serde_json::json!({"id": 7})).await;
        wait_for_calls(&inv, 2).await;

        let received = inv.received.lock().await;
        assert_eq!(received.len(), 2, "only the topic's subscribers fire");
        for (_fn_id, payload) in received.iter() {
            // Parity: the subscriber gets the RAW published data, no envelope.
            assert_eq!(payload, &serde_json::json!({"id": 7}));
        }
        let mut fns: Vec<&str> = received.iter().map(|(f, _)| f.as_str()).collect();
        fns.sort();
        assert_eq!(fns, vec!["fn::a", "fn::b"]);
    }

    #[tokio::test]
    async fn publish_without_subscribers_is_noop() {
        let inv = Arc::new(RecordingInvoker::default());
        let a = LocalAdapter::new(inv.clone());
        a.publish("ghost", serde_json::json!(1)).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(inv.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unsubscribe_removes_only_the_given_id() {
        // Deviation from the builtin (which drops the whole topic entry):
        // the surviving subscriber keeps receiving.
        let inv = Arc::new(RecordingInvoker::default());
        let a = LocalAdapter::new(inv.clone());
        a.subscribe("orders", "sub-1", "fn::a").await;
        a.subscribe("orders", "sub-2", "fn::b").await;
        a.unsubscribe("orders", "sub-1").await;

        a.publish("orders", serde_json::json!({"id": 1})).await;
        wait_for_calls(&inv, 1).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let received = inv.received.lock().await;
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, "fn::b");
    }
}
