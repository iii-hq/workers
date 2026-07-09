//! `harness` — the thin durable turn loop that wires `session-manager`,
//! `context-manager`, and `llm-router` into an agent loop
//! (tech-specs/2026-06-agentic/harness.md).
//!
//! It owns sequencing and nothing else: take an incoming message, persist it,
//! assemble a context, stream a completion, persist the result, execute any
//! function calls, and repeat until the turn stops — as durable enqueued steps
//! so a crash resumes mid-turn.

pub mod clients;
pub mod config;
pub mod configuration;
pub mod contract;
pub mod deferred;
pub mod deps;
pub mod discovery;
pub mod error;
pub mod events;
pub mod filesystem_grants;
pub mod filesystem_scope;
pub mod functions;
pub mod hooks;
pub mod ids;
pub mod locks;
pub mod manifest;
pub mod policy;
pub mod prompt;
pub mod state;
pub mod subagent;
pub mod subscriptions;
pub mod surface;
pub mod trigger;
pub mod turn_loop;
pub mod turn_queues;
pub mod types;
