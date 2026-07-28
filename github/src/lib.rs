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
