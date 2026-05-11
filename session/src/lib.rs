//! Unified session worker library.
//!
//! Exposes two sibling namespaces from a single crate:
//!
//! - [`tree`] — branching session storage, registered as `session-tree::*`.
//! - [`inbox`] — per-session inbox queues, registered as `session-inbox::*`.

pub mod config;
pub mod inbox;
pub mod manifest;
pub mod tree;
