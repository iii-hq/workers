//! `storage::getObject` — read an object inline.

use super::{err_to_str, AppState, INLINE_BODY_CAP};
use crate::backend::GetReq as BackendGetReq;
use crate::error::{backend_error_to_storage, StorageError};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetReq {
    pub bucket: String,
    pub key: String,
    pub version_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetResp {
    pub body_base64: String,
    pub content_type: String,
    pub etag: String,
    pub last_modified: DateTime<Utc>,
    pub size: u64,
}

pub async fn handle(state: &AppState, req: GetReq) -> Result<GetResp, String> {
    if req.key.is_empty() {
        return Err(err_to_str(StorageError::ConfigError {
            message: "key must not be empty".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let resp = backend
        .get(BackendGetReq {
            key: req.key.clone(),
            version_id: req.version_id,
            // Cap is enforced inside the backend BEFORE the body is buffered,
            // so a multi-GiB object is rejected with ObjectTooLarge instead of
            // amplifying memory usage. See review CRITICAL #2.
            max_inline_bytes: Some(INLINE_BODY_CAP),
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
    Ok(GetResp {
        body_base64: B64.encode(&resp.body),
        content_type: resp.content_type,
        etag: resp.etag,
        last_modified: resp.last_modified,
        size: resp.size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::backend::{Backend, BackendError, GetResp as BackendGetResp};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state_with(
        resp: Option<Result<BackendGetResp, BackendError>>,
    ) -> (AppState, Arc<MockBackend>) {
        let m = Arc::new(MockBackend::default());
        m.responses.lock().unwrap().get = resp;
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

    fn req(v: serde_json::Value) -> GetReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn happy_path_encodes_body_to_base64() {
        let (st, _m) = state_with(Some(Ok(BackendGetResp {
            body: b"hello".to_vec(),
            content_type: "text/plain".into(),
            etag: "\"deadbeef\"".into(),
            last_modified: Utc::now(),
            size: 5,
        })));
        let resp = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap();
        assert_eq!(B64.decode(&resp.body_base64).unwrap(), b"hello");
        assert_eq!(resp.size, 5);
    }

    #[tokio::test]
    async fn not_found_maps_to_object_not_found() {
        let (st, _m) = state_with(Some(Err(BackendError::NotFound)));
        let err = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap_err();
        assert!(err.contains("OBJECT_NOT_FOUND"), "got: {err}");
    }

    #[tokio::test]
    async fn object_too_large_errors() {
        let (st, _m) = state_with(Some(Ok(BackendGetResp {
            body: vec![0u8; (INLINE_BODY_CAP + 1) as usize],
            content_type: "application/octet-stream".into(),
            etag: "x".into(),
            last_modified: Utc::now(),
            size: INLINE_BODY_CAP + 1,
        })));
        let err = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap_err();
        assert!(err.contains("OBJECT_TOO_LARGE"), "got: {err}");
    }

    #[tokio::test]
    async fn unknown_bucket_errors() {
        let (st, _m) = state_with(None);
        let err = super::handle(&st, req(json!({"bucket":"missing","key":"k"})))
            .await
            .unwrap_err();
        assert!(err.contains("UNKNOWN_BUCKET"), "got: {err}");
    }
}
