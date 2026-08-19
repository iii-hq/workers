//! `storage::presignPost` — signed multipart/form-data browser upload.

use super::{err_to_str, AppState};
use crate::backend::PresignPostReq as BackendPresignPostReq;
use crate::error::{backend_error_to_storage, StorageError};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MIN_EXPIRY: u64 = 30;
const MAX_EXPIRY: u64 = 86_400;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PresignPostReq {
    pub bucket: String,
    pub key: String,
    pub content_type: String,
    #[serde(default = "default_expires")]
    pub expires_in_seconds: u64,
    /// Optional upload cap enforced while the HTTP body is streamed.
    pub max_size_bytes: Option<u64>,
}

fn default_expires() -> u64 {
    600
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PresignPostResp {
    pub url: String,
    pub fields: HashMap<String, String>,
    pub expires_at: DateTime<Utc>,
}

pub async fn handle(state: &AppState, req: PresignPostReq) -> Result<PresignPostResp, String> {
    if req.key.is_empty() || req.content_type.is_empty() {
        return Err(err_to_str(StorageError::InvalidPresignParams {
            reason: "key and content_type must not be empty".into(),
        }));
    }
    if !(MIN_EXPIRY..=MAX_EXPIRY).contains(&req.expires_in_seconds) {
        return Err(err_to_str(StorageError::InvalidPresignParams {
            reason: format!(
                "expires_in_seconds must be in [{MIN_EXPIRY},{MAX_EXPIRY}]; got {}",
                req.expires_in_seconds
            ),
        }));
    }
    if req.max_size_bytes == Some(0) {
        return Err(err_to_str(StorageError::InvalidPresignParams {
            reason: "max_size_bytes must be greater than zero".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let response = backend
        .presign_post(BackendPresignPostReq {
            key: req.key.clone(),
            content_type: req.content_type,
            expires_in_seconds: req.expires_in_seconds,
            max_size_bytes: req.max_size_bytes,
        })
        .await
        .map_err(|error| {
            err_to_str(backend_error_to_storage(
                error,
                backend.provider(),
                &req.bucket,
                &req.key,
            ))
        })?;
    Ok(PresignPostResp {
        url: response.url,
        fields: response.fields,
        expires_at: response.expires_at,
    })
}
