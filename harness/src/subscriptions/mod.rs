//! Agent-facing ephemeral event subscriptions (harness.md § Subscriptions).
//!
//! The agent calls `harness::subscribe` to register an EPHEMERAL listener on any
//! existing iii trigger type (`cron`, `state`, `stream`, or another worker's
//! custom trigger type) and be NOTIFIED when it fires — instead of polling. When
//! it fires, a per-subscription internal handler injects a `user`-role
//! notification into the owning session, which wakes (idle) or steers (running) a
//! turn. This is the opposite of `harness::spawn`: the turn is never parked.
//!
//! There is no harness-owned emit: the engine's trigger registry already fans a
//! fired trigger out to every bound function, so "emitting" is just whatever
//! already produces the trigger — e.g. for an ad-hoc signal, subscribe to
//! `state` on a key and have the signaller call the existing `state::set`.
//!
//! Routing: built-in triggers fire the bound function with only the engine
//! event payload (no metadata redelivery), so each subscription registers its
//! OWN unique `harness::sub::<id>` function whose closure captures the owning
//! session — the engine routing IS the addressing. The owning session is
//! injected by the trusted dispatch layer (turn loop / `harness::function::trigger`),
//! never trusted from model arguments.

pub mod handler;
pub mod registry;

use serde_json::Value;

pub use registry::{SubEntry, SubscriptionRegistry};

/// Hard cap on active subscriptions per session — a cheap DoS floor. Lifecycle
/// otherwise leans on unsubscribe / `once` / `session::deleted` / process exit.
pub const MAX_SUBSCRIPTIONS_PER_SESSION: usize = 64;

pub const SUBSCRIBE_ID: &str = "harness::subscribe";
pub const SUBSCRIBE_DESC: &str =
    "Be notified when a trigger fires instead of polling. Pass { trigger_type, config } for any \
     iii trigger type — cron | state | stream | another worker's custom trigger type (e.g. \
     approval::pending-resolved). For an ad-hoc signal, subscribe to `state` on a key and have \
     the signaller call `state::set` on it. Non-blocking: you keep working, and when it fires a \
     notification message arrives in this session. Returns a subscription_id.";

pub const UNSUBSCRIBE_ID: &str = "harness::unsubscribe";
pub const UNSUBSCRIBE_DESC: &str =
    "Tear down a subscription created by harness::subscribe (by subscription_id). Idempotent.";

/// Internal per-subscription handler id namespace (`harness::sub::<sub_id>`).
/// These are dynamically registered and kept OFF the agent-facing catalog.
pub const INTERNAL_FN_PREFIX: &str = "harness::sub::";

/// Internal `session::deleted` cleanup handler id.
pub const ON_SESSION_DELETED_ID: &str = "harness::on-session-deleted";
pub const ON_SESSION_DELETED_DESC: &str =
    "Internal: drop a deleted session's ephemeral subscriptions. Not called directly.";

/// The per-subscription internal function id for `sub_id`.
pub fn internal_fn_id(sub_id: &str) -> String {
    format!("{INTERNAL_FN_PREFIX}{sub_id}")
}

/// Trigger types the agent may NOT subscribe to: the harness's own trigger
/// types (turn events, hook points, internal sub handlers). Subscribing to
/// these risks a self-notification loop — a notify wakes a turn whose
/// completion re-fires the trigger — and would expose harness internals.
pub fn is_forbidden_trigger_type(trigger_type: &str) -> bool {
    trigger_type.starts_with("harness::")
}

/// Stamp the owning `session_id` onto a subscription control call's arguments,
/// overriding anything the model supplied (the model can never widen the
/// target). Applied by the dispatch layer where the calling session is known —
/// mirrors how `harness::spawn` receives its parent from the loop, not args.
/// A no-op for any other function id.
pub fn inject_owner_session(function_id: &str, arguments: &mut Value, session_id: &str) {
    if function_id != SUBSCRIBE_ID && function_id != UNSUBSCRIBE_ID {
        return;
    }
    if !arguments.is_object() {
        *arguments = Value::Object(serde_json::Map::new());
    }
    if let Some(obj) = arguments.as_object_mut() {
        obj.insert(
            "session_id".to_string(),
            Value::String(session_id.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn forbids_harness_trigger_types() {
        assert!(is_forbidden_trigger_type("harness::turn-completed"));
        assert!(is_forbidden_trigger_type("harness::sub::abc"));
        assert!(!is_forbidden_trigger_type("cron"));
        assert!(!is_forbidden_trigger_type("subscribe"));
        assert!(!is_forbidden_trigger_type("approval::pending-resolved"));
    }

    #[test]
    fn injects_owner_over_model_supplied_session() {
        let mut args = json!({ "trigger_type": "cron", "session_id": "s_attacker" });
        inject_owner_session(SUBSCRIBE_ID, &mut args, "s_owner");
        assert_eq!(args["session_id"], json!("s_owner"));
    }

    #[test]
    fn injects_into_non_object_arguments() {
        let mut args = json!(null);
        inject_owner_session(UNSUBSCRIBE_ID, &mut args, "s_owner");
        assert_eq!(args["session_id"], json!("s_owner"));
    }

    #[test]
    fn ignores_unrelated_functions() {
        let mut args = json!({ "x": 1 });
        inject_owner_session("shell::run", &mut args, "s_owner");
        assert!(args.get("session_id").is_none());
    }

    #[test]
    fn internal_fn_id_uses_prefix() {
        assert_eq!(internal_fn_id("sub_x"), "harness::sub::sub_x");
    }
}
