//! `storage::listObjects` — paginated object/prefix listing.

use super::{err_to_str, AppState};
use crate::backend::ListReq as BackendListReq;
use crate::error::{backend_error_to_storage, StorageError};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const DEFAULT_LIMIT: u32 = 250;
const MAX_LIMIT: u32 = 1_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListObjectsReq {
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    /// Directory separator. Use `/` for the bucket/folder explorer; omit it
    /// for a flat recursive listing.
    pub delimiter: Option<String>,
    pub cursor: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    DEFAULT_LIMIT
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObjectSummary {
    pub key: String,
    pub etag: String,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
    pub content_type: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListObjectsResp {
    pub objects: Vec<ObjectSummary>,
    pub common_prefixes: Vec<String>,
    pub next_cursor: Option<String>,
}

pub async fn handle(state: &AppState, req: ListObjectsReq) -> Result<ListObjectsResp, String> {
    if req.limit == 0 || req.limit > MAX_LIMIT {
        return Err(err_to_str(StorageError::ConfigError {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        }));
    }
    if req.delimiter.as_deref().is_some_and(str::is_empty) {
        return Err(err_to_str(StorageError::ConfigError {
            message: "delimiter must not be empty".into(),
        }));
    }
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let response = backend
        .list(BackendListReq {
            prefix: req.prefix,
            delimiter: req.delimiter,
            cursor: req.cursor,
            limit: req.limit,
        })
        .await
        .map_err(|error| {
            err_to_str(backend_error_to_storage(
                error,
                backend.provider(),
                &req.bucket,
                "",
            ))
        })?;
    Ok(ListObjectsResp {
        objects: response
            .objects
            .into_iter()
            .map(|object| ObjectSummary {
                key: object.key,
                etag: object.etag,
                size: object.size,
                last_modified: object.last_modified,
                content_type: object.content_type,
            })
            .collect(),
        common_prefixes: response.common_prefixes,
        next_cursor: response.next_cursor,
    })
}
