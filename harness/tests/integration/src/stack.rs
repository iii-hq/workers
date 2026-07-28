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
pub use layout::RunLayout;
pub use supervisor::{free_loopback_port, Stack, StackBootFailure};
