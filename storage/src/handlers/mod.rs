//! RPC handlers for `storage::*` functions.

use crate::backend::Backend;
use crate::error::StorageError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod delete_object;
pub mod get_object;
pub mod head_object;
pub mod list_buckets;
pub mod list_objects;
pub mod presign_post;
pub mod presign_url;
pub mod put_object;

/// Hard safety cap for the intentionally small inline convenience calls.
/// Direct signed HTTP transfers are the supported path for files.
pub const INLINE_BODY_CAP: u64 = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    /// Keyed by the **worker-facing bucket name** (config map key). Wrapped in
    /// an `RwLock` so the configuration-change handler can hot-swap the whole
    /// map without restarting the worker.
    pub backends: Arc<RwLock<HashMap<String, Arc<dyn Backend>>>>,
    /// Short-lived gate shared with the local HTTP runtime. A config update
    /// takes the write side only while publishing prepared resources; requests
    /// take the read side while choosing their backend/service generation.
    pub reconfigure_gate: Arc<RwLock<()>>,
}

impl AppState {
    pub fn new(backends: HashMap<String, Arc<dyn Backend>>) -> Self {
        Self {
            backends: Arc::new(RwLock::new(backends)),
            reconfigure_gate: Arc::new(RwLock::new(())),
        }
    }

    pub async fn backend(&self, bucket: &str) -> Result<Arc<dyn Backend>, StorageError> {
        let _gate = self.reconfigure_gate.read().await;
        let backends = self.backends.read().await;
        backends
            .get(bucket)
            .cloned()
            .ok_or_else(|| StorageError::UnknownBucket {
                bucket: bucket.to_string(),
            })
    }

    pub async fn backends_snapshot(&self) -> HashMap<String, Arc<dyn Backend>> {
        let _gate = self.reconfigure_gate.read().await;
        self.backends.read().await.clone()
    }
}

/// JSON-string error body, mirroring database's err_to_str.
pub fn err_to_str(e: StorageError) -> String {
    e.to_wire_string()
}
