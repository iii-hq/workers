//! Shared plumbing for the console's observable functions (iii 0.25).
//!
//! Each `console::*::changed` trigger type has an owner-delivered
//! `console::on-*` function next to it. Observers of that function are filed
//! in the trigger type's own registry, next to its bindings, and every event
//! the trigger type hands a binding is handed to each observer with
//! [`Observer::call`]. The trigger types stay registered, unchanged, for
//! consumers that still bind them.
//!
//! Events are routed in-process, never by invoking the function: its handler
//! refuses every invocation with `FORBIDDEN`, so no caller can make this
//! worker hand observers a forged event.

use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::Observer;
use serde_json::Value;

/// The code a direct invocation of an observable function is refused with.
pub const FORBIDDEN: &str = "FORBIDDEN";

/// How long a first delivery waits for the engine to mark a new observer
/// active (the SDK acks it only after `on_observer_added` returns).
const ACTIVATION_WAIT: Duration = Duration::from_secs(5);
const ACTIVATION_POLL: Duration = Duration::from_millis(20);

/// The error an observable function's handler answers every invocation with.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args: {{}} }}, target)); \
             only the console worker delivers its events"
        ),
        stacktrace: None,
    }
}

/// One observer as a sink of a trigger type's registry. Abstracted so the
/// fan-out is unit-testable without an engine.
#[async_trait]
pub trait EventObserver: Send + Sync + 'static {
    /// The observer's id (stable across replays).
    fn id(&self) -> &str;

    /// Whether the engine still delivers to it.
    fn is_active(&self) -> bool {
        true
    }

    /// Hands one event to the observer's target.
    async fn deliver(&self, event: Value) -> Result<(), String>;
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn is_active(&self) -> bool {
        Observer::is_active(self)
    }

    async fn deliver(&self, event: Value) -> Result<(), String> {
        self.call::<Value>(event)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// Delivers `event` once the observer is active: used for the first event a
/// new observer gets, sent from `on_observer_added` before the SDK acks it.
/// Gives up quietly when the observer never becomes active (it was rejected
/// or removed meanwhile).
pub async fn deliver_when_active(observer: &dyn EventObserver, event: Value) {
    let deadline = tokio::time::Instant::now() + ACTIVATION_WAIT;
    while !observer.is_active() {
        if tokio::time::Instant::now() >= deadline {
            tracing::debug!(observer = %observer.id(), "observer never became active; first event dropped");
            return;
        }
        tokio::time::sleep(ACTIVATION_POLL).await;
    }
    if let Err(error) = observer.deliver(event).await {
        tracing::debug!(observer = %observer.id(), %error, "observer delivery failed");
    }
}

/// The registry key of an observer, apart from every trigger id.
pub fn observer_key(id: &str) -> String {
    format!("observer:{id}")
}

/// Schema checks for the observable functions' handlers: the engine refuses a
/// function whose request or response schema is untyped.
#[cfg(test)]
pub(crate) mod schema_check {
    use iii_sdk::errors::Error;
    use schemars::JsonSchema;
    use serde_json::Value;

    /// What a typed JSON schema carries at its root.
    const TYPED_KEYS: [&str; 7] = [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
    ];

    /// The schema the SDK publishes for `T` (draft-07 root schema).
    fn schema_for<T: JsonSchema>() -> Value {
        serde_json::to_value(
            schemars::r#gen::SchemaSettings::draft07()
                .into_generator()
                .into_root_schema_for::<T>(),
        )
        .unwrap()
    }

    /// The (request, response) schemas of a sync handler, as
    /// `RegisterFunction::new(handler)` derives them.
    pub fn handler_schemas<F, T, R>(_handler: &F) -> (Value, Value)
    where
        F: Fn(T) -> Result<R, Error>,
        T: JsonSchema,
        R: JsonSchema,
    {
        (schema_for::<T>(), schema_for::<R>())
    }

    /// Panics unless both schemas of `handler` are typed.
    pub fn assert_typed_handler<F, T, R>(function_id: &str, handler: &F)
    where
        F: Fn(T) -> Result<R, Error>,
        T: JsonSchema,
        R: JsonSchema,
    {
        let (request, response) = handler_schemas(handler);
        for (which, schema) in [("request", request), ("response", response)] {
            assert!(
                TYPED_KEYS.iter().any(|key| schema.get(key).is_some()),
                "{function_id} {which} schema is untyped: {schema}"
            );
        }
    }
}
