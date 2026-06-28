//! Agent-facing ephemeral event subscriptions (harness.md § Subscriptions).
//!
//! The agent SUBSCRIBES by calling `engine::register_trigger`, which the harness
//! INTERCEPTS (see [`crate::functions::subscribe::invoke`]) to
//! register an EPHEMERAL listener on any iii trigger type (`cron`, `state`,
//! `stream`, or another worker's custom trigger type) and be NOTIFIED when it
//! fires — instead of polling. When it fires, the shared `harness::notify_agent`
//! handler injects a `user`-role notification into the owning session, which
//! wakes (idle) or steers (running) a turn. This is the opposite of
//! `harness::spawn`: the turn is never parked.
//!
//! There is no harness-owned emit: the engine's trigger registry already fans a
//! fired trigger out to every bound function, so "emitting" is just whatever
//! already produces the trigger — e.g. for an ad-hoc signal, subscribe to
//! `state` on a key and have the signaller call the existing `state::set`.
//!
//! Routing: the interceptor registers a trigger (via `engine::register_trigger`)
//! bound to the ONE shared `harness::notify_agent` function. The engine's
//! per-subscription proxy round-trips the registration metadata into the fired
//! payload under [`TRIGGER_META_KEY`] (`__metadata`). That metadata carries only
//! the subscription id; the local registry is the authority for owner, label,
//! and `once`. The owning session is injected by the harness when it intercepts
//! the agent's `engine::register_trigger` call, never trusted from model
//! arguments or fired payloads.

pub mod notify_agent;
pub mod registry;

pub use registry::{CapExceeded, SubscriptionRegistry};

/// Hard cap on active subscriptions per session — a cheap DoS floor. Lifecycle
/// otherwise leans on unsubscribe / `once` / `session::deleted` / process exit.
pub const MAX_SUBSCRIPTIONS_PER_SESSION: usize = 64;

/// The single shared subscription fire handler id. Every subscription's trigger
/// binds to this (via `engine::register_trigger`); kept OFF the agent-facing catalog.
pub const NOTIFY_AGENT_ID: &str = "harness::notify_agent";
pub const NOTIFY_AGENT_DESC: &str =
    "Internal: the shared subscription fire handler — injects a notification into the owning \
     session (resolved from the local subscription registry). Not called directly.";

/// Reserved payload key under which the engine's subscription proxy round-trips a
/// trigger's registration metadata into the fired payload. MUST match the engine
/// constant `iii::trigger::TRIGGER_META_KEY`.
pub const TRIGGER_META_KEY: &str = "__metadata";

/// Internal `session::deleted` cleanup handler id.
pub const ON_SESSION_DELETED_ID: &str = "harness::on-session-deleted";
pub const ON_SESSION_DELETED_DESC: &str =
    "Internal: drop a deleted session's ephemeral subscriptions. Not called directly.";

/// Trigger types the agent may NOT subscribe to: the harness's own trigger
/// types (turn events, hook points, internal sub handlers). Subscribing to
/// these risks a self-notification loop — a notify wakes a turn whose
/// completion re-fires the trigger — and would expose harness internals.
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
