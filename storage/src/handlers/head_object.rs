//! `storage::headObject` — fetch object metadata without downloading the body.

use super::{err_to_str, AppState};
use crate::backend::HeadReq as BackendHeadReq;
use crate::error::{backend_error_to_storage, StorageError};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HeadReq {
    pub bucket: String,
    pub key: String,
    pub version_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HeadResp {
    pub content_type: String,
    pub etag: String,
    pub last_modified: DateTime<Utc>,
    pub size: u64,
}

pub async fn handle(state: &AppState, req: HeadReq) -> Result<HeadResp, String> {
    if req.key.is_empty() {
        return Err(err_to_str(StorageError::ConfigError {
            message: "key must not be empty".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let resp = backend
        .head(BackendHeadReq {
            key: req.key.clone(),
            version_id: req.version_id,
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
    Ok(HeadResp {
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
    use crate::backend::{Backend, BackendError, HeadResp as BackendHeadResp};
    use chrono::Utc;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state_with(
        resp: Option<Result<BackendHeadResp, BackendError>>,
    ) -> (AppState, Arc<MockBackend>) {
        let m = Arc::new(MockBackend::default());
        m.responses.lock().unwrap().head = resp;
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

    fn req(v: serde_json::Value) -> HeadReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn happy_path_returns_metadata() {
        let now = Utc::now();
        let (st, _m) = state_with(Some(Ok(BackendHeadResp {
            content_type: "image/png".into(),
            etag: "\"abc123\"".into(),
            last_modified: now,
            size: 42,
        })));
        let resp = super::handle(&st, req(json!({"bucket":"uploads","key":"img.png"})))
            .await
            .unwrap();
        assert_eq!(resp.content_type, "image/png");
        assert_eq!(resp.etag, "\"abc123\"");
        assert_eq!(resp.size, 42);
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
    async fn unknown_bucket_errors() {
        let (st, _m) = state_with(None);
        let err = super::handle(&st, req(json!({"bucket":"missing","key":"k"})))
            .await
            .unwrap_err();
        assert!(err.contains("UNKNOWN_BUCKET"), "got: {err}");
    }

    #[tokio::test]
    async fn empty_key_errors() {
        let (st, _m) = state_with(None);
        let err = super::handle(&st, req(json!({"bucket": "uploads", "key": ""})))
            .await
            .unwrap_err();
        assert!(
            err.contains("CONFIG_ERROR") || err.contains("must not be empty"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn version_id_is_threaded_to_backend() {
        let (st, mock) = state_with(Some(Ok(BackendHeadResp {
            content_type: "application/octet-stream".into(),
            etag: String::new(),
            last_modified: Utc::now(),
            size: 1,
        })));
        super::handle(
            &st,
            req(json!({"bucket": "uploads", "key": "k", "version_id": "v2"})),
        )
        .await
        .unwrap();
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        if let crate::backend::mock::MockCall::Head(h) = &calls[0] {
            assert_eq!(h.version_id.as_deref(), Some("v2"));
        } else {
            panic!("expected Head call, got: {:?}", calls[0]);
        }
    }
}
