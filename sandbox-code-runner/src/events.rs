//! The `sandbox-code-runner::event` trigger type this worker emits, and the
//! subscriber registry behind it.
//!
//! After a runtime lifecycle change — a persistent runtime comes up, a run
//! settles, a bus function is registered, a teardown lands —
//! `RuntimeManager` fires one fire-and-forget event (`TriggerAction::Void`)
//! to every subscriber. Subscribers (the console page) bind a trigger
//! instance of the type with the standard two-step pattern; the engine
//! routes each registration through [`EventTriggerHandler`], which stashes
//! the subscriber in [`SubscriberSet`]. Emission is best-effort: a slow or
//! absent subscriber never blocks or fails the real call — the mirror of
//! the `worktree` / `memory` worker fan-out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use serde::Serialize;
use serde_json::Value;

use crate::runner::Lang;

/// The trigger type the console page subscribes to.
pub const EVENT: &str = "sandbox-code-runner::event";
pub const EVENT_DESC: &str = "Fires on runtime lifecycle changes: a persistent runtime came up \
     (runtime_created), a run finished (run_settled), a bus function was \
     registered (function_registered), a teardown landed (teardown), or \
     the daemon fleet changed (fleet — carries the full sandbox::list \
     snapshot, pushed so subscribers never poll). \
     Payload: { kind, runtime_id?, lang?, sandbox_id?, function_id?, \
     daemon_absent?, sandboxes? }.";

/// One lifecycle event. Kind-discriminated; every other field is present
/// only when that kind carries it — never code, source, or output. No
/// `Debug` impl on purpose: `runtime_id` and `sandbox_id` must never reach
/// logs (they travel only to subscribers, who are the operator surface).
#[derive(Clone, Serialize)]
pub struct Event {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<Lang>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon_absent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandboxes: Option<serde_json::Value>,
}

impl Event {
    /// A runtime that OUTLIVES its creating call came up: `run keep=true`,
    /// or a namespace's first registration. One-shot run VMs never emit.
    pub fn runtime_created(runtime_id: &str, lang: Lang, sandbox_id: &str) -> Self {
        Self {
            kind: "runtime_created",
            runtime_id: Some(runtime_id.to_string()),
            lang: Some(lang),
            sandbox_id: Some(sandbox_id.to_string()),
            function_id: None,
            daemon_absent: None,
            sandboxes: None,
        }
    }

    /// A run returned a response. `runtime_id` only when the runtime
    /// persists past the call — a one-shot run's VM is already gone.
    pub fn run_settled(runtime_id: Option<&str>, lang: Lang) -> Self {
        Self {
            kind: "run_settled",
            runtime_id: runtime_id.map(str::to_string),
            lang: Some(lang),
            sandbox_id: None,
            function_id: None,
            daemon_absent: None,
            sandboxes: None,
        }
    }

    /// `register_function` published `function_id` onto `runtime_id`.
    pub fn function_registered(function_id: &str, runtime_id: &str, lang: Lang) -> Self {
        Self {
            kind: "function_registered",
            runtime_id: Some(runtime_id.to_string()),
            lang: Some(lang),
            sandbox_id: None,
            function_id: Some(function_id.to_string()),
            daemon_absent: None,
            sandboxes: None,
        }
    }

    /// One runtime was destroyed — by `teardown` (a namespace teardown
    /// emits one event per runtime it destroys), or by `expire` when the
    /// daemon reaped or lost the VM behind our back.
    pub fn teardown(runtime_id: &str) -> Self {
        Self {
            kind: "teardown",
            runtime_id: Some(runtime_id.to_string()),
            lang: None,
            sandbox_id: None,
            function_id: None,
            daemon_absent: None,
            sandboxes: None,
        }
    }

    /// The daemon fleet changed: `sandboxes` is the FULL current
    /// `sandbox::list` snapshot (small by construction — the daemon caps
    /// concurrent sandboxes), `daemon_absent` flags an engine with no
    /// sandbox daemon. Emitted by the fleet watcher (fleet_watch.rs) so
    /// console tabs subscribe instead of each polling `sandbox::list`.
    pub fn fleet(daemon_absent: bool, sandboxes: serde_json::Value) -> Self {
        Self {
            kind: "fleet",
            runtime_id: None,
            lang: None,
            sandbox_id: None,
            function_id: None,
            daemon_absent: Some(daemon_absent),
            sandboxes: Some(sandboxes),
        }
    }
}

/// Thread-safe subscriber registry keyed by trigger-instance id. Cloned into
/// the [`EventTriggerHandler`] (mutates on register/unregister) and the
/// [`Emitter`] fan-out (iterates read-only).
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl SubscriberSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&self, id: String, function_id: String) {
        self.lock().insert(id, function_id);
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// Snapshot of the current subscriber `function_id`s so the mutex is
    /// never held across delivery.
    fn function_ids(&self) -> Vec<String> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct EventTriggerHandler {
    subscribers: SubscriberSet,
}

// Hand-expanded `#[async_trait]` impl: this worker deliberately carries no
// async-trait dependency, and both methods' real work is synchronous —
// `BoxFuture` is the exact type the macro would generate.
impl TriggerHandler for EventTriggerHandler {
    fn register_trigger<'life0, 'async_trait>(
        &'life0 self,
        config: TriggerConfig,
    ) -> BoxFuture<'async_trait, Result<(), Error>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        tracing::info!(
            trigger_type = EVENT,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config.id, config.function_id);
        Box::pin(async { Ok(()) })
    }

    fn unregister_trigger<'life0, 'async_trait>(
        &'life0 self,
        config: TriggerConfig,
    ) -> BoxFuture<'async_trait, Result<(), Error>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        tracing::info!(trigger_type = EVENT, id = %config.id, "trigger subscription unregistered");
        self.subscribers.remove(&config.id);
        Box::pin(async { Ok(()) })
    }
}

/// How a matched event reaches a subscriber. Production delivers over the
/// bus; tests record.
pub trait EventDeliverer: Send + Sync {
    fn deliver(&self, function_id: &str, payload: Value);
}

/// Ceiling on one subscriber delivery: an unresponsive subscriber fails the
/// trigger here instead of parking its detached task forever.
const DELIVERY_TIMEOUT_MS: u64 = 10_000;

/// Fire-and-forget bus delivery, detached onto its own task so the handler
/// that produced the event never waits on a subscriber round trip and a
/// slow subscriber cannot delay its siblings. Failures are logged and
/// swallowed — the real call already returned.
pub struct IiiDeliverer {
    iii: Arc<IIIClient>,
}

impl IiiDeliverer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

impl EventDeliverer for IiiDeliverer {
    fn deliver(&self, function_id: &str, payload: Value) {
        let iii = self.iii.clone();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let res = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload,
                    action: Some(TriggerAction::Void),
                    timeout_ms: Some(DELIVERY_TIMEOUT_MS),
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(function_id = %function_id, error = %e, "sandbox-code-runner::event fan-out failed");
            }
        });
    }
}

/// Fans each event out to every current subscriber. No subscribers ⇒ no
/// work at all.
#[derive(Clone)]
pub struct Emitter {
    subscribers: SubscriberSet,
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(subscribers: SubscriberSet, deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self {
            subscribers,
            deliverer,
        }
    }

    pub fn emit(&self, event: Event) {
        let targets = self.subscribers.function_ids();
        if targets.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "sandbox-code-runner::event payload failed to serialize");
                return;
            }
        };
        for function_id in targets {
            self.deliverer.deliver(&function_id, payload.clone());
        }
    }
}

/// Register the `sandbox-code-runner::event` trigger type with the engine
/// and return the emitter wired to its subscriber set. An emission, not a
/// callable function: deliberately NOT in `functions::STATIC_IDS` or
/// `catalog()` — nothing invokes it, so there is no wire schema to pin.
pub fn register_event_trigger(iii: &Arc<IIIClient>) -> Emitter {
    let subscribers = SubscriberSet::new();
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        EVENT,
        EVENT_DESC,
        EventTriggerHandler {
            subscribers: subscribers.clone(),
        },
    ));
    tracing::info!(trigger_type = EVENT, "registered trigger type");
    Emitter::new(subscribers, Arc::new(IiiDeliverer::new(iii.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct RecordingDeliverer {
        seen: Mutex<Vec<(String, Value)>>,
    }

    impl EventDeliverer for RecordingDeliverer {
        fn deliver(&self, function_id: &str, payload: Value) {
            self.seen
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }

    #[test]
    fn emit_with_no_subscribers_delivers_nothing() {
        let deliverer = Arc::new(RecordingDeliverer::default());
        let emitter = Emitter::new(SubscriberSet::new(), deliverer.clone());
        emitter.emit(Event::teardown("rt-1"));
        assert!(deliverer.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn emit_fans_out_to_every_subscriber() {
        let subscribers = SubscriberSet::new();
        subscribers.insert("t1".into(), "recv::a".into());
        subscribers.insert("t2".into(), "recv::b".into());
        let deliverer = Arc::new(RecordingDeliverer::default());
        let emitter = Emitter::new(subscribers, deliverer.clone());
        emitter.emit(Event::runtime_created("rt-1", Lang::Node, "sb-1"));
        let mut seen = deliverer.seen.lock().unwrap().clone();
        seen.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, "recv::a");
        assert_eq!(seen[1].0, "recv::b");
        assert_eq!(seen[0].1["kind"], "runtime_created");
        assert_eq!(seen[0].1["runtime_id"], "rt-1");
        assert_eq!(seen[0].1["lang"], "node");
        assert_eq!(seen[0].1["sandbox_id"], "sb-1");
    }

    #[test]
    fn absent_fields_stay_off_the_wire() {
        let v = serde_json::to_value(Event::teardown("rt-1")).unwrap();
        assert_eq!(v, json!({ "kind": "teardown", "runtime_id": "rt-1" }));

        let v = serde_json::to_value(Event::run_settled(None, Lang::Python)).unwrap();
        assert_eq!(v, json!({ "kind": "run_settled", "lang": "python" }));

        let v = serde_json::to_value(Event::function_registered("app::greet", "rt-1", Lang::Node))
            .unwrap();
        assert_eq!(
            v,
            json!({
                "kind": "function_registered",
                "runtime_id": "rt-1",
                "lang": "node",
                "function_id": "app::greet",
            })
        );
    }

    #[test]
    fn unregistered_subscribers_stop_receiving() {
        let subscribers = SubscriberSet::new();
        subscribers.insert("t1".into(), "recv::a".into());
        let deliverer = Arc::new(RecordingDeliverer::default());
        let emitter = Emitter::new(subscribers.clone(), deliverer.clone());
        subscribers.remove("t1");
        emitter.emit(Event::teardown("rt-1"));
        assert!(deliverer.seen.lock().unwrap().is_empty());
    }
}
