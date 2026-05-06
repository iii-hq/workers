//! R2 → Cloudflare Queues poller. Soft-ship per design: structurally complete,
//! relies on CF's REST consume API at:
//!   https://api.cloudflare.com/client/v4/accounts/{account_id}/queues/{queue_id}/messages/pull
//! and ack via the corresponding `/ack` endpoint. Real behaviour validated
//! during the Phase 15 spike — if CF's API turns out to require different
//! pagination or auth shape, this module is the place to adjust.

use crate::triggers::normalize::{normalize_r2, EventKind, NormalizeError, ObjectEventNormalized};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

pub struct CfQueuePoller {
    pub account_id: String,
    pub queue_id: String,
    pub api_token: String,
    pub bucket_reverse: Arc<HashMap<String, String>>,
    pub http: reqwest::Client,
}

impl CfQueuePoller {
    pub fn new(
        account_id: String,
        queue_id: String,
        api_token: String,
        bucket_reverse: Arc<HashMap<String, String>>,
    ) -> Self {
        Self {
            account_id,
            queue_id,
            api_token,
            bucket_reverse,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }

    fn pull_url(&self) -> String {
        format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/queues/{}/messages/pull",
            self.account_id, self.queue_id
        )
    }

    fn ack_url(&self) -> String {
        format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/queues/{}/messages/ack",
            self.account_id, self.queue_id
        )
    }

    /// Pull up to `batch_size` messages, normalize, dispatch, ack on success.
    /// Returns the number of messages received this iteration. Errors are
    /// logged and result in 0 (caller backs off).
    pub async fn poll_once<F, Fut>(
        &self,
        batch_size: u32,
        dispatch_timeout: Duration,
        dispatch: F,
    ) -> usize
    where
        F: Fn(ObjectEventNormalized) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = bool> + Send,
    {
        let resp = match self
            .http
            .post(self.pull_url())
            .bearer_auth(&self.api_token)
            .json(&serde_json::json!({
                "batch_size": batch_size,
                "visibility_timeout_ms": 60_000
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error=%e, "cf-queue pull failed");
                return 0;
            }
        };
        let status = resp.status();
        if !status.is_success() {
            tracing::warn!(status = %status, "cf-queue pull non-success");
            return 0;
        }
        let body: serde_json::Value = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error=%e, "cf-queue body parse failed");
                return 0;
            }
        };
        let messages = body
            .pointer("/result/messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let count = messages.len();
        let mut acked_lease_ids: Vec<String> = Vec::new();
        for msg in &messages {
            let lease_id = msg
                .get("lease_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let payload = match msg.get("body") {
                Some(p) => p,
                None => continue,
            };
            let bucket_reverse = self.bucket_reverse.clone();
            let n = match normalize_r2(payload, &move |u| bucket_reverse.get(u).cloned()) {
                Ok(n) => n,
                Err(NormalizeError::Ignored(_)) | Err(NormalizeError::UnmappedBucket(_)) => {
                    if !lease_id.is_empty() {
                        acked_lease_ids.push(lease_id);
                    }
                    continue;
                }
                Err(_) => continue,
            };
            let f = dispatch(n);
            match timeout(dispatch_timeout, f).await {
                Ok(true) if !lease_id.is_empty() => acked_lease_ids.push(lease_id),
                _ => {}
            }
        }
        if !acked_lease_ids.is_empty() {
            let _ = self
                .http
                .post(self.ack_url())
                .bearer_auth(&self.api_token)
                .json(&serde_json::json!({ "acks": acked_lease_ids }))
                .send()
                .await;
        }
        count
    }
}


/// Run `poll_once` forever. CF Queues' pull API doesn't have native long-poll,
/// so we sleep ~2s between empty pulls.
pub async fn run_loop<D>(poller: CfQueuePoller, dispatcher: std::sync::Arc<D>)
where
    D: crate::triggers::dispatcher::EventDispatcher + 'static,
{
    use tokio::time::{sleep, Duration};
    loop {
        let count = poller
            .poll_once(20, Duration::from_secs(60), |ev| {
                let d = dispatcher.clone();
                async move { d.dispatch(ev).await }
            })
            .await;
        if count == 0 {
            sleep(Duration::from_secs(2)).await;
        }
    }
}

/// Probe the CF Queues consume endpoint at startup so a misconfigured token
/// fails before any trigger fires (rather than silently never delivering).
pub async fn probe_auth(
    account_id: &str,
    queue_id: &str,
    api_token: &str,
) -> Result<(), crate::error::StorageError> {
    let url = format!(
        "https://api.cloudflare.com/client/v4/accounts/{account_id}/queues/{queue_id}/messages/pull"
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| crate::error::StorageError::CfQueueAuthFailed {
            message: format!("reqwest build: {e}"),
        })?;
    // batch_size=1, visibility=1s. We're only probing — if there happens to be
    // a message, we leave it (don't ack) so it redelivers normally.
    let resp = client
        .post(url)
        .bearer_auth(api_token)
        .json(&serde_json::json!({ "batch_size": 1, "visibility_timeout_ms": 1000 }))
        .send()
        .await
        .map_err(|e| crate::error::StorageError::CfQueueAuthFailed {
            message: format!("probe: {e}"),
        })?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(crate::error::StorageError::CfQueueAuthFailed {
            message: format!("HTTP {status}: token missing queue:consume scope?"),
        });
    }
    Ok(())
}

pub fn matches_direction(event: &ObjectEventNormalized, direction: EventKind) -> bool {
    event.event_kind == direction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_queue_poller_constructs() {
        let p = CfQueuePoller::new(
            "acc".into(),
            "qid".into(),
            "tok".into(),
            Arc::new(HashMap::new()),
        );
        assert!(p
            .pull_url()
            .contains("/accounts/acc/queues/qid/messages/pull"));
        assert!(p
            .ack_url()
            .contains("/accounts/acc/queues/qid/messages/ack"));
    }
}
