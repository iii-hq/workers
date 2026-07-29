//! Trigger bindings — the durable record behind every agent-registered
//! subscription (architecture/trigger-bindings.md).
//!
//! The engine's `Trigger` holds `{trigger_type, function_id, config,
//! worker_id, metadata}` and nothing else: no owner, no lifecycle, no
//! capability. Everything the harness needs at fire time therefore lives
//! HERE, keyed by a binding id, and the engine-side metadata carries only
//! `{"__binding": "<id>"}` — a pointer, not a payload. Nothing an agent can
//! smuggle into metadata is read back.
//!
//! The record is durable (state worker, `harness_binding` scope) because the
//! registry it replaces was an in-memory map wiped on every restart: spawn
//! bindings kept firing with no owner bookkeeping, notify bindings stopped
//! delivering entirely, and a startup reconciler existed only to GC the
//! wreckage.

pub mod expiry;
pub mod gc;
mod store;

pub use store::{
    AttachOutcome, BindingStore, ClaimOutcome, ReserveOutcome, MAX_BINDINGS_PER_SESSION,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::types::turn::FunctionPolicy;

/// An armed wake: a one-shot self-delivery binding that has not fired and is
/// not past its lifecycle. An EXPIRED one does not count — the expiry sweep
/// retires it and tells the owner, and a session must not stay non-terminal
/// waiting on a wake that can never fire.
pub fn is_armed_wake(binding: &Binding, now_ms: i64) -> bool {
    binding.lifecycle.once
        && binding.target.function_id == crate::functions::SEND_ID
        && binding.fires == 0
        && !binding.is_exhausted(now_ms)
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
    pub subscription_id: String,
    /// The registered trigger's type/config, read from the canonicalised
    /// registration request. Absent on records that predate it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
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
            .map(|b| {
                let watch = b.trigger_watch();
                ArmedWake {
                    subscription_id: b.id.clone(),
                    trigger_type: watch.map(|(t, _)| t.to_string()),
                    config: watch.map(|(_, c)| c.clone()),
                    created_at: b.created_at,
                    expires_at: b.lifecycle.expires_at,
                }
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

/// What a fire dispatches. `function_id` is any iii function — `harness::send`
/// for a wake or a sub-agent, anything else for a mechanical reaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BindingTarget {
    pub function_id: String,
    /// Payload template; the fired event is injected into it at `event_into`.
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

/// Loop-safety inputs, stamped at registration and never caller-supplied.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Causation {
    /// The reactive depth of the turn that registered this binding. A fire
    /// starts its reaction one level deeper.
    #[serde(default)]
    pub depth: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_by_turn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    /// The engine's own binding id, once registration returns it. `None` for
    /// the window between the local insert and the engine's answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<String>,
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
    #[serde(default)]
    pub causation: Causation,
    /// Canonicalised registration request; a same-session re-registration with
    /// an equal key returns the standing binding instead of wiring a twin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<Value>,
    #[serde(default)]
    pub fires: u64,
    pub created_at: i64,
}

impl Binding {
    /// Whether the lifecycle is spent after `fires` deliveries.
    pub fn is_exhausted(&self, now_ms: i64) -> bool {
        if self.lifecycle.once && self.fires >= 1 {
            return true;
        }
        if let Some(max) = self.lifecycle.max_fires {
            if self.fires >= max {
                return true;
            }
        }
        matches!(self.lifecycle.expires_at, Some(at) if now_ms >= at)
    }

    /// Whether THIS delivery is the last one the lifecycle allows, so the
    /// caller retires the binding in the same pass that dispatches it.
    pub fn retires_after_fire(&self, fires_after: u64) -> bool {
        if self.lifecycle.once {
            return fires_after >= 1;
        }
        matches!(self.lifecycle.max_fires, Some(max) if fires_after >= max)
    }

    /// The registered trigger's `(type, config)`, read from the canonicalised
    /// registration request stored as the dedup key. `None` for records that
    /// predate the key.
    pub fn trigger_watch(&self) -> Option<(&str, &Value)> {
        let key = self.dedup_key.as_ref()?;
        let trigger_type = key.get("trigger_type")?.as_str()?;
        Some((trigger_type, key.get("config").unwrap_or(&Value::Null)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding() -> Binding {
        Binding {
            id: "sub_1".into(),
            trigger_id: None,
            owner: OwnerScope {
                session_id: "s1".into(),
                root_session_id: None,
            },
            target: BindingTarget::new("harness::send"),
            conditions: vec![],
            lifecycle: Lifecycle::default(),
            capability: None,
            causation: Causation::default(),
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

    /// The parked-forever hole: before the expiry sweep, an expired never-fired
    /// wake still counted as "expects wake", so lifecycle on a wake protected
    /// nothing — the session stayed non-terminal next to a deadline that had
    /// already passed.
    #[test]
    fn an_expired_wake_is_no_longer_armed() {
        let mut b = binding();
        b.lifecycle.once = true;
        assert!(is_armed_wake(&b, 0), "a live once-wake parks its session");
        b.lifecycle.expires_at = Some(100);
        assert!(is_armed_wake(&b, 99));
        assert!(
            !is_armed_wake(&b, 100),
            "past the deadline nothing can fire this wake; it must not park the session"
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
    fn trigger_watch_reads_the_dedup_key() {
        let mut b = binding();
        assert!(b.trigger_watch().is_none(), "no key, no watch");
        b.dedup_key = Some(json!({
            "trigger_type": "state",
            "config": { "scope": "run", "key": "go" },
        }));
        let (ttype, config) = b.trigger_watch().unwrap();
        assert_eq!(ttype, "state");
        assert_eq!(config["scope"], json!("run"));
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
