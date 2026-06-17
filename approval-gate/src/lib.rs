//! approval-gate — the policy and decision surface for human-held
//! function calls (tech-specs/2026-06-agentic/approval-gate.md).
//!
//! Three surfaces, one worker:
//! 1. The gate — `approval::gate`, a `pre_trigger` hook the worker binds
//!    itself at startup; answers `continue` / `deny` / `hold`.
//! 2. The decision plane — `approval::resolve` plus the per-session
//!    settings RPCs (human/console-only).
//! 3. The pending inbox — an ephemeral index of held calls
//!    (`approval::list-pending` / `approval::get-pending`) plus the
//!    `approval::pending-created` / `approval::pending-resolved` trigger
//!    types notification workers bind to.
//!
//! The worker codes against the greenfield harness contracts
//! (`harness::hook::pre-trigger`, `harness::function::resolve`,
//! `harness::turn-completed` — harness.md § Hooks, § API Reference);
//! those bindings are best-effort so the worker also boots standalone.

pub mod config;
pub mod configuration;
pub mod decision;
pub mod denial;
pub mod error;
pub mod events;
pub mod functions;
pub mod harness;
pub mod manifest;
pub mod pending;
pub mod policy;
pub mod redact;
pub mod session;
pub mod settings;
pub mod state;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod types;
