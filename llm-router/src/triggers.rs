//! Router events (README § Events) ride the engine's built-in `iii-pubsub`
//! worker: `publish` fans each event out to every function bound to the topic
//! through the `subscribe` trigger type. The engine delivers the payload
//! verbatim (no envelope) and isolates subscriber failures per delivery, so
//! the router only publishes.
use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

/// Fires when a provider reconciles its catalog slice.
pub const MODELS_CHANGED: &str = "router::models::changed";
/// Fires when the provider registry changes (declare / availability flip).
pub const PROVIDER_CHANGED: &str = "router::provider::changed";
/// Fires once when the router finishes booting; providers re-declare on it.
pub const READY: &str = "router::ready";

/// Publish one router event: awaits the engine's ack, swallows errors —
/// delivery to subscribers is fire-and-forget either way.
pub async fn publish(iii: &III, topic: &str, data: Value) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "publish".into(),
            payload: json!({ "topic": topic, "data": data }),
            action: None,
            timeout_ms: None,
        })
        .await;
}
