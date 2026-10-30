//! GitHub CLI (`gh`) as an iii worker: typed `github::*` functions for the
//! high-traffic pr/issue/repo/run/workflow/release/search surface, plus
//! `github::exec` (argv passthrough) and `github::api` (any REST endpoint)
//! escape hatches. The binary is a thin wiring shim; all logic lives here so
//! `tests/` can exercise the contract.

pub mod config;
pub mod configuration;
pub mod events;
pub mod functions;
pub mod gh;
pub mod ui;
pub mod webhooks;

/// The name the github binary registers under, unless its supervisor names it.
pub const WORKER_NAME: &str = "github";

/// The name this worker registers under: a non-empty `III_WORKER_NAME` (the
/// name a supervisor gives it, which the SDK applies), else [`WORKER_NAME`].
pub fn worker_name() -> String {
    std::env::var("III_WORKER_NAME")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| WORKER_NAME.to_string())
}
