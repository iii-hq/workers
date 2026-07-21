//! Scenario execution lifecycle:
//! Allocate → Boot → Probe → Arm → Send → Fault/Release → Await →
//! Collect → Grade → Teardown → Report.
//!
//! Every phase returns [`crate::runtime::RunError`]. Classification is derived
//! once after process state has been inspected.

mod phases;
mod report;
mod runner;
mod serve;
mod state;

pub use runner::{run_scenario, RunOutcome};
pub use serve::{serve_scenario, ServeOutcome, ServeReadyV1, ServeResultV1};

#[cfg(test)]
mod tests;
