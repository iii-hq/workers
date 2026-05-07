//! Library entry for `sandbox-morph`. Holds shared types, error codes, and the
//! capability set advertised by `sandbox::morph::create`.

pub mod client;
pub mod config;
pub mod handler;

use serde::{Deserialize, Serialize};

/// Capability tags advertised by `sandbox::morph::create`. Callers inspect this
/// list to decide whether optional functions like `branch` are available.
pub const CAPABILITIES: &[&str] = &["branch", "snapshot", "expose_port"];

/// Stable error code space shared with every sandbox provider worker in this
/// repo. `S100`-`S400` mirror the libkrun-backed `iii-sandbox` daemon; `S404`
/// and `S500`-`S503` are REST-specific extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SCode {
    ImageNotAllowed,
    ConcurrencyCapReached,
    CapabilityUnsupported,
    RateLimited,
    QuotaExhausted,
    ProviderUnavailable,
    AuthInvalid,
}

impl SCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImageNotAllowed => "S100",
            Self::ConcurrencyCapReached => "S400",
            Self::CapabilityUnsupported => "S404",
            Self::RateLimited => "S500",
            Self::QuotaExhausted => "S501",
            Self::ProviderUnavailable => "S502",
            Self::AuthInvalid => "S503",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("[{}] image not in allowlist: {0}", SCode::ImageNotAllowed.as_str())]
    ImageNotAllowed(String),
    #[error("[{}] concurrency cap reached ({0} active)", SCode::ConcurrencyCapReached.as_str())]
    ConcurrencyCapReached(usize),
    #[error("[{}] capability unsupported: {0}", SCode::CapabilityUnsupported.as_str())]
    CapabilityUnsupported(String),
    #[error("[{}] rate limited by provider", SCode::RateLimited.as_str())]
    RateLimited,
    #[error("[{}] quota exhausted", SCode::QuotaExhausted.as_str())]
    QuotaExhausted,
    #[error("[{}] provider unavailable: {0}", SCode::ProviderUnavailable.as_str())]
    ProviderUnavailable(String),
    #[error("[{}] auth invalid or expired", SCode::AuthInvalid.as_str())]
    AuthInvalid,
    #[error("bad input: {0}")]
    BadInput(String),
}

impl WorkerError {
    pub fn code(&self) -> SCode {
        match self {
            Self::ImageNotAllowed(_) => SCode::ImageNotAllowed,
            Self::ConcurrencyCapReached(_) => SCode::ConcurrencyCapReached,
            Self::CapabilityUnsupported(_) => SCode::CapabilityUnsupported,
            Self::RateLimited => SCode::RateLimited,
            Self::QuotaExhausted => SCode::QuotaExhausted,
            Self::ProviderUnavailable(_) => SCode::ProviderUnavailable,
            Self::AuthInvalid => SCode::AuthInvalid,
            Self::BadInput(_) => SCode::ProviderUnavailable,
        }
    }
}

/// Convert a non-2xx HTTP status from Morph into the right S-code.
pub fn map_http_status(status: u16, body: &str) -> WorkerError {
    match status {
        401 | 403 => WorkerError::AuthInvalid,
        402 => WorkerError::QuotaExhausted,
        429 => WorkerError::RateLimited,
        500..=599 => WorkerError::ProviderUnavailable(format!("status {status}: {body}")),
        _ => WorkerError::ProviderUnavailable(format!("unexpected status {status}: {body}")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRecord {
    pub sandbox_id: String,
    pub image: String,
    /// RFC3339 timestamp from the upstream provider, passed through
    /// untouched. Callers parse if they need a numeric form. We avoid
    /// pulling a date crate just to convert to epoch seconds.
    pub started_at: String,
}
