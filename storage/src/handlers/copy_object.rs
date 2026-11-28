//! `storage::copyObject` — copy an object from one configured bucket (or
//! key) to another.
//!
//! When both buckets are reachable with one set of credentials (S3 or R2
//! buckets on the same endpoint and access key, or two `local` buckets) the
//! provider copies the object itself and the bytes never pass through the
//! worker, whatever their size. Any other pair is copied through the worker,
//! buffered, up to [`COPY_THROUGH_WORKER_CAP`].

use super::{AppState, err_to_str, validate_key};
use crate::backend::{
    BackendError, CopyReq as BackendCopyReq, GetReq, HeadReq, PutReq as BackendPutReq,
};
use crate::error::{StorageError, backend_error_to_storage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Largest object copied through the worker, when the provider cannot copy
/// between the two buckets itself. The object is held in memory.
pub const COPY_THROUGH_WORKER_CAP: u64 = 64 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
/// Copy `source_bucket`/`source_key` to `bucket`/`key`. Without
/// `content_type`, `cache_control` and `metadata`, the copy keeps the
/// source's; with any of them, those three are replaced (an unset content
/// type or metadata keeps the source's, an unset cache control is dropped).
pub struct CopyReq {
    pub source_bucket: String,
    pub source_key: String,
    pub bucket: String,
    pub key: String,
    pub content_type: Option<String>,
    pub cache_control: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CopyResp {
    pub etag: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    /// True when the provider copied the object itself; false when the
    /// bytes went through the worker.
    pub server_side: bool,
}

pub async fn handle(state: &AppState, req: CopyReq) -> Result<CopyResp, String> {
    validate_key(&req.source_key).map_err(err_to_str)?;
    validate_key(&req.key).map_err(err_to_str)?;
    if req.source_bucket == req.bucket && req.source_key == req.key {
        return Err(err_to_str(StorageError::ConfigError {
            message: "the source and the destination are the same object".into(),
        }));
    }
    let source = state
        .backend(&req.source_bucket)
        .await
        .map_err(err_to_str)?;
    let target = state.backend(&req.bucket).await.map_err(err_to_str)?;
    // A missing object is the source; any other failure is reported against
    // the destination, where the write happens.
    let fail = |error: BackendError, provider: &str| {
        let (bucket, key) = match error {
            BackendError::NotFound => (&req.source_bucket, &req.source_key),
            _ => (&req.bucket, &req.key),
        };
        err_to_str(backend_error_to_storage(error, provider, bucket, key))
    };
    let copy = BackendCopyReq {
        source_key: req.source_key.clone(),
        key: req.key.clone(),
        content_type: req.content_type.clone(),
        cache_control: req.cache_control.clone(),
        metadata: req.metadata.clone(),
    };
    if let Some(done) = target
        .copy_from(source.as_ref(), &copy)
        .await
        .map_err(|error| fail(error, target.provider()))?
    {
        return Ok(CopyResp {
            etag: done.etag,
            size: done.size,
            version_id: done.version_id,
            server_side: true,
        });
    }

    let read = source
        .get(GetReq {
            key: req.source_key.clone(),
            max_inline_bytes: Some(COPY_THROUGH_WORKER_CAP),
            ..Default::default()
        })
        .await
        .map_err(|error| fail(error, source.provider()))?;
    let metadata = match copy.metadata {
        Some(metadata) => metadata,
        None => source
            .head(HeadReq {
                key: req.source_key.clone(),
                ..Default::default()
            })
            .await
            .map_err(|error| fail(error, source.provider()))?
            .metadata
            .unwrap_or_default(),
    };
    let written = target
        .put(BackendPutReq {
            key: req.key.clone(),
            body: read.body,
            content_type: copy.content_type.unwrap_or(read.content_type),
            cache_control: copy.cache_control,
            metadata,
        })
        .await
        .map_err(|error| fail(error, target.provider()))?;
    Ok(CopyResp {
        etag: written.etag,
        size: written.size,
        version_id: written.version_id,
        server_side: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::mock::{MockBackend, MockCall};
    use crate::backend::{GetResp, HeadResp};
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;

    fn object(body: &[u8]) -> GetResp {
        GetResp {
            body: body.to_vec(),
            content_type: "application/gzip".into(),
            etag: "\"src\"".into(),
            last_modified: Utc::now(),
            size: body.len() as u64,
            offset: 0,
        }
    }

    fn head(metadata: Option<HashMap<String, String>>) -> HeadResp {
        HeadResp {
            metadata,
            checksum_sha256: None,
            checksum_type: None,
            content_type: "application/gzip".into(),
            etag: "\"src\"".into(),
            last_modified: Utc::now(),
            size: 5,
        }
    }

    /// Two mock buckets: mocks never copy server-side, so every copy here
    /// goes through the worker.
    fn state() -> (AppState, Arc<MockBackend>, Arc<MockBackend>) {
        let source = Arc::new(MockBackend::default());
        let target = Arc::new(MockBackend::default());
        let backends = HashMap::from([
            ("staging".to_string(), source.clone() as Arc<dyn Backend>),
            ("cdn".to_string(), target.clone() as Arc<dyn Backend>),
        ]);
        (AppState::new(backends), source, target)
    }

    fn req(v: serde_json::Value) -> CopyReq {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn a_pair_without_a_server_copy_goes_through_the_worker() {
        let (st, source, target) = state();
        source.responses.lock().unwrap().get = Some(Ok(object(b"hello")));
        source.responses.lock().unwrap().head = Some(Ok(head(Some(HashMap::from([(
            "sha256".to_string(),
            "abc".to_string(),
        )])))));
        let resp = handle(
            &st,
            req(json!({
                "source_bucket": "staging", "source_key": "staging/1/a.dmg",
                "bucket": "cdn", "key": "workers/a/1/a.dmg",
                "cache_control": "public, max-age=31536000, immutable"
            })),
        )
        .await
        .unwrap();
        assert!(!resp.server_side);
        let calls = target.calls.lock().unwrap();
        match &calls[..] {
            [MockCall::Put(put)] => {
                assert_eq!(put.key, "workers/a/1/a.dmg");
                assert_eq!(put.body, b"hello");
                assert_eq!(put.content_type, "application/gzip");
                assert_eq!(
                    put.cache_control.as_deref(),
                    Some("public, max-age=31536000, immutable")
                );
                assert_eq!(put.metadata.get("sha256").map(String::as_str), Some("abc"));
            }
            other => panic!("expected one put, got {other:?}"),
        }
        match &source.calls.lock().unwrap()[0] {
            MockCall::Get(get) => assert_eq!(get.max_inline_bytes, Some(COPY_THROUGH_WORKER_CAP)),
            other => panic!("expected a capped get, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_stated_content_type_and_metadata_replace_the_sources() {
        let (st, source, target) = state();
        source.responses.lock().unwrap().get = Some(Ok(object(b"hello")));
        handle(
            &st,
            req(json!({
                "source_bucket": "staging", "source_key": "a",
                "bucket": "cdn", "key": "b",
                "content_type": "application/x-apple-diskimage",
                "metadata": {}
            })),
        )
        .await
        .unwrap();
        // With the metadata stated, the source's is never read.
        assert!(
            !source
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|call| matches!(call, MockCall::Head(_)))
        );
        match &target.calls.lock().unwrap()[0] {
            MockCall::Put(put) => {
                assert_eq!(put.content_type, "application/x-apple-diskimage");
                assert!(put.metadata.is_empty());
            }
            other => panic!("expected a put, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn too_large_to_go_through_the_worker_is_refused_before_writing() {
        let (st, source, target) = state();
        let mut big = object(b"x");
        big.size = COPY_THROUGH_WORKER_CAP + 1;
        source.responses.lock().unwrap().get = Some(Ok(big));
        let err = handle(
            &st,
            req(
                json!({"source_bucket": "staging", "source_key": "a", "bucket": "cdn", "key": "b"}),
            ),
        )
        .await
        .unwrap_err();
        assert!(err.contains("OBJECT_TOO_LARGE"), "{err}");
        assert!(target.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_source_names_the_source() {
        let (st, _source, _target) = state();
        let err = handle(
            &st,
            req(json!({"source_bucket": "staging", "source_key": "gone", "bucket": "cdn", "key": "b"})),
        )
        .await
        .unwrap_err();
        assert!(err.contains("OBJECT_NOT_FOUND"), "{err}");
        assert!(err.contains("staging") && err.contains("gone"), "{err}");
    }

    #[tokio::test]
    async fn bad_requests_never_reach_a_backend() {
        for (body, code) in [
            (
                json!({"source_bucket": "staging", "source_key": "a", "bucket": "staging", "key": "a"}),
                "CONFIG_ERROR",
            ),
            (
                json!({"source_bucket": "staging", "source_key": "../x", "bucket": "cdn", "key": "b"}),
                "INVALID_KEY",
            ),
            (
                json!({"source_bucket": "staging", "source_key": "a", "bucket": "cdn", "key": "%2e%2e/x"}),
                "INVALID_KEY",
            ),
            (
                json!({"source_bucket": "nope", "source_key": "a", "bucket": "cdn", "key": "b"}),
                "UNKNOWN_BUCKET",
            ),
        ] {
            let (st, source, target) = state();
            let err = handle(&st, req(body.clone())).await.unwrap_err();
            assert!(err.contains(code), "{body}: {err}");
            assert!(source.calls.lock().unwrap().is_empty(), "{body}");
            assert!(target.calls.lock().unwrap().is_empty(), "{body}");
        }
    }
}
