//! RPC handlers for `storage::*` functions.

use crate::backend::Backend;
use crate::config::WorkerConfig;
use crate::error::StorageError;
use crate::material::BucketError;
use crate::secrets::SecretCache;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod buckets;
pub mod delete_object;
pub mod get_object;
pub mod head_object;
pub mod list_buckets;
pub mod list_objects;
pub mod presign_post;
pub mod presign_url;
pub mod put_object;
pub mod test_bucket;

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
    /// The configuration the backends were built from and why the other
    /// buckets have none. Published together with `backends`.
    pub catalog: Arc<RwLock<Catalog>>,
    /// Resolved `secret://` / `env://` values (see `crate::secrets`).
    pub secrets: Arc<SecretCache>,
}

/// The applied configuration, as `storage::listBuckets` reports it.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// The live configuration (bucket entries that failed to parse are in
    /// `config.invalid`).
    pub config: WorkerConfig,
    /// Why a configured bucket has no backend, by bucket.
    pub errors: HashMap<String, BucketError>,
    /// Why a bucket's notifications source is not running (the bucket
    /// itself works; its object triggers do not fire).
    pub notification_errors: HashMap<String, String>,
}

impl AppState {
    pub fn new(backends: HashMap<String, Arc<dyn Backend>>) -> Self {
        Self::with_secrets(backends, Arc::new(SecretCache::default()))
    }

    pub fn with_secrets(
        backends: HashMap<String, Arc<dyn Backend>>,
        secrets: Arc<SecretCache>,
    ) -> Self {
        Self {
            backends: Arc::new(RwLock::new(backends)),
            reconfigure_gate: Arc::new(RwLock::new(())),
            catalog: Arc::new(RwLock::new(Catalog::default())),
            secrets,
        }
    }

    /// The live backend of `bucket`: `BUCKET_UNAVAILABLE` when it is
    /// configured but could not be built, `UNKNOWN_BUCKET` when it is not
    /// configured.
    pub async fn backend(&self, bucket: &str) -> Result<Arc<dyn Backend>, StorageError> {
        let _gate = self.reconfigure_gate.read().await;
        if let Some(backend) = self.backends.read().await.get(bucket) {
            return Ok(backend.clone());
        }
        match self.catalog.read().await.errors.get(bucket) {
            Some(error) => Err(StorageError::BucketUnavailable {
                bucket: bucket.to_string(),
                stage: error.stage.as_str().to_string(),
                message: error.message.clone(),
            }),
            None => Err(StorageError::UnknownBucket {
                bucket: bucket.to_string(),
            }),
        }
    }

    pub async fn catalog_snapshot(&self) -> Catalog {
        let _gate = self.reconfigure_gate.read().await;
        self.catalog.read().await.clone()
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
