//! Approval gate. Subscribes to `agent::before_function_call` and blocks calls
//! whose `function_call.function_id` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

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

// Test-only re-imports so the inline `mod tests` below keeps working
// without an unreasonable churn pass over its assertions.
#[cfg(test)]
use intercept::{
    apply_policy_rules, decide_intercept_action, interpret_classifier_reply, ClassifierDecision,
    InterceptAction, PolicyOutcome,
};
#[cfg(test)]
use state::{merge_from_approval_marker_if_needed, rule_for};
#[cfg(test)]
use sweeper::timeout_resolved_event;
