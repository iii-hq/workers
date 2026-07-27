//! Harness integration E2E runner (see README.md; the architecture
//! spec lives in the iii repo's harness-evaluation tech spec).
//!
//! Deterministic regression track: each scenario boots a fresh isolated iii
//! stack, replaces only the `router::*` boundary with a strict scripted
//! worker, and verifies structured public evidence.

pub mod canonical;
pub mod evidence_data;
pub mod expand;
pub mod fixtures;
pub mod scenario;
pub mod scenarios;
pub mod stack;
pub mod types;

pub(crate) mod artifacts;
pub(crate) mod client;
pub(crate) mod deadline;
pub(crate) mod discovery;
pub(crate) mod matcher;
pub(crate) mod probe;
pub(crate) mod process;
pub(crate) mod runtime;
pub(crate) mod scripted_router;
pub(crate) mod services;
pub(crate) mod trace_evidence;
