//! Semantic computer use: read an app's accessibility tree, find elements by
//! selector, and act on them by reference, through xa11y. It sits beside the
//! coordinate `Driver`, not inside it, and only native macOS/Windows sessions
//! get one.

mod engine;
mod error;
mod executor;
#[cfg(test)]
pub(crate) mod fake;
mod outline;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod platform;
mod refs;
mod selector;
mod walk;

pub use engine::{Query, Scope, SnapshotOut, Verb};
pub use error::{map_act_error, map_read_error, Code, SemanticError};
pub use executor::SemanticSession;
pub use selector::parse_selector;

use std::sync::Arc;
use std::time::Duration;

/// Builds one accessibility provider. Called on the session's executor
/// thread, once per snapshot; dropping the provider frees its handle cache.
pub type ProviderFactory =
    Arc<dyn Fn() -> xa11y_core::Result<Arc<dyn xa11y_core::Provider>> + Send + Sync>;

/// Most elements one snapshot reads (also the default).
pub const MAX_NODES: usize = 256;
/// Default deepest level below the walk root (the root is level 0).
pub const DEFAULT_MAX_DEPTH: u32 = 6;
/// Largest accepted `max_depth`.
pub const MAX_DEPTH: u32 = 12;
/// Longest `computer::find` wait.
pub const MAX_FIND_TIMEOUT_MS: u32 = 30_000;
/// Pause between `computer::find` attempts.
pub const FIND_POLL: Duration = Duration::from_millis(100);
/// Budget for one tree walk; the walker stops and reports `truncated`.
pub const WALK_DEADLINE: Duration = Duration::from_secs(5);
/// Budget for one semantic action before its effect counts as unknown.
pub const ACT_DEADLINE: Duration = Duration::from_secs(10);
/// Longest `set_value` text (never truncated; longer is refused).
pub const MAX_SET_VALUE_CHARS: usize = 16_384;
/// Longest selector.
pub const MAX_SELECTOR_BYTES: usize = 1_024;
/// Most matches `computer::find` lists.
pub const MAX_FIND_LINES: usize = 50;

/// The marker line every tree text block starts with.
pub fn outline_marker() -> &'static str {
    outline::UNTRUSTED
}
