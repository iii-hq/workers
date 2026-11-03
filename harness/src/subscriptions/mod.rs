//! Agent-facing observers (harness.md § Subscriptions).
//!
//! The agent OBSERVES by calling `harness::observe`, which the harness
//! INTERCEPTS (see [`crate::functions::observe::invoke`]) to build a durable
//! binding on any observable function (`state::on-changed`, `cron::on-tick`,
//! `harness::on-timer`, or any other worker's function) and create ONE
//! observer aimed at the delivery hop (`harness::observer::deliver`, metadata
//! `__binding`). A delivery either injects a `user`-role notification into
//! the owning session — waking (idle) or steering (running) a turn — or
//! dispatches a plain, policy-checked function call. A binding never starts
//! an agent. Nothing parks: notifications are non-blocking; only approval
//! holds park.
//!
//! There is no harness-owned emit: iii delivers every call of a function to
//! its observers, so "emitting" is just calling whatever already produces the
//! event — e.g. for an ad-hoc signal, observe `state::on-changed` filtered on
//! a key and have the signaller call the existing `state::set`.

pub mod fired;

/// Internal `session::on-deleted` observer id.
pub const ON_SESSION_DELETED_ID: &str = "harness::on-session-deleted";
pub const ON_SESSION_DELETED_DESC: &str =
    "Internal: drop a deleted session's observers. Not called directly.";

/// Functions the agent may NOT observe, in any shape: the harness's own
/// functions (turn events, hook points, internal handlers) — all but
/// `harness::on-timer`, the deadline agents arm for themselves. A session
/// notified of its own turn ending would wake itself forever, and child
/// outcomes belong in the medium the children write — never in an observer
/// on their turns. The agent's post-turn validator is observed through its
/// own path before this rule applies.
pub fn is_forbidden_source(function_id: &str) -> bool {
    function_id.starts_with("harness::") && function_id != crate::timer::ON_TIMER_FN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbids_harness_functions_but_the_timer() {
        assert!(is_forbidden_source("harness::on-turn-completed"));
        assert!(is_forbidden_source("harness::hook::on-pre-generate"));
        assert!(is_forbidden_source("harness::notify_agent"));
        assert!(!is_forbidden_source("harness::on-timer"));
        assert!(!is_forbidden_source("cron::on-tick"));
        assert!(!is_forbidden_source("publish"));
        assert!(!is_forbidden_source("approval::on-pending-resolved"));
    }
}
