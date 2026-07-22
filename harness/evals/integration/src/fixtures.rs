//! Scenario compilation, validation, and selection over the code registry
//! in [`crate::scenarios`].
//!
//! Every authoring defect the type system cannot reject is reported before
//! the stack starts. The public facade preserves the original `fixtures` API
//! while keeping each responsibility in a focused module.

mod discovery;
mod loading;

pub use discovery::scenario_fixtures;
pub use loading::ScenarioFixture;

#[cfg(test)]
mod tests;
