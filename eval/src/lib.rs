//! The Harness E2E improvement loop: one cycle record started from an
//! objective or a finished E2E execution, durable evidence, a frozen plan,
//! a budget and a decision computed in code.

pub mod candidate;
pub mod contract;
pub mod cost;
pub mod coverage;
pub mod decision;
pub mod diagnostics;
pub mod error;
pub mod evidence;
pub mod functions;
pub mod ids;
pub mod insight;
pub mod locks;
pub mod manifest;
pub mod pull_request;
pub mod queue;
pub mod runtime;
pub mod stages;
pub mod state;
pub mod surface;
pub mod ui;
