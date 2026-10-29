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

/// The request schema the SDK derives for a typed handler, for handlers
/// that take their payload as JSON and parse it with [`parse_request`].
pub fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
    serde_json::to_value(
        schemars::gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .unwrap_or(serde_json::Value::Null)
}

/// Parse a request payload. A wrong-typed field, a missing one, or (for
/// requests that deny them) an unknown key is a `CONFIG_ERROR` naming the
/// field — never echoing a string value, which may be a credential.
pub fn parse_request<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Result<T, String> {
    // The engine adds its own metadata to payloads (`_caller_worker_id`):
    // keys starting with `_` are its, never a request field.
    let mut value = value;
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|key, _| !key.starts_with('_'));
    }
    serde_path_to_error::deserialize(value).map_err(|error| {
        let path = error.path().to_string();
        let reason = crate::config::sanitize_serde_error(&error.inner().to_string());
        let message = if path.is_empty() || path == "." {
            format!("invalid request: {reason}")
        } else {
            format!("invalid request: `{path}`: {reason}")
        };
        err_to_str(StorageError::ConfigError { message })
    })
}

/// JSON-string error body, mirroring database's err_to_str.
pub fn err_to_str(e: StorageError) -> String {
    e.to_wire_string()
}

/// The one rule every handler that takes an object `key` applies before the
/// request reaches a backend.
///
/// Some requests carry the key in a URL path: a path-style S3 endpoint, and the
/// presigned URLs of S3, R2 and GCS, which a browser then opens. URL parsers
/// resolve `.` and `..` segments the way the WHATWG URL standard does, where
/// `%2e` (any case) is a dot, a backslash is a slash, and a tab, CR or LF is
/// dropped. So `%2e%2e/other/x` in bucket `scratch` can be signed or served as
/// `/other/x`, another bucket. Percent-encoding the dots can't prevent that, so
/// a key with such a segment, read the way those parsers read it, is refused.
pub fn validate_key(key: &str) -> Result<(), StorageError> {
    if key.is_empty() {
        return Err(StorageError::ConfigError {
            message: "key must not be empty".into(),
        });
    }
    let as_url: String = key
        .chars()
        .filter(|c| !matches!(*c, '\t' | '\r' | '\n'))
        .collect::<String>()
        .to_ascii_lowercase()
        .replace("%2e", ".");
    if as_url
        .split(['/', '\\'])
        .any(|segment| matches!(segment, "." | ".."))
    {
        return Err(StorageError::InvalidKey {
            reason: "an object key with a '.' or '..' segment is not supported (a dot may also be written %2e, and a backslash counts as a slash)".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Req {
        name: String,
        #[serde(default)]
        timeout_ms: Option<u64>,
    }

    #[test]
    fn bad_requests_are_config_errors_that_name_the_field_not_the_value() {
        let err =
            parse_request::<Req>(json!({ "name": "a", "timeout_ms": "hunter2" })).unwrap_err();
        assert!(
            err.contains("CONFIG_ERROR") && err.contains("`timeout_ms`"),
            "{err}"
        );
        assert!(!err.contains("hunter2"), "{err}");
        let err =
            parse_request::<Req>(json!({ "name": "a", "create_bucket_yes": true })).unwrap_err();
        assert!(err.contains("unknown field `create_bucket_yes`"), "{err}");
        let err = parse_request::<Req>(json!({ "timeout_ms": -1 })).unwrap_err();
        assert!(err.contains("CONFIG_ERROR"), "{err}");
        assert!(parse_request::<Req>(json!({ "name": "a" })).is_ok());
        assert!(
            parse_request::<Req>(json!({ "name": "a", "_caller_worker_id": "w" })).is_ok(),
            "engine metadata is not a request field"
        );
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
        for key in [
            "..",
            ".",
            "../escape",
            "a/../b",
            "a/./b",
            "a/b/..",
            "./a",
            // As a URL parser reads them: %2e is a dot, a backslash a slash,
            // and a tab, CR or LF is dropped.
            "%2e%2e/other/x",
            ".%2E/other/x",
            "%2e./x",
            "a/%2E/b",
            "a\\..\\..\\other\\x",
            ".\t./other/x",
            "..\r\n/x",
        ] {
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
            "a%2eb",
            "%2e%2e%2e",
            "back\\slash.txt",
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
