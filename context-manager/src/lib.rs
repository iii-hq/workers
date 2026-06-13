//! `context-manager` — model-ready context assembly for the agentic
//! worker family (tech-specs/2026-06-agentic/context-manager.md).
//!
//! Pure transforms over caller-supplied `AgentMessage[]`: token
//! counting, function-result pruning, and history compaction, behind
//! four `context::*` functions. Stateless with respect to conversation
//! storage; the only state is short-lived compaction leases.

pub mod adapters;
pub mod config;
pub mod core;
pub mod error;
pub mod functions;
pub mod manifest;
pub mod ports;
pub mod types;
