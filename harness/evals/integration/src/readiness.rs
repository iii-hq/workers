//! Schema-based readiness (spec § Readiness): never sleep-based. The probe
//! retries until every surface is present or the deadline passes, then
//! reports **every** missing surface by name (classification `setup_error`).

mod catalog;
mod probe;
mod spec;

pub use catalog::{
    config_failure, has_function, has_registered_trigger, missing_functions, missing_trigger_types,
    topic_failures,
};
pub use probe::probe;
pub use spec::{ReadinessReport, ReadinessSpec};

pub(crate) use probe::{wait_for_functions, wait_for_registered_triggers};
