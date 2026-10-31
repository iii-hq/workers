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
pub struct ListedPart {
    pub part_number: u32,
    pub size: u64,
    pub etag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListResp {
    pub parts: Vec<ListedPart>,
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
fn listed_part(p: &ListedPart) -> bool {
    (1..=10000).contains(&p.part_number)
        && p.size > 0
        && text(&p.etag)
        && p.checksum_sha256.as_deref().is_none_or(valid_checksum)
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
    if result.parts.iter().any(|p| !listed_part(p))
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
/// Identifies the wire semantics, not a package version or a provider certification.
pub const CONTRACT: &str = "sha256-composite-v2-list-optional";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitiesReq {
    pub bucket: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CapabilitiesResp {
    pub provider: &'static str,
    pub contract: Option<&'static str>,
}

/// Only reads the live configured backend; no objects, uploads or credentials are returned.
pub async fn capabilities(
    state: &AppState,
    req: CapabilitiesReq,
) -> Result<CapabilitiesResp, String> {
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let provider = backend.provider();
    Ok(CapabilitiesResp {
        provider,
        contract: matches!(provider, "s3" | "r2").then_some(CONTRACT),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn multipart_capabilities_are_bucket_scoped_and_metadata_only() {
        use crate::backend::{
            mock::MockBackend,
            s3::{S3Backend, S3BackendOpts, S3StaticCreds},
            Backend,
        };
        use std::{collections::HashMap, sync::Arc};
        for provider in ["s3", "r2"] {
            let backend = S3Backend::build(
                S3BackendOpts {
                    region: "us-east-1",
                    credentials: Some(S3StaticCreds {
                        access_key_id: "test".into(),
                        secret_access_key: "test".into(),
                        session_token: None,
                    }),
                    endpoint_url: Some("http://127.0.0.1:1".into()),
                    force_path_style: true,
                    provider_tag: provider,
                    fast_fail: true,
                },
                "private".into(),
            )
            .await
            .unwrap();
            let state = AppState::new(HashMap::from([(
                "files".into(),
                Arc::new(backend) as Arc<dyn Backend>,
            )]));
            let response = capabilities(
                &state,
                CapabilitiesReq {
                    bucket: "files".into(),
                },
            )
            .await
            .unwrap();
            let json = serde_json::to_value(response).unwrap();
            assert_eq!(
                json,
                serde_json::json!({ "contract": "sha256-composite-v2-list-optional", "provider": provider })
            );
            assert!(capabilities(
                &state,
                CapabilitiesReq {
                    bucket: "missing".into()
                }
            )
            .await
            .unwrap_err()
            .contains("UNKNOWN_BUCKET"));
        }
        let mock = Arc::new(MockBackend::default());
        let state = AppState::new(HashMap::from([(
            "files".into(),
            mock.clone() as Arc<dyn Backend>,
        )]));
        let response = capabilities(
            &state,
            CapabilitiesReq {
                bucket: "files".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(response.contract, None);
        assert!(mock.calls.lock().unwrap().is_empty());
        assert!(super::super::parse_request::<CapabilitiesReq>(
            serde_json::json!({"bucket":"files", "key":"forbidden"})
        )
        .is_err());
    }

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
    #[test]
    fn multipart_complete_requires_checksum_on_each_part() {
        let request = serde_json::json!({
            "bucket": "bucket", "key": "key", "upload_id": "id",
            "parts": [{ "part_number": 1, "size": 1, "etag": "opaque" }]
        });
        assert!(serde_json::from_value::<CompleteReq>(request).is_err());
        let p = Part {
            part_number: 1,
            size: 1,
            etag: "opaque".into(),
            checksum_sha256: "".into(),
        };
        assert!(!part(&p));
    }
    #[test]
    fn multipart_listing_allows_absence_but_validates_present_evidence() {
        let valid = ListedPart {
            part_number: 1,
            size: 1,
            etag: "opaque".into(),
            checksum_sha256: None,
        };
        assert!(listed_part(&valid));
        assert!(listed_part(&ListedPart {
            checksum_sha256: Some(STANDARD.encode([0; 32])),
            ..valid.clone()
        }));
        for invalid in [
            ListedPart {
                part_number: 0,
                ..valid.clone()
            },
            ListedPart {
                part_number: 10001,
                ..valid.clone()
            },
            ListedPart {
                size: 0,
                ..valid.clone()
            },
            ListedPart {
                etag: "".into(),
                ..valid.clone()
            },
            ListedPart {
                etag: "bad\n".into(),
                ..valid.clone()
            },
            ListedPart {
                checksum_sha256: Some("bad".into()),
                ..valid
            },
        ] {
            assert!(!listed_part(&invalid));
        }
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
