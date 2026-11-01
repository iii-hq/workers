//! Turns a normalized object event into a call of `storage::on-object-created`
//! or `storage::on-object-deleted` and answers whether its targets acked.
//! Notifications sources use the answer to decide whether to ack the upstream
//! message.

use crate::events::normalize::{EventKind, ObjectEventNormalized};
use crate::events::object_created::ObjectCreatedEvent;
use crate::events::object_deleted::ObjectDeletedEvent;
use crate::events::source::{self, Delivery};
use iii_sdk::{Call, CallRequest, IIIClient};
use serde_json::{json, Value};

#[async_trait::async_trait]
pub trait EventDispatcher: Send + Sync {
    /// Returns `true` only when every matching target acked. With nothing
    /// observing the bucket it returns `true`, so notifications sources don't
    /// get stuck redelivering messages no one is watching for.
    async fn dispatch(&self, event: ObjectEventNormalized) -> bool;
}

pub struct EngineDispatcher {
    pub iii: IIIClient,
}

impl EngineDispatcher {
    pub fn new(iii: IIIClient) -> Self {
        Self { iii }
    }
}

#[async_trait::async_trait]
impl EventDispatcher for EngineDispatcher {
    async fn dispatch(&self, event: ObjectEventNormalized) -> bool {
        let function_id = source::function_id(event.event_kind);
        // One hash lookup on the bucket's key: an event nobody watches is
        // never built.
        let probe = Call {
            input: json!({ "bucket": event.bucket }),
            ..Call::default()
        };
        if !self.iii.is_observed_for(function_id, &probe) {
            return true;
        }
        let timeout_ms = source::delivery_timeout_ms(&self.iii, function_id, &event.bucket);
        // The function-facing event payload: the shape the typed event structs
        // in object_{created,deleted}.rs serialize to.
        let payload = match event.event_kind {
            EventKind::Created => serde_json::to_value(ObjectCreatedEvent::from(event)),
            EventKind::Deleted => serde_json::to_value(ObjectDeletedEvent::from(event)),
        };
        let payload = match payload {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(function_id, error = %error, "object event serialize failed");
                return false;
            }
        };
        let request = CallRequest {
            timeout_ms: Some(timeout_ms),
            ..CallRequest::new(function_id, payload)
        };
        match self.iii.call(request).await {
            Ok(answer) => acked(&answer),
            Err(error) => {
                tracing::warn!(function_id, error = %error, "object event call failed; not acking");
                false
            }
        }
    }
}

/// The `{ acked }` answer of an object function; anything else is not an
/// ack.
fn acked(answer: &Value) -> bool {
    serde_json::from_value::<Delivery>(answer.clone())
        .map(|delivery| delivery.acked)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_acked_answer_acks() {
        assert!(acked(&json!({ "acked": true })));
        assert!(!acked(&json!({ "acked": false })));
        assert!(!acked(&Value::Null));
        assert!(!acked(&json!({ "ack": true })));
    }
}
