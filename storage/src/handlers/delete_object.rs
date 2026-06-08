//! `storage::deleteObject` — idempotent delete.

use super::{err_to_str, AppState};
use crate::backend::DeleteReq as BackendDeleteReq;
use crate::error::{backend_error_to_storage, StorageError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteReq {
    pub bucket: String,
    pub key: String,
    pub version_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteResp {
    pub deleted: bool,
}

pub async fn handle(state: &AppState, req: DeleteReq) -> Result<DeleteResp, String> {
    if req.key.is_empty() {
        return Err(err_to_str(StorageError::ConfigError {
            message: "key must not be empty".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let r = backend
        .delete(BackendDeleteReq {
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
    Ok(DeleteResp { deleted: r.deleted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::backend::{Backend, BackendError, DeleteResp as BackendDeleteResp};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state_with(resp: Option<Result<BackendDeleteResp, BackendError>>) -> AppState {
        let m = Arc::new(MockBackend::default());
        m.responses.lock().unwrap().delete = resp;
        let mut map = HashMap::new();
        map.insert("uploads".to_string(), m as Arc<dyn Backend>);
        AppState {
            backends: Arc::new(RwLock::new(map)),
            local_ctx: None,
        }
    }

    fn req(v: serde_json::Value) -> DeleteReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn deleted_true_when_object_existed() {
        let st = state_with(Some(Ok(BackendDeleteResp { deleted: true })));
        let r = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap();
        assert!(r.deleted);
    }

    #[tokio::test]
    async fn deleted_false_when_object_missing() {
        let st = state_with(Some(Ok(BackendDeleteResp { deleted: false })));
        let r = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap();
        assert!(!r.deleted);
    }

    #[tokio::test]
    async fn unknown_bucket_errors() {
        let st = state_with(None);
        let err = super::handle(&st, req(json!({"bucket":"missing","key":"k"})))
            .await
            .unwrap_err();
        assert!(err.contains("UNKNOWN_BUCKET"), "got: {err}");
    }
}
