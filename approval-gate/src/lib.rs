//! Approval gate. Subscribes to `agent::before_function_call` and decides
//! every call via the layered rules engine (`rules::evaluate`). Allow →
//! `{block:false}`. Deny → `{block:true, denial:Policy{rule_permission,
//! rule_pattern}}`. Ask → write a Pending record and wait for
//! `approval::resolve`.

pub mod config;
pub mod delivery;
pub mod intercept;
pub mod lifecycle;  // transitional compat shim — deleted in T11 after T5/T6/T8 migrate callsites
pub mod manifest;
pub mod record;
pub mod register;
pub mod resolve;
pub mod rules;
pub mod state;
pub mod sweeper;
pub mod wire;

pub use config::{InterceptorRule, WorkerConfig};
pub use delivery::{
    handle_ack_delivered, handle_consume_undelivered, handle_flush_delivered, handle_list_pending,
    handle_list_undelivered, handle_sweep_session, LIST_UNDELIVERED_DEFAULT_LIMIT,
};
pub use intercept::handle_intercept;
pub use record::{Outcome, Record, Status};
pub use register::{
    register, Refs, FN_ACK_DELIVERED, FN_CONSUME_UNDELIVERED, FN_FLUSH_DELIVERED, FN_LIST_PENDING,
    FN_LIST_UNDELIVERED, FN_LOOKUP_RECORD, FN_RESOLVE, FN_SWEEP_SESSION, STATE_SCOPE,
};
pub use resolve::{handle_lookup_record, handle_resolve};
pub use state::{
    unverified_marker_targets, FunctionExecutor, IiiFunctionExecutor, IiiStateBus, StateBus,
};
pub use wire::{
    block_reply_for, extract_call, pending_key, Decision, Denial, IncomingCall, WireDecision,
};

/// Subscriber's terminal verdict for an incoming call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Allow,
    Deny(Denial),
    Ask,
}

/// Apply the layered rules to an incoming call. Last-matching rule wins;
/// no match defaults to Ask (operator-safe default — paired with the
/// curated default ruleset shipped in `iii.worker.yaml`).
pub(crate) fn verdict_for(
    function_id: &str,
    args: &serde_json::Value,
    rules: &rules::Ruleset,
) -> Verdict {
    let pattern = rules::pattern_for(function_id, args);
    match rules::evaluate(function_id, &pattern, rules) {
        Some(r) => match r.action {
            rules::Action::Allow => Verdict::Allow,
            rules::Action::Deny => Verdict::Deny(Denial::Policy {
                rule_permission: r.permission.clone(),
                rule_pattern: r.pattern.clone(),
            }),
            rules::Action::Ask => Verdict::Ask,
        },
        None => Verdict::Ask,
    }
}

// Test-only re-imports kept as small as possible. Helpers below this line
// will be deleted as their owning modules are rewritten in later tasks.
#[cfg(test)]
use state::{merge_from_approval_marker_if_needed, rule_for};
#[cfg(test)]
use sweeper::timeout_resolved_event;
