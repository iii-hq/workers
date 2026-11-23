//! Shared handler dependencies: the engine client, the hot-swappable
//! store and config cells, and the event emitter. One struct for
//! functions, hooks, and extraction so a `data_dir` reload swaps the
//! store under every path at once.

use std::sync::Arc;

use iii_sdk::IIIClient;

use crate::config::WorkerConfig;
use crate::configuration::{ConfigCell, StoreCell};
use crate::events::Emitter;
use crate::store::Store;

#[derive(Clone)]
pub struct Deps {
    pub hook_binding: Arc<crate::hook_binding::HookBinding>,
    pub iii: Arc<IIIClient>,
    pub store: StoreCell,
    pub config: ConfigCell,
    pub emitter: Arc<Emitter>,
}

impl Deps {
    /// Cheap refcount bump of the live store; never hold across a reload.
    pub async fn store(&self) -> Arc<Store> {
        self.store.read().await.clone()
    }

    pub async fn config(&self) -> Arc<WorkerConfig> {
        self.config.read().await.clone()
    }
}

/// Deps over a store in `dir` and no engine: with embeddings off in `cfg`,
/// nothing reaches the client.
#[cfg(test)]
pub(crate) fn offline_deps(dir: &std::path::Path, cfg: WorkerConfig) -> Deps {
    let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
    let store = Store::open(dir.to_path_buf()).unwrap();
    Deps {
        hook_binding: Arc::default(),
        emitter: Arc::new(Emitter::new(iii.clone())),
        iii,
        store: Arc::new(tokio::sync::RwLock::new(Arc::new(store))),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(cfg))),
    }
}
