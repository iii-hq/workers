//! Shared runtime state for the folded `coder::*` code surface.
//!
//! `ConfigCell` is the hot-swappable config snapshot every cfg-taking code
//! handler reads per call. The `Arc<RwLock<Arc<CoderConfig>>>` shape lets a
//! handler take a `read().await` and cheaply `clone()` the inner `Arc` out
//! without holding the lock across its work, while a config reload
//! whole-snapshot replaces the inner `Arc` under the write lock. The
//! `PathResolver` is NOT stored here — it is the security jail, built once at
//! boot and swapped only under the worker's `reload_lock` (see
//! `configuration.rs`), never per call.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::code::config::CoderConfig;

/// Hot-swappable snapshot shared with every cfg-taking code handler.
pub type ConfigCell = Arc<RwLock<Arc<CoderConfig>>>;
