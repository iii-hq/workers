//! approval-gate — the policy and decision surface for human-held
//! function calls (tech-specs/2026-06-agentic/approval-gate.md).
//!
//! Three surfaces, one worker:
//! 1. The gate — `approval::gate`, a `pre_call` hook: the worker observes
//!    `harness::hook::on-pre-call` with it at startup; answers `continue` /
//!    `deny` / `hold`.
//! 2. The decision plane — `approval::resolve` plus the per-session
//!    settings RPCs (human/console-only).
//! 3. The pending inbox — an ephemeral index of held calls
//!    (`approval::list-pending` / `approval::get-pending`) plus the
//!    `approval::on-pending-created` / `approval::on-pending-resolved` event
//!    functions notification workers observe.
//!
//! The worker codes against the greenfield harness contracts
//! (`harness::hook::on-pre-call`, `harness::function::resolve`,
//! `harness::on-turn-completed` — harness.md § Hooks, § API Reference);
//! its observers stay pending while a sibling is absent, so the worker also
//! boots standalone.

pub mod config;
pub mod configuration;
pub mod decision;
pub mod denial;
pub mod error;
pub mod events;
pub mod filesystem_access;
pub mod filesystem_access_state;
pub mod functions;
pub mod harness;
pub mod manifest;
pub mod pending;
pub mod permissions;
pub mod redact;
pub mod session;
pub mod settings;
pub mod shell_config;
pub mod state;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod types;
