//! Fan-out a normalized `Event` to every subscriber registered for its
//! (account, folder). Mirrors `storage::triggers::dispatcher`.
//!
//! `email::new-mail` bindings are called with `iii.trigger`, then each
//! `email::on-new-mail` observer of the same (account, folder) gets the same
//! payload through `obs.call` (iii 0.25).

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use super::registry::{ObserverSub, TriggerRegistry};
use super::Event;

#[async_trait]
pub trait EventDispatcher: Send + Sync {
    async fn dispatch(&self, event: Event);
}

pub struct EngineDispatcher {
    iii: Arc<IIIClient>,
    registry: Arc<TriggerRegistry>,
}

impl EngineDispatcher {
    pub fn new(iii: Arc<IIIClient>, registry: Arc<TriggerRegistry>) -> Self {
        Self { iii, registry }
    }
}

#[async_trait]
impl EventDispatcher for EngineDispatcher {
    async fn dispatch(&self, event: Event) {
        let subs = self.registry.subscribers_for(&event.account, &event.folder);
        let observers = self.registry.observers_for(&event.account, &event.folder);
        if subs.is_empty() && observers.is_empty() {
            return;
        }
        let payload = event.payload();
        for sub in subs {
            let req = TriggerRequest {
                function_id: sub.function_id.clone(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(sub.handler_timeout_ms),
            };
            let bound = Duration::from_millis(sub.handler_timeout_ms.saturating_add(1_000));
            let namespace = sub.namespace.as_deref().unwrap_or("default");
            let result = timeout(bound, self.iii.trigger(req.namespace(namespace))).await;
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::warn!(
                        function_id = %sub.function_id,
                        error = %e,
                        "email::new-mail subscriber returned error"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        function_id = %sub.function_id,
                        timeout_ms = sub.handler_timeout_ms,
                        "email::new-mail subscriber timed out"
                    );
                }
            }
        }
        deliver_to_observers(observers, &payload).await;
    }
}

/// Hand `payload` to each observer with `obs.call`, sequentially, each bound
/// by its `handler_timeout_ms`. Failures are logged and skipped, as for
/// bindings; a payload an observer's filters reject (or one removed
/// meanwhile) sent nothing and is not a failure.
pub async fn deliver_to_observers(observers: Vec<ObserverSub>, payload: &Value) {
    for sub in observers {
        let call_timeout = Duration::from_millis(sub.handler_timeout_ms);
        let bound = Duration::from_millis(sub.handler_timeout_ms.saturating_add(1_000));
        let result = timeout(bound, sub.observer.deliver(payload.clone(), call_timeout)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) if is_skipped_delivery(&e) => {
                tracing::debug!(
                    observer = %sub.observer.id(),
                    error = %e,
                    "email::on-new-mail observer skipped the event"
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    observer = %sub.observer.id(),
                    error = %e,
                    "email::on-new-mail observer delivery failed"
                );
            }
            Err(_) => {
                tracing::warn!(
                    observer = %sub.observer.id(),
                    timeout_ms = sub.handler_timeout_ms,
                    "email::on-new-mail observer timed out"
                );
            }
        }
    }
}

/// The observer's filters rejected the payload, or it was removed meanwhile:
/// nothing was sent.
fn is_skipped_delivery(error: &Error) -> bool {
    matches!(
        error,
        Error::Remote { code, .. }
            if code == observe_codes::FILTERS_NOT_MATCHED
                || code == observe_codes::OBSERVER_NOT_ACTIVE
    )
}
