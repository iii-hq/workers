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

pub struct Deps {
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
