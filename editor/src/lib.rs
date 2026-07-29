//! Editor surface for code, built on top of the workers that already exist.
//!
//! `editor` opens no files and spawns no processes. Reads, writes and git all
//! go through `shell`, which owns the filesystem jail; the console page is the
//! shared `@iii-dev/console-ui` component set. What this worker adds is the
//! part neither of those has: a diff, a set of git marks, a ranked file
//! search, and a save that refuses to clobber.

pub mod bus;
pub mod config;
pub mod configuration;
pub mod diff;
pub mod functions;
pub mod fuzzy;
pub mod git;
pub mod lang;
pub mod manifest;
pub mod surface;
pub mod tree;
pub mod ui;
pub mod workspace;
