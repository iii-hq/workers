//! Metadata-only multipart control operations.
use super::{err_to_str, validate_key, AppState};
use crate::backend::BackendError;
use crate::error::{backend_error_to_storage, StorageError};
use base64::{engine::general_purpose::STANDARD, Engine};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReq {
    pub bucket: String,
    pub key: String,
    pub content_type: String,
    pub metadata: HashMap<String, String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreateResp {
    pub upload_id: String,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub bucket: String,
    pub key: String,
    pub upload_id: String,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresignReq {
    pub bucket: String,
    pub key: String,
    pub upload_id: String,
    pub part_number: u32,
    pub size: u64,
    pub checksum_sha256: String,
    pub expires_in_seconds: u64,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PresignResp {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub part_number: u32,
    pub size: u64,
    pub etag: String,
    pub checksum_sha256: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListResp {
    pub parts: Vec<Part>,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteReq {
    pub bucket: String,
    pub key: String,
    pub upload_id: String,
    pub parts: Vec<Part>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompleteResp {
    pub etag: String,
    pub checksum_sha256: String,
    pub checksum_type: String,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AbortResp {
    pub aborted: bool,
}

fn invalid(reason: &str) -> String {
    err_to_str(StorageError::ConfigError {
        message: reason.into(),
    })
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}
fn valid_checksum(value: &str) -> bool {
    STANDARD
        .decode(value)
        .is_ok_and(|v| v.len() == 32 && STANDARD.encode(v) == value)
}
fn identity(key: &str, upload_id: &str) -> Result<(), String> {
    validate_key(key).map_err(err_to_str)?;
    if !text(upload_id) {
        return Err(invalid("invalid upload_id"));
    }
    Ok(())
}
fn part(p: &Part) -> bool {
    (1..=10000).contains(&p.part_number)
        && p.size > 0
        && text(&p.etag)
        && valid_checksum(&p.checksum_sha256)
}
fn map_error(e: BackendError, provider: &str, bucket: &str, key: &str) -> String {
    err_to_str(backend_error_to_storage(e, provider, bucket, key))
}

pub async fn create(state: &AppState, req: CreateReq) -> Result<CreateResp, String> {
    validate_key(&req.key).map_err(err_to_str)?;
    if !text(&req.content_type)
        || req
            .metadata
            .iter()
            .any(|(k, v)| !text(k) || v.chars().any(char::is_control))
    {
        return Err(invalid("invalid content type or metadata"));
    }
    let b = state.backend(&req.bucket).await.map_err(err_to_str)?;
    b.multipart_create(&req)
        .await
        .map_err(|e| map_error(e, b.provider(), &req.bucket, &req.key))
}
pub async fn presign(state: &AppState, req: PresignReq) -> Result<PresignResp, String> {
    identity(&req.key, &req.upload_id)?;
    if !(1..=10000).contains(&req.part_number)
        || req.size == 0
        || !valid_checksum(&req.checksum_sha256)
        || !(1..=600).contains(&req.expires_in_seconds)
    {
        return Err(invalid("invalid part or expiry"));
    }
    let b = state.backend(&req.bucket).await.map_err(err_to_str)?;
    b.multipart_presign(&req)
        .await
        .map_err(|e| map_error(e, b.provider(), &req.bucket, &req.key))
}
pub async fn list(state: &AppState, req: Identity) -> Result<ListResp, String> {
    identity(&req.key, &req.upload_id)?;
    let b = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let result = b
        .multipart_list(&req)
        .await
        .map_err(|e| map_error(e, b.provider(), &req.bucket, &req.key))?;
    if result.parts.iter().any(|p| !part(p))
        || result
            .parts
            .windows(2)
            .any(|p| p[0].part_number >= p[1].part_number)
    {
        return Err(invalid("provider returned invalid parts"));
    }
    Ok(result)
}
pub async fn complete(state: &AppState, req: CompleteReq) -> Result<CompleteResp, String> {
    identity(&req.key, &req.upload_id)?;
    if req.parts.is_empty()
        || req
            .parts
            .iter()
            .enumerate()
            .any(|(i, p)| !part(p) || p.part_number as usize != i + 1)
    {
        return Err(invalid("invalid multipart manifest"));
    }
    let b = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let result = b
        .multipart_complete(&req)
        .await
        .map_err(|e| map_error(e, b.provider(), &req.bucket, &req.key))?;
    let (digest, count) = result
        .checksum_sha256
        .rsplit_once('-')
        .ok_or_else(|| invalid("invalid composite checksum"))?;
    if !text(&result.etag)
        || result.checksum_type != "COMPOSITE"
        || !valid_checksum(digest)
        || count.parse::<usize>().ok() != Some(req.parts.len())
    {
        return Err(invalid("invalid composite integrity"));
    }
    Ok(result)
}
pub async fn abort(state: &AppState, req: Identity) -> Result<AbortResp, String> {
    identity(&req.key, &req.upload_id)?;
    let b = state.backend(&req.bucket).await.map_err(err_to_str)?;
    b.multipart_abort(&req)
        .await
        .map_err(|e| map_error(e, b.provider(), &req.bucket, &req.key))?;
    Ok(AbortResp { aborted: true })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multipart_validation_rejects_invalid_checksum() {
        assert!(!valid_checksum("bad"));
        assert!(valid_checksum(&STANDARD.encode([0; 32])));
    }
    #[test]
    fn multipart_validation_checks_part() {
        let p = Part {
            part_number: 10001,
            size: 1,
            etag: "\"opaque\"".into(),
            checksum_sha256: STANDARD.encode([0; 32]),
        };
        assert!(!part(&p));
    }
    #[tokio::test]
    async fn multipart_rejects_invalid_input_before_backend() {
        let state = AppState::new(HashMap::new());
        let valid = STANDARD.encode([0; 32]);
        for (key, id, number, sum, expiry) in [
            ("", "id", 1, valid.as_str(), 60),
            ("key", "", 1, valid.as_str(), 60),
            ("key", "id", 0, valid.as_str(), 60),
            ("key", "id", 10001, valid.as_str(), 60),
            ("key", "id", 1, "bad", 60),
            ("key", "id", 1, valid.as_str(), 0),
            ("key", "id", 1, valid.as_str(), 601),
        ] {
            let error = presign(
                &state,
                PresignReq {
                    bucket: "unknown".into(),
                    key: key.into(),
                    upload_id: id.into(),
                    part_number: number,
                    size: 1,
                    checksum_sha256: sum.into(),
                    expires_in_seconds: expiry,
                },
            )
            .await
            .unwrap_err();
            assert!(
                !error.contains("UNKNOWN_BUCKET"),
                "validation did not run first: {error}"
            );
        }
    }
}
