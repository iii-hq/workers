//! Scenario execution lifecycle:
//! Allocate → Boot → Arm → Send → Fault/Release → Await →
//! Collect → Grade → Teardown → Report.
//! (Observe inserts Probe/wait-start between Arm and Send, then waits for
//! observer shutdown after Await before Collect.)
//!
//! Every phase returns [`crate::runtime::RunError`]. Classification is derived
//! once after process state has been inspected.

pub mod floor;

mod observe;
mod phases;
mod report;
mod runner;
mod state;

pub use observe::{observe_scenario, ObserveOutcome, ObserveReadyV1, ObserveResultV1};
pub use runner::{run_scenario, RunOutcome};

#[cfg(test)]
mod tests;
