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
    PlaygroundOutcome, PlaygroundReadyV1, PlaygroundResultV1, playground_scenario,
};
pub use runner::{RunOutcome, run_scenario};

#[cfg(test)]
mod tests;
