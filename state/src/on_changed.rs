//! `state::on-changed`: one call per committed state change.
//!
//! A private function whose handler returns the change event it is called
//! with, so observers observe the event as the call's `input` and `output`.
//! It takes no args and has no hooks: iii delivers every call to each
//! observer whose filters match, and observers select with filters on
//! `input.scope`, `input.key` and `input.event_type`
//! (`iii.observe({ function_id: 'state::on-changed', filters }, target)`).
//!
//! After every committed change [`StateCtx`](crate::functions::StateCtx)
//! calls the function from the writing handler's own future, fire-and-forget,
//! so the call runs in-process and its `caller` is that handler
//! (`caller.function_id`) with the worker that called it as
//! `caller.parent`. With nobody observing, nothing is built or called
//! ([`ChangeEmitter::is_observed`], one atomic load). Being private, only
//! this worker may call it: the engine refuses any other caller with
//! `FORBIDDEN`.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use serde_json::Value;

use crate::structs::{StateEventData, StateEventType};

pub const ON_CHANGED_FN: &str = "state::on-changed";

pub const ON_CHANGED_DESCRIPTION: &str = "One call per committed state change (state::set, state::update, state::delete, a \
     swapped state::compare-and-set). The handler returns the change event it is called \
     with, { type, event_type, scope, key, old_value, new_value }, so observers observe it \
     as input and output. Observe it with no args and filter on input.scope / input.key / \
     input.event_type; caller.function_id is the state function that wrote, and \
     caller.parent the worker that called it. Private: only the state worker calls it. \
     Private namespaces never emit, and `events_enabled: false` silences it.";

/// Where change events go on their way to `state::on-changed`. Abstracts the
/// SDK so the emission gate is unit-testable without an engine.
#[async_trait::async_trait]
pub trait ChangeEmitter: Send + Sync + 'static {
    /// Whether any observer is active on `state::on-changed` (one atomic load).
    fn is_observed(&self) -> bool;

    /// Calls `state::on-changed` with `event` as its input.
    async fn emit(&self, event: Value);
}

/// The SDK-backed emitter: a fire-and-forget local call of
/// `state::on-changed`, gated on its [`FunctionRef`].
pub struct SdkChangeEmitter {
    pub iii: Arc<IIIClient>,
    pub function: FunctionRef,
}

#[async_trait::async_trait]
impl ChangeEmitter for SdkChangeEmitter {
    fn is_observed(&self) -> bool {
        self.function.is_observed()
    }

    async fn emit(&self, event: Value) {
        // Called from the writing handler's own future: the call's caller is
        // that handler (caller.function_id) and its caller (caller.parent).
        let request = CallRequest {
            function_id: ON_CHANGED_FN.to_string(),
            payload: event,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = self.iii.call(request).await {
            tracing::warn!(function_id = ON_CHANGED_FN, %error, "state change not emitted");
        }
    }
}

/// The event schema, for both the request and the response: the handler
/// returns what it is called with.
fn event_format() -> Value {
    serde_json::to_value(schemars::schema_for!(StateEventData)).unwrap_or(Value::Null)
}

/// Registers `state::on-changed`: private, and its handler returns the event
/// it is called with. The handler takes the event as JSON so a local call
/// passes it through by move, without a second copy of `old_value` and
/// `new_value`.
pub fn register(iii: &IIIClient) -> FunctionRef {
    iii.register_function(
        ON_CHANGED_FN,
        RegisterFunction::new(|event: Value| Ok::<_, Error>(event))
            .description(ON_CHANGED_DESCRIPTION)
            .request_format(event_format())
            .response_format(event_format())
            .private(),
    )
}

/// Calls `state::on-changed` for one change, building the event only when
/// someone observes it and change events are enabled.
pub(crate) async fn emit_lazy<F>(emitter: &dyn ChangeEmitter, enabled: bool, build: F)
where
    F: FnOnce() -> StateEventData,
{
    if !enabled || !emitter.is_observed() {
        return;
    }
    emitter.emit(event_json(build())).await;
}

/// The event as JSON. Every field already holds JSON or a string: move them
/// rather than serializing `old_value` / `new_value` into a second identical
/// tree.
pub(crate) fn event_json(event: StateEventData) -> Value {
    let kind = match event.event_type {
        StateEventType::Created => "state:created",
        StateEventType::Updated => "state:updated",
        StateEventType::Deleted => "state:deleted",
    };
    let mut object = serde_json::Map::new();
    object.insert("type".into(), Value::String(event.message_type));
    object.insert("event_type".into(), Value::String(kind.into()));
    object.insert("scope".into(), Value::String(event.scope));
    object.insert("key".into(), Value::String(event.key));
    object.insert("old_value".into(), event.old_value.unwrap_or(Value::Null));
    object.insert("new_value".into(), event.new_value);
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct RecordingEmitter {
        observed: AtomicBool,
        emitted: tokio::sync::Mutex<Vec<Value>>,
    }

    #[async_trait::async_trait]
    impl ChangeEmitter for RecordingEmitter {
        fn is_observed(&self) -> bool {
            self.observed.load(Ordering::SeqCst)
        }
        async fn emit(&self, event: Value) {
            self.emitted.lock().await.push(event);
        }
    }

    fn change() -> StateEventData {
        StateEventData {
            message_type: "state".to_string(),
            event_type: StateEventType::Updated,
            scope: "orders".to_string(),
            key: "o1".to_string(),
            old_value: Some(serde_json::json!({"status": "pending"})),
            new_value: serde_json::json!({"status": "shipped"}),
        }
    }

    #[tokio::test]
    async fn unobserved_or_disabled_never_builds_or_emits() {
        let emitter = RecordingEmitter::default();
        emit_lazy(&emitter, true, || panic!("built while unobserved")).await;
        emitter.observed.store(true, Ordering::SeqCst);
        emit_lazy(&emitter, false, || panic!("built while disabled")).await;
        assert!(emitter.emitted.lock().await.is_empty());
    }

    #[tokio::test]
    async fn observed_emits_the_event_document() {
        let emitter = RecordingEmitter::default();
        emitter.observed.store(true, Ordering::SeqCst);
        emit_lazy(&emitter, true, change).await;
        let emitted = emitter.emitted.lock().await;
        assert_eq!(emitted.len(), 1);
        assert_eq!(
            emitted[0],
            serde_json::to_value(change()).unwrap(),
            "the call's input is the serialized event"
        );
        assert_eq!(
            emitted[0],
            serde_json::json!({
                "type": "state",
                "event_type": "state:updated",
                "scope": "orders",
                "key": "o1",
                "old_value": {"status": "pending"},
                "new_value": {"status": "shipped"},
            })
        );
    }

    #[test]
    fn moved_event_matches_the_serialized_shape() {
        for kind in [
            StateEventType::Created,
            StateEventType::Updated,
            StateEventType::Deleted,
        ] {
            let e = StateEventData {
                message_type: "state".into(),
                event_type: kind,
                scope: "s".into(),
                key: "k".into(),
                old_value: Some(serde_json::json!({"a":[null,true,"中文"]})),
                new_value: Value::Null,
            };
            let expected = serde_json::to_value(&e).unwrap();
            assert_eq!(event_json(e), expected);
        }
    }

    #[test]
    fn the_formats_describe_the_event() {
        let format = event_format();
        let properties = format["properties"].as_object().expect("object schema");
        for field in [
            "type",
            "event_type",
            "scope",
            "key",
            "old_value",
            "new_value",
        ] {
            assert!(properties.contains_key(field), "missing {field}");
        }
    }
}
