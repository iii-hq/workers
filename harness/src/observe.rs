//! Shared plumbing for the harness's observable functions (iii 0.25).
//!
//! Each event trigger type the harness registers (`harness::ready`,
//! `harness::turn-started`, `harness::turn-completed`,
//! `harness::message-queued`, `harness::triggers-changed`,
//! `harness::compaction`, `harness::session-tree-deletion` and `timer`) has an
//! owner-delivered `harness::on-*` function next to it, whose args are the
//! trigger type's config. Observers of that function are filed in the trigger
//! type's own subscriber set (or bus), next to its bindings, and every event a
//! binding receives is handed to each matching observer with
//! [`Observer::call`]. The trigger types stay registered, unchanged, for
//! consumers that still bind them.
//!
//! Events are routed in-process, never by invoking the function: its handler
//! refuses every invocation with `FORBIDDEN`, so no caller can make the
//! harness hand observers a forged event. The synchronous hook points
//! (`harness::hook::*`) are not observable functions: their result is
//! consumed in-path.

use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, Observe, ObserveHandler, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Codes the `harness::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config, or a
    /// target the trigger type refuses (`harness::spawn`).
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
}

/// How long a delivery waits for the engine to mark a new observer active
/// (the SDK acks it only after `on_observer_added` returns, and a ready or
/// past-deadline event is sent from that callback).
const ACTIVATION_WAIT: Duration = Duration::from_secs(5);
const ACTIVATION_POLL: Duration = Duration::from_millis(20);

/// The error an observable function's handler answers every invocation with.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the harness worker emits its events"
        ),
        stacktrace: None,
    }
}

/// One observer of a `harness::on-*` function, as the emitter sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
pub trait EventObserver: Send + Sync + 'static {
    /// The observer's id (stable across replays).
    fn id(&self) -> &str;

    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// `TriggerAction::Void` delivery: never blocks the emitter.
    fn deliver(&self, event: Value);
}

impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn deliver(&self, event: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + ACTIVATION_WAIT;
            while !obs.is_active() {
                if tokio::time::Instant::now() >= deadline {
                    tracing::debug!(observer = %obs.id(), function_id = %obs.function_id(), "observer not active; event dropped");
                    return;
                }
                tokio::time::sleep(ACTIVATION_POLL).await;
            }
            match obs.call::<Value>(event).await {
                Ok(_) => {}
                // The observer's own filters declined this event.
                Err(Error::Remote { code, .. }) if code == "FILTERS_NOT_MATCHED" => {}
                Err(error) => {
                    tracing::warn!(observer = %obs.id(), function_id = %obs.function_id(), %error, "observer delivery failed");
                }
            }
        });
    }
}

/// The key an observer is filed under where a registry also holds trigger
/// ids (the timer bus), so the two never collide.
pub fn observer_key(id: &str) -> String {
    format!("observer:{id}")
}

/// Registers `function_id` as an owner-delivered observable function whose
/// args are `A` (the trigger type's config) and whose request format is the
/// event `E`. Its handler refuses every invocation; `handler` files the
/// observers. On an engine without observe it registers as a plain function
/// and its hooks never run. The returned handle unregisters it.
pub fn register_observable<A, E, H>(
    iii: &IIIClient,
    function_id: &'static str,
    description: &str,
    handler: H,
) -> FunctionRef
where
    A: JsonSchema,
    E: DeserializeOwned + JsonSchema + Send + 'static,
    H: ObserveHandler,
{
    let function = iii.register_function(
        function_id,
        RegisterFunction::new(move |_event: E| -> Result<(), Error> {
            Err(refuse_invocation(function_id))
        })
        .description(description)
        .observe(Observe::owner().args_format::<A>().handler(handler)),
    );
    tracing::info!(function_id, "registered observable function");
    function
}
