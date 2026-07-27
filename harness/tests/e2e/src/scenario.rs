//! Scenario execution lifecycle: allocate → boot → arm → send/observe →
//! await → collect → grade → teardown → report.
//!
//! Every phase returns [`crate::runtime::RunError`]. Classification is derived
//! once after process state has been inspected.

pub mod floor;

mod phases;
mod playground;
mod report;
mod runner;
mod state;

pub use playground::{
    playground_scenario, PlaygroundOutcome, PlaygroundReadyV1, PlaygroundResultV1,
};
pub use runner::{run_scenario, RunOutcome};

#[cfg(test)]
mod tests;
