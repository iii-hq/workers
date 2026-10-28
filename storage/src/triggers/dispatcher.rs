//! Fan-out dispatcher: takes a normalized event, looks up subscribers in the
//! registry, calls `iii.trigger()` for each binding and `obs.call` for each
//! observer (same payload), returns true only if every subscriber acked.
//! Pollers use the boolean return to decide whether to ack the upstream
//! message.

use crate::triggers::normalize::{EventKind, ObjectEventNormalized};
use crate::triggers::registry::TriggerRegistry;
use crate::triggers::{object_created, object_deleted};
use iii_sdk::errors::Error;
use iii_sdk::protocol::{observe_codes, TriggerRequest};
use iii_sdk::IIIClient;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

#[async_trait::async_trait]
pub trait EventDispatcher: Send + Sync {
    /// Returns `true` only when every registered subscriber acked. With zero
    /// subscribers we still return `true` so pollers don't get stuck redelivering
    /// messages no one is listening for.
    async fn dispatch(&self, event: ObjectEventNormalized) -> bool;
}

pub struct EngineDispatcher {
    pub iii: IIIClient,
    pub registry: Arc<TriggerRegistry>,
}

impl EngineDispatcher {
    pub fn new(iii: IIIClient, registry: Arc<TriggerRegistry>) -> Self {
        Self { iii, registry }
    }
}

#[async_trait::async_trait]
impl EventDispatcher for EngineDispatcher {
    async fn dispatch(&self, event: ObjectEventNormalized) -> bool {
        let subs = self
            .registry
            .subscribers_for(&event.bucket, event.event_kind);
        let observers = self.registry.observers_for(&event.bucket, event.event_kind);
        if subs.is_empty() && observers.is_empty() {
            return true;
        }
        // The function-facing event payload — same shape that the typed
        // `Event` structs in object_{created,deleted}.rs serialize to, for
        // bindings and observers alike.
        let payload = match event_payload(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "trigger event serialize failed");
                return false;
            }
        };
        let mut all_acked = true;
        for sub in subs {
            let payload = payload.clone();
            let req = TriggerRequest {
                function_id: sub.function_id.clone(),
                payload,
                action: None,
                timeout_ms: Some(sub.handler_timeout_ms),
            };
            // Bound the wait at the configured handler_timeout_ms; the engine
            // itself enforces the same, but the local timeout protects against
            // a stuck connection.
            let fut = self.iii.trigger(req);
            let acked = match timeout(
                Duration::from_millis(sub.handler_timeout_ms.saturating_add(1_000)),
                fut,
            )
            .await
            {
                // Match database's ack contract (see `ack_of`).
                Ok(Ok(value)) => ack_of(&value),
                Ok(Err(e)) => {
                    tracing::warn!(
                        function_id = %sub.function_id,
                        error = %e,
                        "trigger dispatch errored; not acking"
                    );
                    false
                }
                Err(_) => {
                    tracing::warn!(
                        function_id = %sub.function_id,
                        timeout_ms = sub.handler_timeout_ms,
                        "trigger dispatch timed out; not acking"
                    );
                    false
                }
            };
            if !acked {
                all_acked = false;
            }
        }
        for obs in observers {
            let acked = match timeout(
                Duration::from_millis(obs.handler_timeout_ms.saturating_add(1_000)),
                obs.observer
                    .deliver(&payload, Duration::from_millis(obs.handler_timeout_ms)),
            )
            .await
            {
                Ok(Ok(value)) => ack_of(&value),
                // The observer's filters rejected the event, or it was removed
                // meanwhile: nothing was sent, nothing to redeliver.
                Ok(Err(Error::Remote { code, .. }))
                    if code == observe_codes::FILTERS_NOT_MATCHED
                        || code == observe_codes::OBSERVER_NOT_ACTIVE =>
                {
                    true
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        observer = %obs.observer.id(),
                        error = %e,
                        "observer delivery errored; not acking"
                    );
                    false
                }
                Err(_) => {
                    tracing::warn!(
                        observer = %obs.observer.id(),
                        timeout_ms = obs.handler_timeout_ms,
                        "observer delivery timed out; not acking"
                    );
                    false
                }
            };
            if !acked {
                all_acked = false;
            }
        }
        all_acked
    }
}

/// The payload a binding and an observer receive for `event`.
pub fn event_payload(event: &ObjectEventNormalized) -> serde_json::Result<serde_json::Value> {
    match event.event_kind {
        EventKind::Created => serde_json::to_value(object_created::Event::from(event.clone())),
        EventKind::Deleted => serde_json::to_value(object_deleted::Event::from(event.clone())),
    }
}

/// `null` from a void-returning function counts as ack=true; otherwise the
/// function may return `{ack: bool}` to control redelivery explicitly.
fn ack_of(value: &serde_json::Value) -> bool {
    if value.is_null() {
        true
    } else {
        value.get("ack").and_then(|v| v.as_bool()).unwrap_or(true)
    }
}
