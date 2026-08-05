//! Agent-facing ephemeral event subscriptions (harness.md § Subscriptions).
//!
//! The agent SUBSCRIBES by calling `engine::register_trigger`, which the
//! harness INTERCEPTS (see [`crate::functions::subscribe::invoke`]) to build a
//! durable binding on any external iii trigger type (`cron`, `state`, `timer`,
//! or another worker's custom type) and register ONE engine trigger pointing
//! at the delivery hop (`harness::trigger::deliver`, metadata `__binding`).
//! A fire either injects a `user`-role notification into the owning session —
//! waking (idle) or steering (running) a turn — or dispatches a plain,
//! policy-checked function call. A binding never starts an agent. Nothing
//! parks: notifications are non-blocking; only approval holds park.
//!
//! There is no harness-owned emit: the engine's trigger registry already fans
//! a fired trigger out to every bound function, so "emitting" is just whatever
//! already produces the trigger — e.g. for an ad-hoc signal, subscribe to
//! `state` on a key and have the signaller call the existing `state::set`.

pub mod fired;

/// Internal `session::deleted` cleanup handler id.
pub const ON_SESSION_DELETED_ID: &str = "harness::on-session-deleted";
pub const ON_SESSION_DELETED_DESC: &str =
    "Internal: drop a deleted session's ephemeral subscriptions. Not called directly.";

/// Trigger types the agent may NOT subscribe to, in any shape: the harness's
/// own trigger types (turn events, hook points, internal handlers). A session
/// notified of its own turn ending would wake itself forever, and child
/// outcomes belong in the medium the children write — never in a binding on
/// their turns.
pub fn is_forbidden_trigger_type(trigger_type: &str) -> bool {
    trigger_type.starts_with("harness::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbids_harness_trigger_types() {
        assert!(is_forbidden_trigger_type("harness::turn-completed"));
        assert!(is_forbidden_trigger_type("harness::notify_agent"));
        assert!(!is_forbidden_trigger_type("cron"));
        assert!(!is_forbidden_trigger_type("subscribe"));
        assert!(!is_forbidden_trigger_type("approval::pending-resolved"));
    }
}
