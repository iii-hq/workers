//! iii-cloud: take any project to iii cloud from the harness. Sign in, check
//! the cloud plan and run the app here the way the cloud runs it, keep secret
//! values out of the agent's context, merge and deploy pinned to the plan the
//! person saw, and follow the deploy to Ready. The binary is a thin wiring
//! shim; all logic lives here so `tests/` can exercise the contract.

pub mod api;
pub mod check;
pub mod cloud;
pub mod config;
pub mod configuration;
pub mod connect;
pub mod console;
pub mod create;
pub mod ctx;
pub mod deploy;
pub mod error;
pub mod functions;
pub mod handoff;
pub mod local;
pub mod problems;
pub mod project;
pub mod run;
pub mod secret;
pub mod status;
pub mod store;
pub mod tool;
pub mod types;
pub mod ui;
pub mod values;
pub mod watch;

/// The name this worker registers under; the vault's consumer lists match on it.
pub const WORKER_NAME: &str = "iii-cloud";

/// Turn off the SDK's recording of function inputs and outputs in spans.
/// `iii-cloud::secret::provide` receives a secret value: without this it would
/// sit in the trace as `iii.payload.json`.
///
/// # Safety
/// The entire process must be single-threaded (it sets an environment variable).
pub unsafe fn disable_trace_payloads() {
    // SAFETY: the caller guarantees the process is single-threaded.
    unsafe { std::env::set_var("III_DISABLE_TRACE_PAYLOADS", "1") };
}
