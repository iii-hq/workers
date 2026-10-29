//! `sandbox-code-runner::on-event`, the function that carries this worker's
//! lifecycle events, and the emitter that calls it.
//!
//! After a runtime lifecycle change — a persistent runtime comes up, a run
//! settles, a bus function is registered, a teardown lands — and on every
//! change of the daemon fleet, `RuntimeManager` and the fleet watcher call
//! `sandbox-code-runner::on-event` fire-and-forget (`CallAction::Void`) with
//! the [`Event`]. The function is private (only this worker calls it), takes
//! no args, and returns the event it is called with: iii delivers each call
//! to every observer whose filters match, so observers (the console page)
//! select kinds with a filter on `input.kind`. Emission is best-effort and
//! skipped while nothing observes the function: an absent or slow observer
//! never blocks or fails the real call.
//!
//! The function's observers also drive the fleet watcher: the first starts
//! it and the last stops it (fleet_watch.rs), so `sandbox::list` is polled
//! only while something observes.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::Engine;
use crate::runner::Lang;

/// The function the console page observes.
pub const ON_EVENT_FN: &str = "sandbox-code-runner::on-event";
pub const ON_EVENT_DESC: &str =
    "Called on runtime lifecycle changes: a persistent runtime came up \
     (runtime_created), a run finished (run_settled), a bus function was \
     registered (function_registered), a teardown landed (teardown), or \
     the daemon fleet changed (fleet — carries the full sandbox::list \
     snapshot, pushed so observers never poll). No args; filter on input.kind. \
     Event: { kind, runtime_id?, lang?, sandbox_id?, function_id?, \
     daemon_absent?, sandboxes? }.";

/// What happened.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RuntimeCreated,
    RunSettled,
    FunctionRegistered,
    Teardown,
    Fleet,
}

/// One lifecycle event. Kind-discriminated; every other field is present
/// only when that kind carries it — never code, source, or output. No
/// `Debug` impl on purpose: `runtime_id` and `sandbox_id` must never reach
/// logs (they travel only to observers, who are the operator surface).
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    kind: EventKind,
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
            kind: EventKind::RuntimeCreated,
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
            kind: EventKind::RunSettled,
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
            kind: EventKind::FunctionRegistered,
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
            kind: EventKind::Teardown,
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
    /// console tabs observe instead of each polling `sandbox::list`.
    pub fn fleet(daemon_absent: bool, sandboxes: serde_json::Value) -> Self {
        Self {
            kind: EventKind::Fleet,
            runtime_id: None,
            lang: None,
            sandbox_id: None,
            function_id: None,
            daemon_absent: Some(daemon_absent),
            sandboxes: Some(sandboxes),
        }
    }
}

/// How an event reaches `sandbox-code-runner::on-event`. Production calls it
/// over the bus; tests record.
pub trait EventDeliverer: Send + Sync {
    /// Whether anything observes `function_id`: the emitter builds no event
    /// nobody wants.
    fn is_observed(&self, function_id: &str) -> bool;

    fn deliver(&self, function_id: &str, payload: Value);
}

/// Fire-and-forget call of the event function, detached onto its own task
/// so the handler that produced the event never waits on it. Failures are
/// logged and swallowed — the real call already returned.
pub struct IiiDeliverer {
    iii: Arc<IIIClient>,
}

impl IiiDeliverer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

impl EventDeliverer for IiiDeliverer {
    fn is_observed(&self, function_id: &str) -> bool {
        self.iii.is_observed(function_id)
    }

    fn deliver(&self, function_id: &str, payload: Value) {
        let iii = self.iii.clone();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let res = iii
                .call(CallRequest {
                    function_id: function_id.clone(),
                    payload,
                    action: Some(CallAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(function_id = %function_id, error = %e, "sandbox-code-runner::on-event call failed");
            }
        });
    }
}

/// Calls `sandbox-code-runner::on-event` with each event while it is
/// observed. Nothing observing ⇒ no work at all.
#[derive(Clone)]
pub struct Emitter {
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { deliverer }
    }

    pub fn emit(&self, event: Event) {
        if !self.deliverer.is_observed(ON_EVENT_FN) {
            return;
        }
        let payload = match serde_json::to_value(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "sandbox-code-runner::on-event payload failed to serialize");
                return;
            }
        };
        self.deliverer.deliver(ON_EVENT_FN, payload);
    }
}

/// Register `sandbox-code-runner::on-event` and return the emitter that calls
/// it. The fleet watcher's observer hooks are set by id first, so they see
/// every observer. Not in `functions::STATIC_IDS` or `catalog()`: those list
/// the callable surface, and only this worker may call a private function.
pub fn register_event_function(iii: &Arc<IIIClient>, engine: Arc<dyn Engine>) -> Emitter {
    let emitter = Emitter::new(Arc::new(IiiDeliverer::new(iii.clone())));
    crate::fleet_watch::run_while_observed(iii, engine, emitter.clone());
    iii.register_function(
        ON_EVENT_FN,
        RegisterFunction::new(|event: Event| Ok::<_, Error>(event))
            .description(ON_EVENT_DESC)
            .private(),
    );
    tracing::info!(function_id = ON_EVENT_FN, "registered event function");
    emitter
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    struct RecordingDeliverer {
        observed: bool,
        seen: Mutex<Vec<(String, Value)>>,
    }

    impl RecordingDeliverer {
        fn new(observed: bool) -> Arc<Self> {
            Arc::new(Self {
                observed,
                seen: Mutex::new(Vec::new()),
            })
        }
    }

    impl EventDeliverer for RecordingDeliverer {
        fn is_observed(&self, _function_id: &str) -> bool {
            self.observed
        }

        fn deliver(&self, function_id: &str, payload: Value) {
            self.seen
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }

    #[test]
    fn emit_while_unobserved_delivers_nothing() {
        let deliverer = RecordingDeliverer::new(false);
        let emitter = Emitter::new(deliverer.clone());
        emitter.emit(Event::teardown("rt-1"));
        assert!(deliverer.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn emit_calls_the_event_function_once() {
        let deliverer = RecordingDeliverer::new(true);
        let emitter = Emitter::new(deliverer.clone());
        emitter.emit(Event::runtime_created("rt-1", Lang::Node, "sb-1"));
        let seen = deliverer.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "sandbox-code-runner::on-event");
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

    /// The handler returns the event it is called with, so an event read
    /// back from the wire serializes unchanged.
    #[test]
    fn an_event_round_trips_through_the_identity_handler() {
        let wire =
            serde_json::to_value(Event::fleet(false, json!([{ "sandbox_id": "a" }]))).unwrap();
        let back: Event = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(back).unwrap(), wire);
    }
}
