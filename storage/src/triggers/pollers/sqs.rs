//! S3 → SQS poller. Long-polls SQS, normalizes the S3 event JSON,
//! and yields `ObjectEventNormalized`s for the dispatcher to deliver.

use crate::triggers::normalize::{normalize_s3, EventKind, NormalizeError, ObjectEventNormalized};
use aws_sdk_sqs::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

pub struct SqsPoller {
    pub client: Client,
    pub queue_url: String,
    /// Map underlying-bucket-name → worker-facing-name. Multiple buckets can share a queue.
    pub bucket_reverse: Arc<HashMap<String, String>>,
}

impl SqsPoller {
    pub fn new(
        client: Client,
        queue_url: String,
        bucket_reverse: Arc<HashMap<String, String>>,
    ) -> Self {
        Self {
            client,
            queue_url,
            bucket_reverse,
        }
    }

    /// One iteration of the long-poll loop. Receives up to 10 messages,
    /// normalizes each, calls `dispatch` for each event. Acks via
    /// `delete_message` only when `dispatch` returns `Ok(true)`.
    /// `dispatch_timeout` bounds each individual dispatch call.
    ///
    /// Returns the number of messages received (0 on empty / error).
    pub async fn poll_once<F, Fut>(&self, dispatch_timeout: Duration, dispatch: F) -> usize
    where
        F: Fn(ObjectEventNormalized) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = bool> + Send,
    {
        let out = match self
            .client
            .receive_message()
            .queue_url(&self.queue_url)
            .max_number_of_messages(10)
            .wait_time_seconds(20)
            .send()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(error=%e, "sqs receive failed; skipping iteration");
                return 0;
            }
        };
        let messages = out.messages.unwrap_or_default();
        let count = messages.len();
        for msg in messages {
            let body = match msg.body.as_deref() {
                Some(b) => b,
                None => continue,
            };
            let value: serde_json::Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error=%e, "sqs body not JSON; skipping");
                    continue;
                }
            };
            let bucket_reverse = self.bucket_reverse.clone();
            let events = match normalize_s3(&value, &move |u| bucket_reverse.get(u).cloned()) {
                Ok(events) => events,
                Err(NormalizeError::Ignored(_)) | Err(NormalizeError::UnmappedBucket(_)) => {
                    continue
                }
                Err(e) => {
                    tracing::warn!(error=?e, "normalize failed; skipping");
                    continue;
                }
            };
            let mut all_acked = true;
            for ev in events {
                let f = dispatch(ev);
                match timeout(dispatch_timeout, f).await {
                    Ok(true) => {}
                    Ok(false) => all_acked = false,
                    Err(_) => {
                        tracing::warn!("trigger dispatch timed out");
                        all_acked = false;
                    }
                }
            }
            if all_acked {
                if let Some(rh) = msg.receipt_handle {
                    let _ = self
                        .client
                        .delete_message()
                        .queue_url(&self.queue_url)
                        .receipt_handle(rh)
                        .send()
                        .await;
                }
            }
        }
        count
    }
}

/// Run `poll_once` forever, sleeping briefly between empty pulls so we don't
/// hammer SQS when the queue is quiet. Cancel by aborting the JoinHandle.
pub async fn run_loop<D>(poller: SqsPoller, dispatcher: std::sync::Arc<D>)
where
    D: crate::triggers::dispatcher::EventDispatcher + 'static,
{
    use tokio::time::{sleep, Duration};
    loop {
        let count = poller
            .poll_once(Duration::from_secs(60), |ev| {
                let d = dispatcher.clone();
                async move { d.dispatch(ev).await }
            })
            .await;
        if count == 0 {
            // SQS already long-polled (wait_time_seconds=20); short sleep just
            // smooths over transient errors that returned 0 immediately.
            sleep(Duration::from_millis(500)).await;
        }
    }
}

/// Filter helper: should we dispatch this event to a trigger that subscribes
/// to `direction`? Used by the registration layer.
pub fn matches_direction(event: &ObjectEventNormalized, direction: EventKind) -> bool {
    event.event_kind == direction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::normalize::EventKind;

    #[test]
    fn matches_direction_filters_by_kind() {
        let mut ev = ObjectEventNormalized {
            bucket: "u".into(),
            key: "k".into(),
            size: 0,
            etag: "".into(),
            content_type: None,
            created_at: None,
            deleted_at: None,
            event_kind: EventKind::Created,
            raw_event_id: "".into(),
        };
        assert!(matches_direction(&ev, EventKind::Created));
        assert!(!matches_direction(&ev, EventKind::Deleted));
        ev.event_kind = EventKind::Deleted;
        assert!(matches_direction(&ev, EventKind::Deleted));
    }
}
