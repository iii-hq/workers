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

/// The one rule every handler that takes an object `key` applies before the
/// request reaches a backend.
///
/// S3 and R2 carry the key in the request path (`/bucket/key`), GCS in a URL,
/// and a presigned URL ends up in a browser. URL parsers, proxies and servers
/// resolve `.` and `..` segments, also written `%2e%2e`, so `/scratch/../escape`
/// is served as `/escape`: another bucket, or a signature over a path that is
/// not the one that arrives. Percent-encoding the dots can't prevent that, so
/// such keys are refused.
pub fn validate_key(key: &str) -> Result<(), StorageError> {
    if key.is_empty() {
        return Err(StorageError::ConfigError {
            message: "key must not be empty".into(),
        });
    }
    match key.split('/').find(|segment| matches!(*segment, "." | "..")) {
        Some(segment) => Err(StorageError::InvalidKey {
            reason: format!(
                "an object key can't have a '.' or '..' segment; rename the '{segment}' segment in {key:?}"
            ),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use serde_json::{from_value, json};

    /// Runs every handler that takes a `key` against one mock-backed bucket.
    async fn every_key_handler(state: &AppState, key: &str) -> Vec<Result<(), String>> {
        let bucket = "b";
        vec![
            super::put_object::handle(
                state,
                from_value(json!({"bucket": bucket, "key": key, "body_base64": ""})).unwrap(),
            )
            .await
            .map(|_| ()),
            super::get_object::handle(
                state,
                from_value(json!({"bucket": bucket, "key": key})).unwrap(),
            )
            .await
            .map(|_| ()),
            super::head_object::handle(
                state,
                from_value(json!({"bucket": bucket, "key": key})).unwrap(),
            )
            .await
            .map(|_| ()),
            super::delete_object::handle(
                state,
                from_value(json!({"bucket": bucket, "key": key})).unwrap(),
            )
            .await
            .map(|_| ()),
            super::presign_url::handle(
                state,
                from_value(json!({"bucket": bucket, "key": key, "method": "GET"})).unwrap(),
            )
            .await
            .map(|_| ()),
            super::presign_post::handle(
                state,
                from_value(json!({"bucket": bucket, "key": key, "content_type": "text/plain"}))
                    .unwrap(),
            )
            .await
            .map(|_| ()),
        ]
    }

    fn state() -> (AppState, Arc<MockBackend>) {
        let mock = Arc::new(MockBackend::default());
        let backends = HashMap::from([("b".to_string(), mock.clone() as Arc<dyn Backend>)]);
        (AppState::new(backends), mock)
    }

    #[tokio::test]
    async fn every_handler_refuses_a_key_with_a_dot_segment() {
        for key in ["..", ".", "../escape", "a/../b", "a/./b", "a/b/..", "./a"] {
            let (state, mock) = state();
            for result in every_key_handler(&state, key).await {
                let err = result.expect_err(key);
                assert!(err.contains("INVALID_KEY"), "{key}: {err}");
                assert!(err.contains("'.' or '..' segment"), "{key}: {err}");
            }
            assert!(
                mock.calls.lock().unwrap().is_empty(),
                "{key} reached the backend"
            );
        }
    }

    #[tokio::test]
    async fn dots_that_are_not_a_whole_segment_are_fine() {
        for key in [
            "a.b",
            "...",
            "..a",
            "a..",
            ".hidden",
            "a/.b/c..d",
            "dir/my file&+%.txt",
        ] {
            let (state, mock) = state();
            for result in every_key_handler(&state, key).await {
                if let Err(err) = result {
                    assert!(!err.contains("INVALID_KEY"), "{key}: {err}");
                }
            }
            assert_eq!(mock.calls.lock().unwrap().len(), 6, "{key}");
        }
    }
}
