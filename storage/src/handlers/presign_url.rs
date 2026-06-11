//! `storage::presignUrl` — short-lived URL for browser direct upload/download.

use super::{err_to_str, AppState};
use crate::backend::{PresignMethod, PresignReq as BackendPresignReq};
use crate::error::{backend_error_to_storage, StorageError};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MIN_EXPIRY: u64 = 30;
const MAX_EXPIRY: u64 = 86400;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PresignReq {
    pub bucket: String,
    pub key: String,
    pub method: HttpMethod,
    pub content_type: Option<String>,
    #[serde(default = "default_expires")]
    pub expires_in_seconds: u64,
}

#[derive(Debug, Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Put,
}

fn default_expires() -> u64 {
    600
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PresignResp {
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn handle(state: &AppState, req: PresignReq) -> Result<PresignResp, String> {
    if req.key.is_empty() {
        return Err(err_to_str(StorageError::ConfigError {
            message: "key must not be empty".into(),
        }));
    }
    if req.expires_in_seconds < MIN_EXPIRY || req.expires_in_seconds > MAX_EXPIRY {
        return Err(err_to_str(StorageError::InvalidPresignParams {
            reason: format!(
                "expires_in_seconds must be in [{MIN_EXPIRY},{MAX_EXPIRY}]; got {}",
                req.expires_in_seconds
            ),
        }));
    }
    let method = match req.method {
        HttpMethod::Get => PresignMethod::Get,
        HttpMethod::Put => PresignMethod::Put,
    };
    if matches!(method, PresignMethod::Put) && req.content_type.is_none() {
        return Err(err_to_str(StorageError::InvalidPresignParams {
            reason: "content_type is required for PUT presigns to prevent type smuggling".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let resp = backend
        .presign(BackendPresignReq {
            key: req.key.clone(),
            method,
            content_type: req.content_type,
            expires_in_seconds: req.expires_in_seconds,
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
    Ok(PresignResp {
        url: resp.url,
        expires_at: resp.expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::backend::Backend;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state() -> AppState {
        let m = Arc::new(MockBackend::default());
        let mut map = HashMap::new();
        map.insert("uploads".to_string(), m as Arc<dyn Backend>);
        AppState {
            backends: Arc::new(RwLock::new(map)),
            local_ctx: None,
        }
    }

    fn req(v: serde_json::Value) -> PresignReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn presign_get_succeeds() {
        let st = state();
        let r = super::handle(
            &st,
            req(json!({
                "bucket":"uploads","key":"k","method":"GET","expires_in_seconds":600
            })),
        )
        .await
        .unwrap();
        assert!(r.url.starts_with("https://"));
    }

    #[tokio::test]
    async fn presign_put_requires_content_type() {
        let st = state();
        let err = super::handle(
            &st,
            req(json!({
                "bucket":"uploads","key":"k","method":"PUT","expires_in_seconds":600
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PRESIGN_PARAMS"), "got: {err}");
        assert!(err.contains("content_type"), "got: {err}");
    }

    #[tokio::test]
    async fn presign_expiry_below_30_errors() {
        let st = state();
        let err = super::handle(
            &st,
            req(json!({
                "bucket":"uploads","key":"k","method":"GET","expires_in_seconds":10
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PRESIGN_PARAMS"), "got: {err}");
    }

    #[tokio::test]
    async fn presign_expiry_above_86400_errors() {
        let st = state();
        let err = super::handle(
            &st,
            req(json!({
                "bucket":"uploads","key":"k","method":"GET","expires_in_seconds":100000
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PRESIGN_PARAMS"), "got: {err}");
    }
}
