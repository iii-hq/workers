//! RPC handlers for `storage::*` functions.

use crate::backend::factory::LocalBackendCtx;
use crate::backend::Backend;
use crate::error::StorageError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod delete_object;
pub mod get_object;
pub mod head_object;
pub mod presign_url;
pub mod put_object;

/// Maximum inline body size accepted by `putObject` and returned by `getObject`.
/// 10 MiB matches the spec.
pub const INLINE_BODY_CAP: u64 = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    /// Keyed by the **worker-facing bucket name** (config map key). Wrapped in
    /// an `RwLock` so the configuration-change handler can hot-swap the whole
    /// map without restarting the worker.
    pub backends: Arc<RwLock<HashMap<String, Arc<dyn Backend>>>>,
    /// Boot-time rustfs sidecar context, reused when rebuilding local backends
    /// on a config change (the sidecar is never respawned). `None` when no
    /// `provider: local` bucket was configured at boot.
    pub local_ctx: Option<LocalBackendCtx>,
}

impl AppState {
    pub async fn backend(&self, bucket: &str) -> Result<Arc<dyn Backend>, StorageError> {
        let backends = self.backends.read().await;
        backends
            .get(bucket)
            .cloned()
            .ok_or_else(|| StorageError::UnknownBucket {
                bucket: bucket.to_string(),
            })
    }
}

/// JSON-string error body, mirroring database's err_to_str.
pub fn err_to_str(e: StorageError) -> String {
    e.to_wire_string()
}
