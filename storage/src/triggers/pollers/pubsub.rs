//! GCS → Pub/Sub poller. Pulls from a subscription, normalizes the GCS
//! notification JSON, dispatches.

use crate::triggers::normalize::{normalize_gcs, EventKind, NormalizeError, ObjectEventNormalized};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

pub struct PubsubPoller {
    /// Resolved subscription name (e.g. `projects/X/subscriptions/Y`).
    pub subscription: String,
    pub bucket_reverse: Arc<HashMap<String, String>>,
}

impl PubsubPoller {
    pub fn new(subscription: String, bucket_reverse: Arc<HashMap<String, String>>) -> Self {
        Self {
            subscription,
            bucket_reverse,
        }
    }

    /// One iteration of the pull loop. The actual GCS Pub/Sub pull cycle
    /// requires a `gcloud_pubsub::Client` instance; live wiring lands when
    /// `main.rs` plumbs the trigger registration. This method is the
    /// in-process logic — given a raw event JSON value, normalize and
    /// dispatch.
    ///
    /// `event` must be the published Pub/Sub message attributes+data
    /// envelope (matching the fixture shape under tests/fixtures/events/).
    pub async fn dispatch_one<F, Fut>(
        &self,
        event: &serde_json::Value,
        dispatch_timeout: Duration,
        dispatch: F,
    ) -> bool
    where
        F: FnOnce(ObjectEventNormalized) -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let bucket_reverse = self.bucket_reverse.clone();
        let n = match normalize_gcs(event, &move |u| bucket_reverse.get(u).cloned()) {
            Ok(n) => n,
            Err(NormalizeError::Ignored(_)) | Err(NormalizeError::UnmappedBucket(_)) => {
                return true; // drop the message — ack so it doesn't redeliver forever
            }
            Err(e) => {
                tracing::warn!(error=?e, "normalize failed; not acking");
                return false;
            }
        };
        matches!(timeout(dispatch_timeout, dispatch(n)).await, Ok(true))
    }
}


/// Pull from the configured subscription and dispatch each message. The
/// gcloud-pubsub client owns the gRPC connection — caller passes one in so
/// we don't rebuild auth/connection per loop iteration.
pub async fn run_loop<D>(
    poller: PubsubPoller,
    pubsub_client: gcloud_pubsub::client::Client,
    dispatcher: std::sync::Arc<D>,
) where
    D: crate::triggers::dispatcher::EventDispatcher + 'static,
{
    use tokio::time::{sleep, Duration};
    let subscription = pubsub_client.subscription(&poller.subscription);
    loop {
        // batch=10, no retry on the pull itself — let the outer loop retry.
        let messages = match subscription.pull(10, None).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    subscription = %poller.subscription,
                    error = %e,
                    "pubsub pull failed"
                );
                sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        if messages.is_empty() {
            // Pub/Sub doesn't have a true server-side long-poll like SQS, so
            // we sleep briefly to avoid a tight CPU loop on idle topics.
            sleep(Duration::from_millis(500)).await;
            continue;
        }
        for received in messages {
            // Reconstruct the JSON shape normalize_gcs expects: an envelope
            // with `attributes` map and `data` body. The Pub/Sub SDK gives us
            // those as separate fields on PubsubMessage.
            let pm = &received.message;
            let attrs: serde_json::Map<String, serde_json::Value> = pm
                .attributes
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            // GCS publishes the Object resource as the JSON body; parse if we can.
            let data = serde_json::from_slice::<serde_json::Value>(&pm.data)
                .unwrap_or(serde_json::Value::Null);
            let envelope = serde_json::json!({
                "messageId": pm.message_id,
                "attributes": serde_json::Value::Object(attrs),
                "data": data,
            });
            let acked = poller
                .dispatch_one(&envelope, Duration::from_secs(60), |ev| {
                    let d = dispatcher.clone();
                    async move { d.dispatch(ev).await }
                })
                .await;
            if acked {
                if let Err(e) = received.ack().await {
                    tracing::warn!(error = %e, "pubsub ack failed; will redeliver");
                }
            } else if let Err(e) = received.nack().await {
                tracing::warn!(error = %e, "pubsub nack failed");
            }
        }
    }
}

pub fn matches_direction(event: &ObjectEventNormalized, direction: EventKind) -> bool {
    event.event_kind == direction
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fixture(name: &str) -> serde_json::Value {
        let path = format!(
            "{}/tests/fixtures/events/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    #[tokio::test]
    async fn dispatch_one_acks_on_handler_success() {
        let mut br = HashMap::new();
        br.insert("my-app-documents".to_string(), "documents".to_string());
        let p = PubsubPoller::new("sub-x".into(), Arc::new(br));
        let ev = fixture("gcs_object_finalize");
        let called = AtomicBool::new(false);
        let acked = p
            .dispatch_one(&ev, Duration::from_secs(1), |_| async {
                called.store(true, Ordering::SeqCst);
                true
            })
            .await;
        assert!(acked);
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dispatch_one_drops_unmapped_bucket() {
        let p = PubsubPoller::new("sub-x".into(), Arc::new(HashMap::new()));
        let ev = fixture("gcs_object_finalize");
        let acked = p
            .dispatch_one(&ev, Duration::from_secs(1), |_| async { false })
            .await;
        assert!(acked, "unmapped events should be acked (dropped)");
    }
}
