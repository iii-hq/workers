//! Scenario loading, validation, discovery, and selection.
//!
//! Every authoring defect is reported before the stack starts. The public
//! facade preserves the original `fixtures` API while keeping each
//! responsibility in a focused module.

mod discovery;
mod loading;
mod script_validation;

pub use discovery::{ensure_scenario_id_available, scenario_fixtures};
pub use loading::ScenarioFixture;
pub use script_validation::validate_script;

#[cfg(test)]
mod tests;
