//! The router's three event functions (README § Events).
//!
//! Each is a private function whose handler returns the event it is called
//! with, so its observers observe the event and only this worker can call it:
//!
//! - [`ON_READY_ID`] — once per router start, after boot. An observer added
//!   after that receives the ready event through itself once it is accepted,
//!   so a provider that starts (or observes again) after the router still
//!   re-declares.
//! - [`ON_MODELS_CHANGED_ID`] — a provider reconciled its catalog slice.
//! - [`ON_PROVIDER_CHANGED_ID`] — the provider registry changed.
//!
//! The router calls them only while they are observed, fire-and-forget
//! (`CallAction::Void`): iii delivers each call to every observer whose
//! filters match. Failures are logged and swallowed so a misbehaving observer
//! never breaks the write path.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, Observer, RegisterFunction};
use serde::Serialize;

use crate::surface::{ON_MODELS_CHANGED_DESC, ON_PROVIDER_CHANGED_DESC, ON_READY_DESC};
pub use crate::surface::{ON_MODELS_CHANGED_ID, ON_PROVIDER_CHANGED_ID, ON_READY_ID};
use crate::types::router::{ModelsChangedEvent, ProviderChangedEvent, RouterReadyEvent};

/// Calls the router's event functions; register once at boot, clone into the
/// handlers that report changes.
pub struct RouterEvents {
    iii: IIIClient,
    ready: Arc<AtomicBool>,
}

impl RouterEvents {
    /// Registers the three event functions, and the added hook of
    /// [`ON_READY_ID`] that sends the ready event to an observer added once
    /// the router is ready.
    pub fn register(iii: &IIIClient) -> Arc<Self> {
        let ready = Arc::new(AtomicBool::new(false));

        // Set by id before registering, so it runs for every observer.
        {
            let (iii_for_hook, ready) = (iii.clone(), ready.clone());
            iii.on_observer_added_for(ON_READY_ID, move |observer: Observer| {
                let (iii, ready) = (iii_for_hook.clone(), ready.clone());
                async move {
                    // A call through the observer reaches it only once it is
                    // active, which is after this hook returns.
                    tokio::spawn(send_ready(iii, ready, observer));
                    Ok(())
                }
            });
        }

        iii.register_function(
            ON_READY_ID,
            RegisterFunction::new(|event: RouterReadyEvent| Ok::<_, Error>(event))
                .description(ON_READY_DESC)
                .private(),
        );
        iii.register_function(
            ON_MODELS_CHANGED_ID,
            RegisterFunction::new(|event: ModelsChangedEvent| Ok::<_, Error>(event))
                .description(ON_MODELS_CHANGED_DESC)
                .private(),
        );
        iii.register_function(
            ON_PROVIDER_CHANGED_ID,
            RegisterFunction::new(|event: ProviderChangedEvent| Ok::<_, Error>(event))
                .description(ON_PROVIDER_CHANGED_DESC)
                .private(),
        );
        tracing::info!(
            functions = ?[ON_READY_ID, ON_MODELS_CHANGED_ID, ON_PROVIDER_CHANGED_ID],
            "registered router event functions"
        );

        Arc::new(Self {
            iii: iii.clone(),
            ready,
        })
    }

    /// The router finished booting: mark it ready and call [`ON_READY_ID`]
    /// once. Observers added from here on receive the event through
    /// themselves.
    pub async fn ready(&self) {
        // Before the call: an observer accepted after it was made finds the
        // flag set and receives the event through itself.
        self.ready.store(true, Ordering::SeqCst);
        if self.iii.is_observed(ON_READY_ID) {
            self.call(ON_READY_ID, RouterReadyEvent {}).await;
        }
    }

    /// A provider reconciled its catalog slice to `count` models.
    pub async fn models_changed(&self, provider: &str, count: usize) {
        if !self.iii.is_observed(ON_MODELS_CHANGED_ID) {
            return;
        }
        let event = ModelsChangedEvent {
            provider: provider.to_string(),
            count,
        };
        self.call(ON_MODELS_CHANGED_ID, event).await;
    }

    /// The provider registry changed for `provider`; `op` is `register`,
    /// `discovery`, `available` or `unavailable`.
    pub async fn provider_changed(&self, provider: &str, op: &str) {
        if !self.iii.is_observed(ON_PROVIDER_CHANGED_ID) {
            return;
        }
        let event = ProviderChangedEvent {
            provider: provider.to_string(),
            op: op.to_string(),
        };
        self.call(ON_PROVIDER_CHANGED_ID, event).await;
    }

    /// One fire-and-forget call of `function_id`.
    async fn call(&self, function_id: &str, event: impl Serialize) {
        let payload = match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(function_id, %error, "router event did not serialize");
                return;
            }
        };
        let request = CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = self.iii.call(request).await {
            tracing::warn!(function_id, %error, "router event call failed");
        }
    }
}

/// Sends the ready event to `observer` alone once it is active, when the
/// router is already ready. An observer accepted before that receives the
/// boot-time call of [`ON_READY_ID`] instead.
async fn send_ready(iii: IIIClient, ready: Arc<AtomicBool>, observer: Observer) {
    if let Err(error) = observer.accepted().await {
        tracing::debug!(observer_id = %observer.id(), %error, "router::on-ready observer not accepted");
        return;
    }
    if !ready.load(Ordering::SeqCst) {
        return;
    }
    let Ok(payload) = serde_json::to_value(RouterReadyEvent {}) else {
        return;
    };
    let request = CallRequest::observer(&observer, payload).action(CallAction::Void);
    if let Err(error) = iii.call(request).await {
        tracing::warn!(observer_id = %observer.id(), %error, "router ready event not delivered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface;

    #[test]
    fn event_functions_are_in_the_catalog() {
        let ids: Vec<&str> = surface::catalog().iter().map(|s| s.function_id).collect();
        for id in [ON_READY_ID, ON_MODELS_CHANGED_ID, ON_PROVIDER_CHANGED_ID] {
            assert!(ids.contains(&id), "{id} missing from the surface catalog");
        }
    }

    /// Unobserved, every event is skipped before it reaches the wire (here,
    /// a client without an engine), and `ready` still marks the router ready.
    #[tokio::test]
    async fn unobserved_events_are_not_called() {
        let iii = IIIClient::new("ws://127.0.0.1:1");
        let events = RouterEvents::register(&iii);
        assert!(!iii.is_observed(ON_MODELS_CHANGED_ID));
        events.models_changed("openai", 3).await;
        events.provider_changed("openai", "register").await;
        events.ready().await;
        assert!(events.ready.load(Ordering::SeqCst));
    }
}
