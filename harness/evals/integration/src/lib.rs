//! Harness integration E2E runner (see README.md; the architecture
//! spec lives in the iii repo's harness-evaluation tech spec).
//!
//! Deterministic regression track: each scenario boots a fresh isolated iii
//! stack, replaces only the `router::*` boundary with a strict scripted
//! worker, and grades structured public evidence.

pub mod artifacts;
pub mod canonical;
pub mod client;
pub mod deadline;
pub mod expand;
pub mod fixtures;
pub mod grader;
pub mod matcher;
pub mod process;
pub mod readiness;
pub mod recorder;
pub mod runtime;
pub mod scenario;
pub mod scripted_router;
pub mod services;
pub mod stack;
pub mod types;
