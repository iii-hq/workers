//! Per-scenario stack planning and supervision.
//!
//! Engine facts verified against the pinned source:
//! - `iii-worker-manager` accepts `config: { port, host }`;
//! - `configuration` accepts an fs adapter directory;
//! - enabled-by-default builtins must be listed when a config file is used.

mod bins;
mod config;
mod layout;
mod manifest;
mod supervisor;

#[cfg(test)]
mod tests;

pub use crate::process::EarlyExit;
pub use bins::StackBins;
pub use config::{
    expected_config_entries, expected_harness_config_entry, render_engine_yaml, render_seed,
    WORKER_START_ORDER,
};
pub use layout::RunLayout;
pub use supervisor::{free_loopback_port, Stack};

/// Compatibility name retained for callers of the original stack API.
pub type RunPaths = RunLayout;
