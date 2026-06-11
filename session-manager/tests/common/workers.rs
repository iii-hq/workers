//! In-process registration of the production `session-manager` surface
//! against the shared SDK handle.
//!
//! Re-uses the production entry points in the binary's fs-mode boot
//! order — six public trigger types, the internal store-events feed,
//! the `session::store::*` protocol, and the 14 handlers — backed by a
//! real `FsStore` over a per-binary tempdir, so @engine scenarios
//! exercise identical code paths end to end and can read the JSONL
//! files back for persistence assertions.

use std::path::PathBuf;
use std::sync::Arc;

use iii_sdk::III;
use tokio::sync::OnceCell;

use session_manager::config::WorkerConfig;
use session_manager::events::{
    register_store_events_type, register_trigger_types, Emitter, EventSink, IiiDeliverer,
};
use session_manager::functions::{self, store_protocol, Deps};
use session_manager::service::SessionService;
use session_manager::store::{FsStore, SessionStore};

pub struct Shared {
    /// The main (fs-mode) instance's data directory; steps read the
    /// per-session JSONL files from here.
    pub data_dir: PathBuf,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<III>) -> Arc<Shared> {
    SHARED
        .get_or_init(|| async {
            let cfg = WorkerConfig::default();

            // Leaked tempdir that lives for the test binary lifetime.
            let tmp = tempfile::tempdir().expect("create main data_dir tempdir");
            let data_dir = tmp.keep();

            // Same boot order as src/main.rs in fs mode.
            let sets = register_trigger_types(iii);
            let bridges = register_store_events_type(iii);
            let emitter = Arc::new(Emitter::with_bridges(
                sets,
                Arc::new(IiiDeliverer::new(iii.clone())),
                bridges,
            ));

            let store: Arc<dyn SessionStore> =
                Arc::new(FsStore::new(&data_dir).expect("open main FsStore"));
            store_protocol::register_store_protocol(iii, store.clone(), emitter.clone());

            let service = Arc::new(SessionService::new(store, &cfg));
            let sink: Arc<dyn EventSink> = emitter;
            let deps = Arc::new(Deps { service, sink });
            functions::register_all(iii, &deps);

            // Give the SDK a beat to publish the registrations before
            // scenarios start triggering them.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            Arc::new(Shared { data_dir })
        })
        .await
        .clone()
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}
