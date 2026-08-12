//! sandbox-code-runner: run Node/Python in iii-sandbox microVMs, register bus
//! functions whose handlers execute inside them, tear them down.
//!
//! This worker executes nothing itself — every run and every handler call is
//! delegated over the bus to the iii-sandbox daemon's `sandbox::*` triggers.

pub mod config;
pub mod engine;
pub mod error;
pub mod events;
pub mod fleet_watch;
pub mod functions;
pub mod manager;
pub mod manifest;
pub mod runner;
pub mod ui;
