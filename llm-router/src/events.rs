//! The router's three event functions (README § Events).
//!
//! Each is a private function whose handler returns the event it is called
//! with, so its observers observe the event and only this worker can call it:
//!
//! - [`ON_READY_ID`] — the router is ready. Each observer receives the event
//!   once the router has booted and the observer is accepted: once per router
//!   start for an observer that stays, and once more each time it is added
//!   again (its worker reconnected within the observer grace period). So a
//!   provider that starts, or observes again, after the router still
//!   re-declares.
//! - [`ON_MODELS_CHANGED_ID`] — a provider reconciled its catalog slice.
//! - [`ON_PROVIDER_CHANGED_ID`] — the provider registry changed.
//!
//! The router calls them only while they are observed, fire-and-forget
//! (`CallAction::Void`): iii delivers each call to every observer whose
//! filters match. Failures are logged and swallowed so a misbehaving observer
//! never breaks the write path.
//!
//! The ready event is a call for one observer: a call of [`ON_READY_ID`] with
//! metadata `{ "for_observer": <observer id> }`, which the function's match
//! hook gives to that observer alone. iii delivers it like every call, with
//! the observer's `deliver`, `capture` and filters.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, Match, Observer, RegisterFunction};
use serde::Serialize;
use serde_json::json;
use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::surface::{ON_MODELS_CHANGED_DESC, ON_PROVIDER_CHANGED_DESC, ON_READY_DESC};
pub use crate::surface::{ON_MODELS_CHANGED_ID, ON_PROVIDER_CHANGED_ID, ON_READY_ID};
use crate::types::router::{ModelsChangedEvent, ProviderChangedEvent, RouterReadyEvent};

/// The metadata key of a call for one observer.
const FOR_OBSERVER: &str = "for_observer";

/// Calls the router's event functions; register once at boot, clone into the
/// handlers that report changes.
pub struct RouterEvents {
    iii: IIIClient,
    ready: watch::Sender<bool>,
}

/// The task that sends one observer of [`ON_READY_ID`] its ready event, by
/// observer id, held from the observer's added hook to its removed hook. The
/// removed hook aborts the task, so an observer removed before its event went
/// out and added again (its worker reconnected within the observer grace
/// period) receives the event once: the SDK runs an observer's removed hooks
/// before it adds an observer with the same id again.
type ReadyTasks = Arc<Mutex<HashMap<String, AbortHandle>>>;

impl RouterEvents {
    /// Registers the three event functions, and the hooks of [`ON_READY_ID`]
    /// that send each of its observers the ready event once the router is
    /// ready.
    pub fn register(iii: &IIIClient) -> Arc<Self> {
        let (ready, is_ready) = watch::channel(false);
        let tasks: ReadyTasks = Arc::default();

        // Set by id before registering, so they run for every observer.
        {
            let (iii_for_hook, tasks) = (iii.clone(), tasks.clone());
            iii.on_observer_added_for(ON_READY_ID, move |observer: Observer| {
                let (iii, ready, tasks) = (iii_for_hook.clone(), is_ready.clone(), tasks.clone());
                async move {
                    // A call for the observer reaches it only once it is
                    // active, which is after this hook returns.
                    let id = observer.id().to_string();
                    let task = tokio::spawn(send_ready(iii, ready, observer)).abort_handle();
                    lock(&tasks).insert(id, task);
                    Ok(())
                }
            });
        }
        {
            let tasks = tasks.clone();
            iii.on_observer_removed_for(ON_READY_ID, move |observer: Observer| {
                let tasks = tasks.clone();
                async move {
                    if let Some(task) = lock(&tasks).remove(observer.id()) {
                        task.abort();
                    }
                }
            });
        }
        iii.on_observer_match_for(ON_READY_ID, Match::predicate(for_this_observer));

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

    /// The router finished booting: every observer of [`ON_READY_ID`] that is
    /// accepted, and every one accepted from here on, receives the ready
    /// event once.
    pub async fn ready(&self) {
        self.ready.send_replace(true);
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

/// The match hook of [`ON_READY_ID`]: a call for one observer goes to that
/// observer alone, any other call to every observer.
fn for_this_observer(observer: &Observer, call: &iii_sdk::Call) -> bool {
    call.metadata
        .as_ref()
        .and_then(|metadata| metadata.get(FOR_OBSERVER))
        .and_then(|id| id.as_str())
        .is_none_or(|id| id == observer.id())
}

/// Sends `observer` its ready event once it is accepted and the router is
/// ready: a call of [`ON_READY_ID`] for this observer alone. A rejected
/// observer gets nothing.
async fn send_ready(iii: IIIClient, mut ready: watch::Receiver<bool>, observer: Observer) {
    if let Err(error) = observer.accepted().await {
        tracing::debug!(observer_id = %observer.id(), %error, "router::on-ready observer not accepted");
        return;
    }
    if ready.wait_for(|ready| *ready).await.is_err() {
        return;
    }
    let Ok(payload) = serde_json::to_value(RouterReadyEvent {}) else {
        return;
    };
    let request = CallRequest::new(ON_READY_ID, payload)
        .metadata(json!({ FOR_OBSERVER: observer.id() }))
        .action(CallAction::Void);
    if let Err(error) = iii.call(request).await {
        tracing::warn!(observer_id = %observer.id(), %error, "router ready event not delivered");
    }
}

fn lock(tasks: &ReadyTasks) -> std::sync::MutexGuard<'_, HashMap<String, AbortHandle>> {
    tasks.lock().unwrap_or_else(PoisonError::into_inner)
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
        assert!(!*events.ready.borrow());
        events.ready().await;
        assert!(*events.ready.borrow());
    }
}
