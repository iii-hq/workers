//! Shared runtime state for the folded `coder::*` code surface.
//!
//! `ConfigCell` is the hot-swappable config snapshot every cfg-taking code
//! handler reads per call. The `Arc<RwLock<Arc<CoderConfig>>>` shape lets a
//! handler take a `read().await` and cheaply `clone()` the inner `Arc` out
//! without holding the lock across its work, while a config reload
//! whole-snapshot replaces the inner `Arc` under the write lock. The
//! `PathResolver` is the security jail and is hot-swapped with the matching
//! config under the worker's `reload_lock` (see `configuration.rs`). Handlers
//! clone both Arcs per call so they never hold a lock across filesystem work.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::code::config::CoderConfig;
use crate::code::path::PathResolver;

/// Hot-swappable snapshot shared with every cfg-taking code handler.
pub type ConfigCell = Arc<RwLock<Arc<CoderConfig>>>;
pub type ResolverCell = Arc<RwLock<Arc<PathResolver>>>;

#[derive(Clone)]
pub struct CodeCells {
    pub config: ConfigCell,
    pub resolver: ResolverCell,
}
