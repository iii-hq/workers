//! Observer bindings — the durable record behind every agent-created
//! observer (architecture/observer-bindings.md).
//!
//! The engine's observer holds what is observed, the target, and a metadata
//! sidecar; nothing about an owner, a lifecycle the harness counts, or a
//! frozen capability. Everything the harness needs at fire time therefore
//! lives HERE, keyed by the binding id, which is also the observer's id. The
//! observer's metadata carries only `{"__binding": "<id>"}` — a pointer, not
//! a payload. Nothing an agent can smuggle into metadata is read back.
//!
//! The record is durable (state worker, `harness_binding` scope) because the
//! engine's observers are not: they belong to the harness connection, so a
//! harness or engine restart loses them, and the boot pass re-creates every
//! surviving binding's observer from this store.

pub mod compose;
pub mod expiry;
pub mod gc;
pub mod migrate;
mod store;

pub use store::{
    AttachOutcome, BindingStore, ClaimOutcome, ReserveOutcome, MAX_BINDINGS_PER_SESSION,
};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::protocol::CallRequest;
use iii_sdk::{ObserverHandle, ObserverOptions, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::types::turn::FunctionPolicy;

/// Added to a binding's remaining lifetime when it becomes the observer's
/// `expires_in` backstop: the harness's own deadline timer retires the
/// binding first and tells the owner; the engine's expiry only catches an
/// observer the harness lost track of.
const EXPIRY_BACKSTOP_GRACE: Duration = Duration::from_secs(60);

/// Live observer handles of this process's bindings, keyed by binding id
/// (which is also the observer id). Disposing goes THROUGH the handle so the
/// SDK's reconnect replay forgets the observer too.
#[derive(Clone, Default)]
pub struct ObserverHandles(Arc<Mutex<HashMap<String, ObserverHandle>>>);

impl ObserverHandles {
    pub fn insert(&self, binding_id: String, handle: ObserverHandle) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(binding_id, handle);
    }

    pub fn remove(&self, binding_id: &str) -> Option<ObserverHandle> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(binding_id)
    }

    pub fn contains(&self, binding_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(binding_id)
    }
}

/// Create the binding's observer: its source, the delivery hop as target, the
/// binding id as observer id and `__binding` sidecar, and the lifecycle
/// deadline as a backstop. The handle is kept in [`Deps::observer_handles`].
/// `None` for a binding without a source.
pub fn observe_binding(deps: &Deps, binding: &Binding) -> Option<ObserverHandle> {
    let source = binding.source.as_ref()?;
    let mut options = ObserverOptions::default()
        .id(binding.id.clone())
        .metadata(json!({ "__binding": binding.id }));
    if let Some(at) = binding.lifecycle.expires_at {
        let remaining = at - crate::types::message::AgentMessage::now_ms();
        if remaining > 0 {
            options =
                options.expires_in(Duration::from_millis(remaining as u64) + EXPIRY_BACKSTOP_GRACE);
        }
    }
    let handle = deps.iii.observe_with(
        source.to_source(),
        crate::functions::observer_deliver::DELIVER_ID,
        options,
    );
    deps.observer_handles
        .insert(binding.id.clone(), handle.clone());
    Some(handle)
}

/// Dispose a binding's observer; `true` when it is gone. Through this
/// process's handle when it holds one, else through
/// `engine::observers::dispose` (an observer created before the last restart
/// that the boot pass has not re-created yet).
pub async fn dispose_observer(deps: &Deps, binding_id: &str) -> bool {
    if let Some(handle) = deps.observer_handles.remove(binding_id) {
        return match handle.dispose().await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(binding = %binding_id, %error, "observer dispose failed");
                false
            }
        };
    }
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    match deps
        .iii
        .call(CallRequest {
            function_id: OBSERVERS_DISPOSE_ID.to_string(),
            payload: json!({ "id": binding_id }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(binding = %binding_id, %error, "engine observer dispose failed");
            false
        }
    }
}

/// The engine builtin that disposes an observer by id.
pub const OBSERVERS_DISPOSE_ID: &str = "engine::observers::dispose";

/// What a binding observes: the source half of its observer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceSpec {
    /// The observed function.
    pub function_id: String,
    /// Where the observed function is hosted; unset resolves like a call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Args for the worker that hosts the function.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Which calls reach the binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Value>,
}

impl SourceSpec {
    pub fn to_source(&self) -> Source {
        let mut source = Source::function(self.function_id.clone());
        if let Some(namespace) = &self.namespace {
            source = source.namespace(namespace.clone());
        }
        if let Some(args) = &self.args {
            source = source.args(args.clone());
        }
        if let Some(filters) = &self.filters {
            source = source.filters(filters.clone());
        }
        source
    }

    /// The `$eq` value of a top-level filter on `path` (`input.scope`).
    pub fn filter_eq(&self, path: &str) -> Option<&Value> {
        self.filters.as_ref()?.get(path)?.get("$eq")
    }
}

/// An armed wake: a one-shot self-delivery binding that has not fired and is
/// not past its lifecycle. An EXPIRED one does not count — its deadline
/// timer retires it and tells the owner, and a session must not stay
/// non-terminal waiting on a wake that can never fire.
pub fn is_armed_wake(binding: &Binding, _now_ms: i64) -> bool {
    // Deliberately NOT gated on `is_exhausted`: a past-deadline unfired wake
    // still OWES its owner the expiry notice (bindings::expiry). Retirement
    // deletes the record before notifying, so the flag flips exactly when
    // the notice is on its way — checking the clock here instead opened a
    // window (then a sweep interval) where the session looked quietly
    // terminal while the notice was still coming (live-caught by the
    // `subagent_validation_failure` e2e: the suite finalized in that window
    // and the notice was lost with the teardown).
    binding.lifecycle.once
        && binding.target.function_id == crate::functions::SEND_ID
        && binding.fires == 0
}

/// Whether a session is parked on a wake it has not consumed — a one-shot
/// binding that delivers back into the session itself. A turn completing with
/// one armed is NOT terminal: the run continues when the wake fires.
///
/// Reads the durable store, not an in-memory flag: the flag did not survive a
/// harness restart, so every parked session came back looking terminal.
pub async fn session_expects_wake(deps: &Deps, session_id: &str) -> bool {
    let now = crate::types::message::AgentMessage::now_ms();
    match deps.bindings().await.list_for_owner(session_id).await {
        Ok(bindings) => bindings.iter().any(|b| is_armed_wake(b, now)),
        // Fail CLOSED on an unreadable store: reporting "no wake" would mark a
        // parked run terminal and finalize an exchange that is still going.
        Err(_) => true,
    }
}

/// One armed wake as `harness::status` reports it — enough for a console or a
/// poller to say "parked on state operation_meta/status since T, deadline T2"
/// instead of showing a session that just looks quietly done.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArmedWake {
    pub observer_id: String,
    /// The observed function and the observer's args and filters. Absent on
    /// records without a source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<Value>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

/// The session's armed wakes, in detail. `None` when the store is unreadable —
/// callers deciding terminality must fail closed on it, exactly like
/// [`session_expects_wake`] does.
pub async fn armed_wakes(deps: &Deps, session_id: &str) -> Option<Vec<ArmedWake>> {
    let now = crate::types::message::AgentMessage::now_ms();
    let bindings = deps
        .bindings()
        .await
        .list_for_owner(session_id)
        .await
        .ok()?;
    Some(
        bindings
            .iter()
            .filter(|b| is_armed_wake(b, now))
            .map(|b| ArmedWake {
                observer_id: b.id.clone(),
                function_id: b.source.as_ref().map(|s| s.function_id.clone()),
                args: b.source.as_ref().and_then(|s| s.args.clone()),
                filters: b.source.as_ref().and_then(|s| s.filters.clone()),
                created_at: b.created_at,
                expires_at: b.lifecycle.expires_at,
            })
            .collect(),
    )
}

/// Who registered a binding. The session is the unregister-ownership check and
/// the delivery destination for a wake; the root is console nesting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnerScope {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_session_id: Option<String>,
}

/// What a delivery dispatches. `function_id` is any iii function —
/// `harness::send` for a wake, anything else for a mechanical reaction.
/// Strict: an unknown key (sub-agent fields such as `task`) is refused when
/// the agent observes, never silently dropped.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingTarget {
    pub function_id: String,
    /// Payload template; the delivered event is injected into it at
    /// `event_into`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// JSON pointer where the event lands inside the payload (default
    /// `/event`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_into: Option<String>,
}

impl BindingTarget {
    pub fn new(function_id: impl Into<String>) -> Self {
        Self {
            function_id: function_id.into(),
            payload: None,
            event_into: None,
        }
    }

    pub const DEFAULT_EVENT_INTO: &'static str = "/event";

    pub fn event_pointer(&self) -> &str {
        self.event_into
            .as_deref()
            .unwrap_or(Self::DEFAULT_EVENT_INTO)
    }
}

/// One declared condition. Evaluated at fire time with the typed decision
/// contract; ordinary iii functions, so a barrier or a claim is just a
/// function someone wrote.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ConditionSpec {
    pub function_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

/// When a binding stops. `once` retires it after the first DELIVERED fire (a
/// skipped one does not consume it); `max_fires` is a lifetime budget;
/// `expires_at` a wall-clock deadline.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lifecycle {
    #[serde(default)]
    pub once: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fires: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// The binding id, which is also its observer's id.
    pub id: String,
    /// The observer's id once this process created it. `None` for the window
    /// between the local insert and the observe call. Records written before
    /// 0.26 name it `trigger_id`; the boot pass rewrites them.
    #[serde(default, alias = "trigger_id", skip_serializing_if = "Option::is_none")]
    pub observer_id: Option<String>,
    /// What the binding observes. Absent only on records the boot pass has
    /// not translated yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceSpec>,
    pub owner: OwnerScope,
    pub target: BindingTarget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ConditionSpec>,
    #[serde(default)]
    pub lifecycle: Lifecycle,
    /// The registrant's dispatch policy, FROZEN at registration. A fired call
    /// is checked against this, not against the owner's current policy — which
    /// may have widened since, and a binding must not gain reach because its
    /// owner did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<FunctionPolicy>,
    /// Canonicalised registration request; a same-session re-registration with
    /// an equal key returns the standing binding instead of wiring a twin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<Value>,
    #[serde(default)]
    pub fires: u64,
    pub created_at: i64,
}

impl Binding {
    /// User-facing event text declared at registration time. It lives in the
    /// canonical request so active rows, retirement records, and fires all
    /// describe the same event without interpreting source-specific args.
    pub fn event_action(&self) -> Option<&str> {
        self.dedup_key
            .as_ref()
            .and_then(|key| key.get("action"))
            .and_then(Value::as_str)
            .filter(|action| !action.trim().is_empty())
    }

    /// Whether the lifecycle is spent after `fires` deliveries.
    pub fn is_exhausted(&self, now_ms: i64) -> bool {
        self.is_spent() || matches!(self.lifecycle.expires_at, Some(at) if now_ms >= at)
    }

    /// Whether DELIVERED fires used up the fire budget (`once` consumed, or
    /// `max_fires` reached). The delivery that claimed the last slot owns
    /// retiring the binding and its one `observer_fired` record; any other
    /// attempt that finds it spent is a duplicate (the same event arriving
    /// by a second path), never a user-visible outcome of its own.
    pub fn is_spent(&self) -> bool {
        (self.lifecycle.once && self.fires >= 1)
            || self
                .lifecycle
                .max_fires
                .is_some_and(|max| self.fires >= max)
    }

    /// Whether THIS delivery is the last one the lifecycle allows, so the
    /// caller retires the binding in the same pass that dispatches it.
    pub fn retires_after_fire(&self, fires_after: u64) -> bool {
        if self.lifecycle.once {
            return fires_after >= 1;
        }
        matches!(self.lifecycle.max_fires, Some(max) if fires_after >= max)
    }

    /// The `(scope, key)` a `state::on-changed` binding watches, from its
    /// `$eq` filters on `input.scope` / `input.key`.
    pub fn state_watch(&self) -> (Option<&str>, Option<&str>) {
        match self.source.as_ref() {
            Some(source) if source.function_id == STATE_ON_CHANGED_FN => (
                source.filter_eq("input.scope").and_then(Value::as_str),
                source.filter_eq("input.key").and_then(Value::as_str),
            ),
            _ => (None, None),
        }
    }

    /// `function_id` with its args and filters, for owner-facing text.
    pub fn source_text(&self) -> Option<String> {
        let source = self.source.as_ref()?;
        let mut text = format!("`{}`", source.function_id);
        if let Some(args) = &source.args {
            text.push_str(&format!(" args {args}"));
        }
        if let Some(filters) = &source.filters {
            text.push_str(&format!(" filters {filters}"));
        }
        Some(text)
    }
}

/// The state worker's change event.
pub const STATE_ON_CHANGED_FN: &str = "state::on-changed";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding() -> Binding {
        Binding {
            id: "sub_1".into(),
            observer_id: None,
            source: None,
            owner: OwnerScope {
                session_id: "s1".into(),
                root_session_id: None,
            },
            target: BindingTarget::new("harness::send"),
            conditions: vec![],
            lifecycle: Lifecycle::default(),
            capability: None,
            dedup_key: None,
            fires: 0,
            created_at: 0,
        }
    }

    #[test]
    fn event_pointer_defaults_to_slash_event() {
        let mut t = BindingTarget::new("state::set");
        assert_eq!(t.event_pointer(), "/event");
        t.event_into = Some("/value".into());
        assert_eq!(t.event_pointer(), "/value");
    }

    #[test]
    fn event_action_reads_the_registration_request() {
        let mut b = binding();
        b.dedup_key = Some(json!({
            "function_id": "explorer::on-message",
            "action": "new Explorer message received"
        }));
        assert_eq!(b.event_action(), Some("new Explorer message received"));
        b.dedup_key.as_mut().unwrap()["action"] = json!(7);
        assert_eq!(b.event_action(), None);
    }

    #[test]
    fn once_is_exhausted_by_its_first_delivered_fire() {
        let mut b = binding();
        b.lifecycle.once = true;
        assert!(!b.is_exhausted(0));
        assert!(b.retires_after_fire(1));
        b.fires = 1;
        assert!(b.is_exhausted(0));
    }

    #[test]
    fn max_fires_is_a_lifetime_budget() {
        let mut b = binding();
        b.lifecycle.max_fires = Some(3);
        b.fires = 2;
        assert!(!b.is_exhausted(0));
        assert!(!b.retires_after_fire(2));
        assert!(b.retires_after_fire(3));
        b.fires = 3;
        assert!(b.is_exhausted(0));
    }

    #[test]
    fn a_deadline_exhausts_a_standing_binding() {
        let mut b = binding();
        b.lifecycle.expires_at = Some(100);
        assert!(!b.is_exhausted(99));
        assert!(b.is_exhausted(100));
        // A standing binding with no deadline never exhausts on time.
        let plain = binding();
        assert!(!plain.is_exhausted(i64::MAX));
    }

    /// The parked-forever hole: before wake expiry, an expired never-fired
    /// wake still counted as "expects wake", so lifecycle on a wake protected
    /// nothing — the session stayed non-terminal next to a deadline that had
    /// already passed.
    #[test]
    fn an_expired_wake_stays_armed_until_its_retirement_deletes_it() {
        let mut b = binding();
        b.lifecycle.once = true;
        assert!(is_armed_wake(&b, 0), "a live once-wake parks its session");
        b.lifecycle.expires_at = Some(100);
        assert!(is_armed_wake(&b, 99));
        // Past the deadline the wake cannot FIRE — but the owner is still
        // owed the expiry notice, so the session must keep reporting an
        // armed wake until retirement deletes the record (and delivers the
        // notice). Flipping on the clock opened a window where a parked
        // session looked quietly terminal.
        assert!(
            is_armed_wake(&b, 100),
            "the expiry notice is still owed; the session is still parked"
        );
        // A consumed wake and a non-wake target never arm.
        let mut fired = binding();
        fired.lifecycle.once = true;
        fired.fires = 1;
        assert!(!is_armed_wake(&fired, 0));
        let mut call = binding();
        call.lifecycle.once = true;
        call.target = BindingTarget::new("state::set");
        assert!(!is_armed_wake(&call, 0));
        // A standing notify (once: false) is not a park either.
        assert!(!is_armed_wake(&binding(), 0));
    }

    #[test]
    fn state_watch_reads_the_eq_filters_of_a_state_source() {
        let mut b = binding();
        assert_eq!(b.state_watch(), (None, None), "no source, no watch");
        b.source = Some(SourceSpec {
            function_id: STATE_ON_CHANGED_FN.into(),
            namespace: None,
            args: None,
            filters: Some(json!({
                "input.scope": { "$eq": "run" },
                "input.key": { "$eq": "go" },
            })),
        });
        assert_eq!(b.state_watch(), (Some("run"), Some("go")));
        b.source.as_mut().unwrap().function_id = "database::on-row-changed".into();
        assert_eq!(
            b.state_watch(),
            (None, None),
            "only a state source watches a key"
        );
    }

    #[test]
    fn records_written_before_observers_still_read() {
        let stored = json!({
            "id": "sub_1",
            "trigger_id": "sdk:sub_1",
            "owner": { "session_id": "s1" },
            "target": { "function_id": "harness::send" },
            "causation": { "depth": 0 },
            "dedup_key": { "trigger_type": "state", "config": { "scope": "run" } },
            "created_at": 0
        });
        let b: Binding = serde_json::from_value(stored).unwrap();
        assert_eq!(b.observer_id.as_deref(), Some("sdk:sub_1"));
        assert!(b.source.is_none(), "the boot pass translates the source");
    }

    #[test]
    fn record_round_trips_through_json() {
        let mut b = binding();
        b.conditions = vec![ConditionSpec {
            function_id: "state::barrier".into(),
            config: Some(json!({ "expect": 3 })),
        }];
        b.target.payload = Some(json!({ "message": "go" }));
        let v = serde_json::to_value(&b).unwrap();
        let back: Binding = serde_json::from_value(v).unwrap();
        assert_eq!(back.conditions, b.conditions);
        assert_eq!(back.target, b.target);
    }
}
