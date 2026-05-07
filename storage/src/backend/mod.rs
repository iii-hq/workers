//! Provider-agnostic object-storage backend.
//!
//! Every concrete provider (S3, GCS, R2, local) implements `Backend`.
//! Handlers depend only on this trait — provider quirks are isolated to one
//! file each.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use thiserror::Error;

pub mod factory;
pub mod gcs;
pub mod local;
pub mod r2;
pub mod s3;

#[async_trait]
pub trait Backend: Send + Sync {
    async fn put(&self, req: PutReq) -> Result<PutResp, BackendError>;
    async fn get(&self, req: GetReq) -> Result<GetResp, BackendError>;
    async fn delete(&self, req: DeleteReq) -> Result<DeleteResp, BackendError>;
    async fn presign(&self, req: PresignReq) -> Result<PresignResp, BackendError>;

    /// Provider tag used in error envelopes.
    fn provider(&self) -> &'static str;
}

#[derive(Debug, Clone)]
pub struct PutReq {
    pub key: String,
    pub body: Vec<u8>,
    pub content_type: String,
    pub cache_control: Option<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PutResp {
    pub etag: String,
    pub size: u64,
    pub version_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GetReq {
    pub key: String,
    pub version_id: Option<String>,
    /// If set, the backend MUST return `BackendError::ObjectTooLarge` (with the
    /// real size) instead of buffering an object whose advertised content_length
    /// exceeds this cap. Prevents memory amplification when callers only accept
    /// inline-sized objects.
    pub max_inline_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct GetResp {
    pub body: Vec<u8>,
    pub content_type: String,
    pub etag: String,
    pub last_modified: DateTime<Utc>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct DeleteReq {
    pub key: String,
    pub version_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeleteResp {
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresignMethod {
    Get,
    Put,
}

#[derive(Debug, Clone)]
pub struct PresignReq {
    pub key: String,
    pub method: PresignMethod,
    pub content_type: Option<String>,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct PresignResp {
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("object not found")]
    NotFound,

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("presign unsupported: {0}")]
    PresignUnsupported(String),

    #[error("local backend down: {0}")]
    LocalBackendDown(String),

    /// The remote object exists but its size exceeds `max_inline_bytes`. Backends
    /// signal this BEFORE buffering the body, so callers can reject without
    /// holding the bytes in memory.
    #[error("object too large: {actual_size} > {cap}")]
    ObjectTooLarge { actual_size: u64, cap: u64 },

    /// Wraps any other provider-side error. `inner_code` is the SDK's stable
    /// code (e.g. AWS error code, GCS HTTP status, CF API error number).
    #[error("provider error: {message}")]
    Provider {
        inner_code: Option<String>,
        message: String,
    },
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    /// A `Backend` that records calls and serves canned responses. Use in
    /// handler tests so the handler logic is exercised without any real
    /// network or process.
    #[derive(Default)]
    pub struct MockBackend {
        pub calls: Mutex<Vec<MockCall>>,
        pub responses: Mutex<MockResponses>,
    }

    #[derive(Debug, Clone)]
    pub enum MockCall {
        Put(PutReq),
        Get(GetReq),
        Delete(DeleteReq),
        Presign(PresignReq),
    }

    #[derive(Default)]
    pub struct MockResponses {
        pub put: Option<Result<PutResp, BackendError>>,
        pub get: Option<Result<GetResp, BackendError>>,
        pub delete: Option<Result<DeleteResp, BackendError>>,
        pub presign: Option<Result<PresignResp, BackendError>>,
    }

    #[async_trait]
    impl Backend for MockBackend {
        async fn put(&self, req: PutReq) -> Result<PutResp, BackendError> {
            self.calls.lock().unwrap().push(MockCall::Put(req));
            self.responses
                .lock()
                .unwrap()
                .put
                .clone()
                .unwrap_or_else(|| {
                    Ok(PutResp {
                        etag: "\"deadbeef\"".to_string(),
                        size: 0,
                        version_id: None,
                    })
                })
        }

        async fn get(&self, req: GetReq) -> Result<GetResp, BackendError> {
            // Mirror real backends: reject oversized objects BEFORE returning
            // the body so handler tests exercise the same cap path as production.
            let cap = req.max_inline_bytes;
            self.calls.lock().unwrap().push(MockCall::Get(req));
            let resp = self
                .responses
                .lock()
                .unwrap()
                .get
                .clone()
                .unwrap_or_else(|| Err(BackendError::NotFound))?;
            if let Some(c) = cap {
                if resp.size > c {
                    return Err(BackendError::ObjectTooLarge {
                        actual_size: resp.size,
                        cap: c,
                    });
                }
            }
            Ok(resp)
        }

        async fn delete(&self, req: DeleteReq) -> Result<DeleteResp, BackendError> {
            self.calls.lock().unwrap().push(MockCall::Delete(req));
            self.responses
                .lock()
                .unwrap()
                .delete
                .clone()
                .unwrap_or(Ok(DeleteResp { deleted: true }))
        }

        async fn presign(&self, req: PresignReq) -> Result<PresignResp, BackendError> {
            let expires_in = req.expires_in_seconds;
            self.calls.lock().unwrap().push(MockCall::Presign(req));
            self.responses
                .lock()
                .unwrap()
                .presign
                .clone()
                .unwrap_or_else(|| {
                    Ok(PresignResp {
                        url: "https://example.test/presigned".to_string(),
                        expires_at: Utc::now() + chrono::Duration::seconds(expires_in as i64),
                    })
                })
        }

        fn provider(&self) -> &'static str {
            "mock"
        }
    }

    // BackendError doesn't derive Clone (thiserror doesn't), so we add a manual
    // one strictly for the mock — production code never clones it.
    impl Clone for BackendError {
        fn clone(&self) -> Self {
            match self {
                BackendError::NotFound => BackendError::NotFound,
                BackendError::AuthFailed(s) => BackendError::AuthFailed(s.clone()),
                BackendError::PresignUnsupported(s) => BackendError::PresignUnsupported(s.clone()),
                BackendError::LocalBackendDown(s) => BackendError::LocalBackendDown(s.clone()),
                BackendError::ObjectTooLarge { actual_size, cap } => BackendError::ObjectTooLarge {
                    actual_size: *actual_size,
                    cap: *cap,
                },
                BackendError::Provider {
                    inner_code,
                    message,
                } => BackendError::Provider {
                    inner_code: inner_code.clone(),
                    message: message.clone(),
                },
            }
        }
    }
}

#[cfg(test)]
mod backend_tests {
    use super::mock::*;
    use super::*;

    #[tokio::test]
    async fn mock_records_put_call() {
        let m = MockBackend::default();
        m.put(PutReq {
            key: "k".into(),
            body: vec![1, 2, 3],
            content_type: "text/plain".into(),
            cache_control: None,
            metadata: Default::default(),
        })
        .await
        .unwrap();
        let calls = m.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], MockCall::Put(_)));
    }

    #[tokio::test]
    async fn mock_get_returns_not_found_by_default() {
        let m = MockBackend::default();
        let err = m
            .get(GetReq {
                key: "k".into(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::NotFound));
    }
}
