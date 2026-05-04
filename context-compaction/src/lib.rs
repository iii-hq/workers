//! Context compaction worker.
//!
//! Two surfaces split across modules:
//!
//! - [`watcher`] — reactive: subscribes to the `agent::events` stream and
//!   republishes `error_kind == "context_overflow"` events to the
//!   `agent::transform_context` pubsub topic. Stateless.
//!
//! - [`compactor`] — proactive: subscribes to the same stream, accumulates
//!   token usage per session, and triggers `session::compact` when usage
//!   crosses a configured threshold. Per-session [`Compactor`] state lives
//!   in a [`CompactorRegistry`].
//!
//! Both surfaces use the engine's `stream` trigger type (not `subscribe`) —
//! `agent::events` is an iii stream, not a pubsub topic.

pub mod compactor;
pub mod watcher;

pub use compactor::{
    extract_file_ops, register as register_compactor, CompactionConfig, CompactionDetails,
    CompactionError, Compactor, CompactorHandle, CompactorRegistry, IiiBus, IiiSdkBus, SummariseFn,
};
pub use watcher::{payload_signals_overflow, register as register_watcher, WatcherHandle};

use std::sync::Arc;

use iii_sdk::{IIIError, III};

/// Register both surfaces (watcher + compactor) on `iii`. Returns handles
/// for both so the caller can deregister them on shutdown.
pub fn register_with_iii<F: SummariseFn + 'static>(
    iii: &III,
    summariser: Arc<F>,
) -> Result<Handles, IIIError> {
    Ok(Handles {
        watcher: register_watcher(iii)?,
        compactor: register_compactor(iii, summariser)?,
    })
}

/// Compound handle returned by [`register_with_iii`].
pub struct Handles {
    pub watcher: WatcherHandle,
    pub compactor: CompactorHandle,
}

impl Handles {
    pub fn unregister_all(self) {
        self.watcher.unregister_all();
        self.compactor.unregister_all();
    }
}
