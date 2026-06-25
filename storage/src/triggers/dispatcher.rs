//! Fan-out dispatcher: takes a normalized event, looks up subscribers in the
//! registry, calls `iii.trigger()` for each, returns true only if every
//! subscriber acked. Pollers use the boolean return to decide whether to ack
//! the upstream message.

use crate::triggers::normalize::{EventKind, ObjectEventNormalized};
use crate::triggers::registry::TriggerRegistry;
use crate::triggers::{object_created, object_deleted};
use iii_sdk::protocol::TriggerRequest;
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
        if subs.is_empty() {
            return true;
        }
        let mut all_acked = true;
        for sub in subs {
            // Build the function-facing event payload — same shape that the
            // typed `Event` structs in object_{created,deleted}.rs serialize to.
            let payload = match event.event_kind {
                EventKind::Created => {
                    serde_json::to_value(object_created::Event::from(event.clone()))
                }
                EventKind::Deleted => {
                    serde_json::to_value(object_deleted::Event::from(event.clone()))
                }
            };
            let payload = match payload {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "trigger event serialize failed");
                    all_acked = false;
                    continue;
                }
            };
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
                Ok(Ok(value)) => {
                    // Match database's contract: `null` from a void-returning
                    // function counts as ack=true; otherwise the function may
                    // return `{ack: bool}` to control redelivery explicitly.
                    if value.is_null() {
                        true
                    } else {
                        value.get("ack").and_then(|v| v.as_bool()).unwrap_or(true)
                    }
                }
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
        all_acked
    }
}
