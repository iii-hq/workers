//! Schema-based readiness (spec § Readiness): never sleep-based. The probe
//! retries until every surface is present or the deadline passes, then
//! reports **every** missing surface by name (classification `setup_error`).

mod catalog;
mod contracts;
mod probe;
mod spec;

pub use catalog::{
    config_failure, has_function, has_registered_trigger, missing_functions, missing_trigger_types,
    registered_trigger_count, registered_trigger_failures, topic_failures,
};
pub use contracts::{
    contract_failures, validation_schema, ExpectedFunctionContract, ExpectedTriggerBinding,
    SchemaComparison,
};
pub use probe::probe;
pub use spec::{ReadinessReport, ReadinessSpec};

pub(crate) use contracts::{controlled_contracts, router_contract};
pub(crate) use probe::{
    registered_trigger_snapshot, wait_for_contracts, wait_for_registered_triggers,
};
