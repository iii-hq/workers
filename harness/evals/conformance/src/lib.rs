//! Harness conformance E2E runner (spec:
//! `tech-specs/2026-07-15-harness-evaluation/conformance-e2e.md`).
//!
//! Deterministic regression track: each scenario boots a fresh isolated iii
//! stack, replaces only the `router::*` boundary with a strict scripted
//! worker, and grades structured public evidence.

pub mod artifacts;
pub mod canonical;
pub mod client;
pub mod console_recording;
pub mod expand;
pub mod fixtures;
pub mod grader;
pub mod matcher;
pub mod readiness;
pub mod recorder;
pub mod scenario;
pub mod scripted_router;
pub mod stack;
pub mod types;
