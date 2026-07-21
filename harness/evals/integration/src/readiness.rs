//! Schema-based readiness: never sleep-based. The probe
//! retries until every surface is present or the deadline passes, then
//! reports **every** missing surface by name (classification `setup_error`).

mod catalog;
mod contracts;
mod probe;
mod spec;

pub use catalog::{
    config_failure, has_function, has_registered_trigger, missing_functions, missing_trigger_types,
    registered_trigger_failures, topic_failures,
};
pub use contracts::ExpectedTriggerBinding;
pub use spec::ReadinessSpec;

pub(crate) use catalog::registered_trigger_count;
pub(crate) use contracts::{
    contract_failures, controlled_contracts, router_contract, ExpectedFunctionContract,
};
pub(crate) use probe::{
    probe, registered_trigger_snapshot, wait_for_contracts, wait_for_registered_triggers,
};
pub(crate) use spec::ReadinessReport;
