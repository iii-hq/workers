//! RPC handlers for `storage::*` functions.

use crate::backend::Backend;
use crate::error::StorageError;
use std::collections::HashMap;
use std::sync::Arc;

pub mod delete_object;
pub mod get_object;
pub mod presign_url;
pub mod put_object;

/// Maximum inline body size accepted by `putObject` and returned by `getObject`.
/// 10 MiB matches the spec.
pub const INLINE_BODY_CAP: u64 = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    /// Keyed by the **worker-facing bucket name** (config map key).
    pub backends: Arc<HashMap<String, Arc<dyn Backend>>>,
}

impl AppState {
    pub fn backend(&self, bucket: &str) -> Result<&Arc<dyn Backend>, StorageError> {
        self.backends
            .get(bucket)
            .ok_or_else(|| StorageError::UnknownBucket {
                bucket: bucket.to_string(),
            })
    }
}

/// JSON-string error body, mirroring iii-database's err_to_str.
pub fn err_to_str(e: StorageError) -> String {
    e.to_wire_string()
}
