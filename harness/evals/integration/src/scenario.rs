//! Scenario execution lifecycle:
//! Allocate → Boot → Probe → Arm → Send → Fault/Release → Await →
//! Collect → Grade → Teardown → Report.
//!
//! Every phase returns [`crate::runtime::RunError`]. Classification is derived
//! once after process state has been inspected.

mod phases;
mod report;
mod runner;
mod state;

pub use runner::{run_scenario, RunOutcome};

#[cfg(test)]
mod tests;
