//! Debounced per-provider follow-ups after a configuration change
//! (`router::on_config_changed`) or a credential change
//! (`router::on_secret_changed`, a secret resolving differently). One quiet
//! period coalesces a burst — the paste-a-key flow writes the secret and the
//! slice back to back — into one call per provider.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::json;

/// What a provider is asked to do; the stronger request wins when both are
/// pending for the same provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refresh {
    /// `provider::<id>::refresh_models`: the provider's slice changed.
    Models,
    /// `provider::<id>::on_router_ready`: the credential behind the slice
    /// changed. The provider drops its cached resolve (otherwise stale for up
    /// to `provider_scaffold::cache::RESOLVE_TTL`), re-declares and refreshes
    /// its models — the same per-provider nudge a router restart sends.
    Credential,
}

impl Refresh {
    pub fn function_id(self, provider: &str) -> String {
        match self {
            Refresh::Models => format!("provider::{provider}::refresh_models"),
            Refresh::Credential => format!("provider::{provider}::on_router_ready"),
        }
    }

    /// The ready nudge is bounded like `registry::rediscover`'s, so one slow
    /// provider cannot hold up the rest of the flush.
    fn timeout_ms(self) -> Option<u64> {
        match self {
            Refresh::Models => None,
            Refresh::Credential => Some(10_000),
        }
    }
}

/// Delivers one follow-up; fire-and-forget (discovery reports back through
/// `router::models::reconcile`).
pub type Fire = Arc<dyn Fn(String, Refresh) -> BoxFuture<'static, ()> + Send + Sync>;

pub fn bus_fire(iii: IIIClient) -> Fire {
    Arc::new(move |provider: String, refresh: Refresh| {
        let iii = iii.clone();
        Box::pin(async move {
            let _ = iii
                .trigger(TriggerRequest {
                    function_id: refresh.function_id(&provider),
                    payload: json!({}),
                    action: None,
                    timeout_ms: refresh.timeout_ms(),
                })
                .await;
        })
    })
}

#[derive(Clone)]
pub struct RefreshQueue {
    fire: Fire,
    debounce: Duration,
    pending: Arc<Mutex<BTreeMap<String, Refresh>>>,
    generation: Arc<AtomicU64>,
}

impl RefreshQueue {
    pub fn new(fire: Fire, debounce: Duration) -> Self {
        Self {
            fire,
            debounce,
            pending: Arc::default(),
            generation: Arc::default(),
        }
    }

    /// Queue follow-ups and (re-)arm the flush. With nothing queued and
    /// nothing pending this is a no-op.
    pub fn schedule(&self, items: impl IntoIterator<Item = (String, Refresh)>) {
        {
            let mut pending = self.pending.lock().unwrap();
            for (provider, refresh) in items {
                let slot = pending.entry(provider).or_insert(refresh);
                *slot = (*slot).max(refresh);
            }
            if pending.is_empty() {
                return;
            }
        }

        // Debounce: each call arms a fresh flush; a superseded task notices
        // via the generation check and exits BEFORE draining. Never abort a
        // task — one past its drain has sole custody of the taken ids (their
        // fingerprints already advanced), so a mid-flush kill would silently
        // drop refreshes.
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let queue = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(queue.debounce).await;
            if queue.generation.load(Ordering::SeqCst) != generation {
                return; // a newer call re-armed the flush; it owns the drain
            }
            let due = std::mem::take(&mut *queue.pending.lock().unwrap());
            for (provider, refresh) in due {
                (queue.fire)(provider, refresh).await;
            }
        });
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// A [`RefreshQueue`] whose deliveries land in the returned log.
    pub fn recording_queue(debounce: Duration) -> (RefreshQueue, Arc<Mutex<Vec<String>>>) {
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = log.clone();
        let fire: Fire = Arc::new(move |provider: String, refresh: Refresh| {
            sink.lock().unwrap().push(refresh.function_id(&provider));
            Box::pin(async {})
        });
        (RefreshQueue::new(fire, debounce), log)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::recording_queue;
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn coalesces_a_burst_and_keeps_the_stronger_refresh() {
        let (queue, log) = recording_queue(Duration::from_secs(2));
        queue.schedule([("a".to_string(), Refresh::Models)]);
        tokio::time::sleep(Duration::from_secs(1)).await;
        queue.schedule([
            ("a".to_string(), Refresh::Credential),
            ("b".to_string(), Refresh::Models),
        ]);
        queue.schedule([("a".to_string(), Refresh::Models)]);
        tokio::time::sleep(Duration::from_millis(1999)).await;
        assert!(
            log.lock().unwrap().is_empty(),
            "still inside the quiet period"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                "provider::a::on_router_ready".to_string(),
                "provider::b::refresh_models".to_string(),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_queued_arms_nothing() {
        let (queue, log) = recording_queue(Duration::from_millis(10));
        queue.schedule(std::iter::empty());
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(log.lock().unwrap().is_empty());
    }
}
