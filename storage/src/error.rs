//! Discriminated, stable error codes returned to the engine. Mirrors
//! `database/src/error.rs`. The `code` field is the contract; the rest
//! is diagnostic.

use crate::backend::BackendError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "code")]
pub enum StorageError {
    #[serde(rename = "CONFIG_ERROR")]
    #[error("config error: {message}")]
    ConfigError { message: String },

    #[serde(rename = "UNKNOWN_BUCKET")]
    #[error("unknown bucket {bucket}")]
    UnknownBucket { bucket: String },

    #[serde(rename = "OBJECT_NOT_FOUND")]
    #[error("object not found: {bucket}/{key}")]
    ObjectNotFound { bucket: String, key: String },

    #[serde(rename = "BODY_TOO_LARGE")]
    #[error("body exceeds 10 MiB cap; use presignUrl for large objects")]
    BodyTooLarge { size: u64, cap: u64 },

    #[serde(rename = "OBJECT_TOO_LARGE")]
    #[error("object exceeds 10 MiB cap; use presignUrl for large objects")]
    ObjectTooLarge { size: u64, cap: u64 },

    #[serde(rename = "INVALID_BASE64")]
    #[error("invalid base64 payload: {reason}")]
    InvalidBase64 { reason: String },

    #[serde(rename = "INVALID_PRESIGN_PARAMS")]
    #[error("invalid presign params: {reason}")]
    InvalidPresignParams { reason: String },

    #[serde(rename = "PRESIGN_UNSUPPORTED")]
    #[error("presign unsupported on this backend: {reason}")]
    PresignUnsupported { reason: String },

    #[serde(rename = "LOCAL_BACKEND_DOWN")]
    #[error("local rustfs backend is down: {reason}")]
    LocalBackendDown { reason: String },

    #[serde(rename = "LOCAL_BACKEND_BIN_NOT_FOUND")]
    #[error("rustfs binary not found: {reason}")]
    LocalBackendBinNotFound { reason: String },

    #[serde(rename = "LOCAL_BACKEND_BOOT_FAILED")]
    #[error("rustfs boot failed: {reason}")]
    LocalBackendBootFailed { reason: String },

    #[serde(rename = "PROVIDER_ERROR")]
    #[error("provider {provider} error: {message}")]
    ProviderError {
        provider: String,
        #[serde(rename = "inner_code")]
        inner_code: Option<String>,
        message: String,
    },

    #[serde(rename = "PROVIDER_AUTH_FAILED")]
    #[error("provider {provider} auth failed: {message}")]
    ProviderAuthFailed { provider: String, message: String },

    #[serde(rename = "CF_QUEUE_AUTH_FAILED")]
    #[error("cloudflare queue auth failed: {message}")]
    CfQueueAuthFailed { message: String },

    #[serde(rename = "TRIGGER_DISPATCH_TIMEOUT")]
    #[error("trigger handler exceeded {timeout_ms}ms")]
    TriggerDispatchTimeout { timeout_ms: u64 },
}

impl StorageError {
    /// Convert into the JSON string body that handlers return as their `Err`.
    /// Mirrors `database`'s `err_to_str`.
    pub fn to_wire_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                "{{\"code\":\"PROVIDER_ERROR\",\"message\":{:?}}}",
                self.to_string()
            )
        })
    }
}

impl From<StorageError> for iii_sdk::IIIError {
    fn from(e: StorageError) -> Self {
        iii_sdk::IIIError::Handler(e.to_wire_string())
    }
}

/// Map a `BackendError` (provider-agnostic) into a `StorageError` (wire-stable).
/// The `provider` and `bucket`/`key` come from the call site since `BackendError`
/// alone doesn't know which provider was invoked.
pub fn backend_error_to_storage(
    err: BackendError,
    provider: &str,
    bucket: &str,
    key: &str,
) -> StorageError {
    match err {
        BackendError::NotFound => StorageError::ObjectNotFound {
            bucket: bucket.to_string(),
            key: key.to_string(),
        },
        BackendError::AuthFailed(message) => StorageError::ProviderAuthFailed {
            provider: provider.to_string(),
            message,
        },
        BackendError::PresignUnsupported(reason) => StorageError::PresignUnsupported { reason },
        BackendError::LocalBackendDown(reason) => StorageError::LocalBackendDown { reason },
        BackendError::ObjectTooLarge { actual_size, cap } => StorageError::ObjectTooLarge {
            size: actual_size,
            cap,
        },
        BackendError::Provider {
            inner_code,
            message,
        } => StorageError::ProviderError {
            provider: provider.to_string(),
            inner_code,
            message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn body_too_large_serializes_with_stable_code() {
        let e = StorageError::BodyTooLarge {
            size: 20_000_000,
            cap: 10 * 1024 * 1024,
        };
        let v: Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "BODY_TOO_LARGE");
        assert_eq!(v["size"], 20_000_000);
    }

    #[test]
    fn object_not_found_includes_bucket_and_key() {
        let e = StorageError::ObjectNotFound {
            bucket: "uploads".into(),
            key: "u/1/profile.jpg".into(),
        };
        let v: Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "OBJECT_NOT_FOUND");
        assert_eq!(v["bucket"], "uploads");
        assert_eq!(v["key"], "u/1/profile.jpg");
    }

    #[test]
    fn provider_error_includes_inner_code() {
        let e = StorageError::ProviderError {
            provider: "s3".into(),
            inner_code: Some("PermanentRedirect".into()),
            message: "wrong region".into(),
        };
        let v: Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "PROVIDER_ERROR");
        assert_eq!(v["provider"], "s3");
        assert_eq!(v["inner_code"], "PermanentRedirect");
    }

    #[test]
    fn backend_not_found_maps_to_object_not_found() {
        let s = backend_error_to_storage(BackendError::NotFound, "s3", "uploads", "k");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["code"], "OBJECT_NOT_FOUND");
    }

    #[test]
    fn backend_auth_failed_maps_to_provider_auth_failed() {
        let s =
            backend_error_to_storage(BackendError::AuthFailed("expired".into()), "s3", "u", "k");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["code"], "PROVIDER_AUTH_FAILED");
        assert_eq!(v["provider"], "s3");
    }

    #[test]
    fn into_iii_error_carries_wire_body() {
        let e = StorageError::UnknownBucket { bucket: "x".into() };
        let iii_e: iii_sdk::IIIError = e.into();
        let body = format!("{iii_e:?}");
        assert!(body.contains("UNKNOWN_BUCKET"));
    }
}
