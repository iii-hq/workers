//! Editor surface for code, built on top of the workers that already exist.
//!
//! `editor` opens no files and spawns no processes. Reads, writes and git all
//! go through `shell`, which owns the filesystem jail; the console page edits
//! in the shared `@iii-dev/console-ui` Monaco and renders files and diffs with
//! `@pierre/diffs`. What this worker adds is the part none of those has: a
//! diff, a set of git marks, a ranked file search, and a save that refuses to
//! clobber.

pub mod bus;
pub mod config;
pub mod configuration;
pub mod diff;
pub mod events;
pub mod functions;
pub mod fuzzy;
pub mod git;
pub mod lang;
pub mod manifest;
pub mod observe;
pub mod surface;
pub mod tree;
pub mod ui;
pub mod workspace;
