//! `storage::getObject` — read an object, or a byte range of one, inline.

use super::{err_to_str, AppState, INLINE_BODY_CAP};
use crate::backend::{ByteRange, GetReq as BackendGetReq};
use crate::error::{backend_error_to_storage, StorageError};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
/// Read a small object, or a byte range of any object, inline. The bytes
/// returned are buffered and base64-encoded, with a 10 MiB decoded hard limit
/// on what one call returns. Use a signed GET from `storage::presignUrl` for
/// files when the client can reach the provider; read large objects through
/// this function in ranges (`offset` + `length`) when it cannot.
pub struct GetReq {
    pub bucket: String,
    pub key: String,
    pub version_id: Option<String>,
    /// First byte to return. Default 0. Past the end of the object →
    /// `INVALID_RANGE`; equal to its size → an empty body.
    #[serde(default)]
    pub offset: Option<u64>,
    /// How many bytes to return at most (clamped to the end of the object).
    /// Default: to the end.
    #[serde(default)]
    pub length: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetResp {
    pub body_base64: String,
    pub content_type: String,
    pub etag: String,
    pub last_modified: DateTime<Utc>,
    /// The object's total size.
    pub size: u64,
    /// Where the returned bytes start in the object.
    pub offset: u64,
    /// How many bytes were returned.
    pub length: u64,
}

pub async fn handle(state: &AppState, req: GetReq) -> Result<GetResp, String> {
    if req.key.is_empty() {
        return Err(err_to_str(StorageError::ConfigError {
            message: "key must not be empty".into(),
        }));
    }
    let range = (req.offset.is_some() || req.length.is_some()).then(|| ByteRange {
        offset: req.offset.unwrap_or(0),
        length: req.length,
    });
    let backend = state.backend(&req.bucket).await.map_err(err_to_str)?;
    let resp = backend
        .get(BackendGetReq {
            key: req.key.clone(),
            version_id: req.version_id,
            // Cap is enforced inside the backend BEFORE the body is buffered,
            // so a multi-GiB read is rejected with ObjectTooLarge instead of
            // amplifying memory usage. See review CRITICAL #2. With a range
            // it caps the bytes returned, not the object.
            max_inline_bytes: Some(INLINE_BODY_CAP),
            range,
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
        offset: resp.offset,
        length: resp.body.len() as u64,
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

    fn state_with(
        resp: Option<Result<BackendGetResp, BackendError>>,
    ) -> (AppState, Arc<MockBackend>) {
        let m = Arc::new(MockBackend::default());
        m.responses.lock().unwrap().get = resp;
        let mut map = HashMap::new();
        map.insert("uploads".to_string(), m.clone() as Arc<dyn Backend>);
        (AppState::new(map), m)
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
            offset: 0,
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
            offset: 0,
        })));
        let err = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap_err();
        assert!(err.contains("OBJECT_TOO_LARGE"), "got: {err}");
    }

    fn ten_bytes() -> Option<Result<BackendGetResp, BackendError>> {
        Some(Ok(BackendGetResp {
            body: b"0123456789".to_vec(),
            content_type: "text/plain".into(),
            etag: "e".into(),
            last_modified: Utc::now(),
            size: 10,
            offset: 0,
        }))
    }

    #[tokio::test]
    async fn ranged_reads_return_the_span_its_offset_and_length() {
        let (st, _m) = state_with(ten_bytes());
        let resp = super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"k","offset":3,"length":4})),
        )
        .await
        .unwrap();
        assert_eq!(B64.decode(&resp.body_base64).unwrap(), b"3456");
        assert_eq!((resp.offset, resp.length, resp.size), (3, 4, 10));
        // Clamped to the end; to the end without a length.
        let resp = super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"k","offset":8,"length":100})),
        )
        .await
        .unwrap();
        assert_eq!(B64.decode(&resp.body_base64).unwrap(), b"89");
        let resp = super::handle(&st, req(json!({"bucket":"uploads","key":"k","offset":7})))
            .await
            .unwrap();
        assert_eq!((resp.offset, resp.length), (7, 3));
        // At the end: empty; past it: INVALID_RANGE.
        let resp = super::handle(&st, req(json!({"bucket":"uploads","key":"k","offset":10})))
            .await
            .unwrap();
        assert_eq!((resp.length, resp.size), (0, 10));
        let err = super::handle(&st, req(json!({"bucket":"uploads","key":"k","offset":11})))
            .await
            .unwrap_err();
        assert!(
            err.contains("INVALID_RANGE") && err.contains("\"size\":10"),
            "{err}"
        );
        // A whole read reports offset 0 and the full length.
        let resp = super::handle(&st, req(json!({"bucket":"uploads","key":"k"})))
            .await
            .unwrap();
        assert_eq!((resp.offset, resp.length), (0, 10));
    }

    #[tokio::test]
    async fn the_cap_applies_to_the_returned_length_not_the_object() {
        let big = INLINE_BODY_CAP * 3;
        let (st, _m) = state_with(Some(Ok(BackendGetResp {
            body: vec![7u8; big as usize],
            content_type: "application/octet-stream".into(),
            etag: "x".into(),
            last_modified: Utc::now(),
            size: big,
            offset: 0,
        })));
        let resp = super::handle(
            &st,
            req(json!({"bucket":"uploads","key":"k","offset":INLINE_BODY_CAP,"length":2097152})),
        )
        .await
        .unwrap();
        assert_eq!(resp.length, 2_097_152);
        assert_eq!(resp.size, big);
        let err = super::handle(&st, req(json!({"bucket":"uploads","key":"k","offset":1})))
            .await
            .unwrap_err();
        assert!(err.contains("OBJECT_TOO_LARGE"), "{err}");
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
