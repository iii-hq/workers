//! `storage::putObject` — write an object to a configured bucket.

use super::{err_to_str, AppState, INLINE_BODY_CAP};
use crate::backend::PutReq as BackendPutReq;
use crate::error::{backend_error_to_storage, StorageError};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PutReq {
    pub bucket: String,
    pub key: String,
    /// Base64-encoded body. The legacy field name `body` is accepted as an
    /// alias for backward compatibility with clients that pre-date the
    /// rename.
    #[serde(alias = "body")]
    pub body_base64: String,
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PutResp {
    pub etag: String,
    pub size: u64,
    pub version_id: Option<String>,
}

pub async fn handle(state: &AppState, req: PutReq) -> Result<PutResp, String> {
    if req.key.is_empty() {
        return Err(err_to_str(StorageError::ConfigError {
            message: "key must not be empty".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    // Reject before allocating: a base64 string of length L decodes to at
    // most (L*3)/4 bytes. If even the upper bound exceeds the cap, refuse
    // without ever allocating the decode buffer.
    let max_decoded: u64 = (req.body_base64.len() as u64).saturating_mul(3) / 4;
    if max_decoded > INLINE_BODY_CAP {
        return Err(err_to_str(StorageError::BodyTooLarge {
            size: max_decoded,
            cap: INLINE_BODY_CAP,
        }));
    }
    let bytes = B64.decode(&req.body_base64).map_err(|e| {
        err_to_str(StorageError::InvalidBase64 {
            reason: e.to_string(),
        })
    })?;
    if bytes.len() as u64 > INLINE_BODY_CAP {
        return Err(err_to_str(StorageError::BodyTooLarge {
            size: bytes.len() as u64,
            cap: INLINE_BODY_CAP,
        }));
    }
    let content_type = req
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let resp = backend
        .put(BackendPutReq {
            key: req.key.clone(),
            body: bytes,
            content_type,
            cache_control: req.cache_control,
            metadata: req.metadata,
        })
        .await
        .map_err(|e| {
            err_to_str(backend_error_to_storage(
                e,
                backend.provider(),
                &req.bucket,
                &req.key,
            ))
        })?;
    Ok(PutResp {
        etag: resp.etag,
        size: resp.size,
        version_id: resp.version_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::{MockBackend, MockCall};
    use crate::backend::Backend;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// Returns `(AppState, Arc<MockBackend>)` — keep the typed Arc for
    /// inspection alongside the trait-object stored in AppState.
    fn state() -> (AppState, Arc<MockBackend>) {
        let m = Arc::new(MockBackend::default());
        let mut map = HashMap::new();
        map.insert("uploads".to_string(), m.clone() as Arc<dyn Backend>);
        (
            AppState {
                backends: Arc::new(RwLock::new(map)),
                local_ctx: None,
            },
            m,
        )
    }

    fn req(v: serde_json::Value) -> PutReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn happy_path_decodes_and_forwards() {
        let (st, mock) = state();
        let body = B64.encode(b"hello");
        let resp = super::handle(
            &st,
            req(json!({
                "bucket": "uploads",
                "key": "k",
                "body_base64": body,
                "content_type": "text/plain"
            })),
        )
        .await
        .unwrap();
        assert!(!resp.etag.is_empty());
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            MockCall::Put(p) => {
                assert_eq!(p.body, b"hello");
                assert_eq!(p.content_type, "text/plain");
            }
            other => panic!("expected Put, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_bucket_returns_unknown_bucket_error() {
        let (st, _mock) = state();
        let body = B64.encode(b"x");
        let err = super::handle(
            &st,
            req(json!({"bucket":"missing","key":"k","body_base64":body})),
        )
        .await
        .unwrap_err();
        assert!(err.contains("UNKNOWN_BUCKET"), "got: {err}");
    }

    #[tokio::test]
    async fn invalid_base64_errors() {
        let (st, _mock) = state();
        let err = super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"k","body_base64":"@@@not_b64@@@"})),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_BASE64"), "got: {err}");
    }

    #[tokio::test]
    async fn body_too_large_errors() {
        let (st, _mock) = state();
        let big = vec![0u8; (INLINE_BODY_CAP + 1) as usize];
        let body = B64.encode(&big);
        let err = super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"k","body_base64":body})),
        )
        .await
        .unwrap_err();
        assert!(err.contains("BODY_TOO_LARGE"), "got: {err}");
    }

    #[tokio::test]
    async fn empty_key_errors() {
        let (st, _mock) = state();
        let body = B64.encode(b"x");
        let err = super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"","body_base64":body})),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("CONFIG_ERROR") || err.contains("must not be empty"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn missing_content_type_defaults() {
        let (st, mock) = state();
        let body = B64.encode(b"x");
        super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"k","body_base64":body})),
        )
        .await
        .unwrap();
        let calls = mock.calls.lock().unwrap();
        match &calls[0] {
            MockCall::Put(p) => assert_eq!(p.content_type, "application/octet-stream"),
            _ => panic!(),
        }
    }
}
