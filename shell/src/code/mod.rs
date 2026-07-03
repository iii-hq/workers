//! The folded `coder::*` code surface.
//!
//! `coder` was a separate path-jailed file worker; the merge folds its 9
//! agent-ergonomic file functions (`coder::read-file`/`search`/`update-file`/
//! `create-file`/`delete-file`/`list-folder`/`tree`/`move`/`info`) into the
//! `shell` binary verbatim — same function ids, same C2xx error codes, same
//! wire shapes. They run over a [`path::PathResolver`] (multi-root jail with
//! glob "visible-but-locked" protection) built from the merged config's
//! `code:` block, sharing the [`crate::path`] canonicalization leaf with the
//! `shell::fs::*` jail.
//!
//! These handlers keep their original `std::fs`-over-`PathResolver` logic; the
//! heaviest unbounded scans (`tree`/`search` recursion, batched reads) are
//! offloaded via `spawn_blocking` so a large traversal cannot stall the shared
//! tokio runtime that also dispatches `shell::exec`/jobs/config-reload.

pub mod config;
pub mod error;
pub mod functions;
pub mod path;
pub mod state;

pub use functions::register_all;
