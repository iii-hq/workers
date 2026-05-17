//! Approval gate. Subscribes to `agent::before_function_call` and decides
//! every call via the layered rules engine (`rules::evaluate`). Allow →
//! `{block:false}`. Deny → `{block:true, denial:Policy{rule_permission,
//! rule_pattern}}`. Ask → write a Pending record and wait for
//! `approval::resolve`.

pub mod config;
pub mod delivery;
pub mod intercept;
pub mod manifest;
pub mod record;
pub mod register;
pub mod resolve;
pub mod rules;
pub mod state;
pub mod wire;

pub use config::WorkerConfig;
pub use delivery::{
    handle_consume, handle_list_pending, handle_sweep_session, handle_tick_timeouts,
    CONSUME_DEFAULT_LIMIT,
};
pub use intercept::handle_intercept;
pub use record::{Outcome, Record, Status, IN_FLIGHT_GRACE_MS};
pub use register::{
    build_approval_resolved_event, register, run_tick, IIIWaker, OrchestratorWaker, Refs,
    FN_CONSUME, FN_LIST_PENDING, FN_LOOKUP_RECORD, FN_RESOLVE, FN_SWEEP_SESSION, FN_TICK_TIMEOUTS,
    STATE_SCOPE,
};
pub use rules::LayeredRules;
pub use resolve::{handle_lookup_record, handle_resolve};
pub use state::{FunctionExecutor, IiiFunctionExecutor, IiiStateBus, StateBus};
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
