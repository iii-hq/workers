//! Shared ABI for the sandbox::* worker family.
//!
//! Every sandbox worker (sandbox, sandbox-e2b, sandbox-daytona, sandbox-vercel,
//! sandbox-morph, sandbox-modal, sandbox-cloudflare) speaks this contract.
//! The `sandbox` worker owns the caller-facing `sandbox::*` ids and dispatches
//! by the `provider` field to `sandbox::provider::<name>::*`. Adapters register
//! only the provider-namespaced ids.

use serde::{Deserialize, Serialize};

pub const ABI_VERSION: &str = "v0";

pub const DEFAULT_PROVIDER: &str = "local";

pub mod ids {
    //! Canonical function ids. Use these constants, not raw strings.

    pub const CREATE: &str = "sandbox::create";
    pub const EXEC: &str = "sandbox::exec";
    pub const STOP: &str = "sandbox::stop";
    pub const LIST: &str = "sandbox::list";
    pub const SNAPSHOT: &str = "sandbox::snapshot";
    pub const EXPOSE_PORT: &str = "sandbox::expose_port";
    pub const BRANCH: &str = "sandbox::branch";
    pub const FS_READ: &str = "sandbox::fs::read";
    pub const FS_WRITE: &str = "sandbox::fs::write";

    pub fn provider(name: &str, leaf: &str) -> String {
        format!("sandbox::provider::{name}::{leaf}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Snapshot,
    Branch,
    ExposePort,
    Fs,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Branch => "branch",
            Self::ExposePort => "expose_port",
            Self::Fs => "fs",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRequest {
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u64>,
    /// Optional. Router dispatches by this field. Adapters ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResponse {
    pub sandbox_id: String,
    pub image: String,
    pub started_at: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub sandbox_id: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopRequest {
    pub sandbox_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub sandboxes: Vec<serde_json::Value>,
    pub in_flight: usize,
    pub cap: usize,
    pub remaining: usize,
    pub reconciled: bool,
}

/// Stable S-codes shared across the family. Workers emit
/// `IIIError::Handler("[Sxxx] ...")`; callers pattern-match the leading
/// bracketed token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SCode {
    ImageNotAllowed,
    ResourceOversize,
    HostCannotBoot,
    ConcurrencyCapReached,
    CapabilityUnsupported,
    RateLimited,
    QuotaExhausted,
    ProviderUnavailable,
    AuthInvalid,
    UnknownProvider,
}

impl SCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImageNotAllowed => "S100",
            Self::ResourceOversize => "S200",
            Self::HostCannotBoot => "S300",
            Self::ConcurrencyCapReached => "S400",
            Self::CapabilityUnsupported => "S404",
            Self::RateLimited => "S500",
            Self::QuotaExhausted => "S501",
            Self::ProviderUnavailable => "S502",
            Self::AuthInvalid => "S503",
            Self::UnknownProvider => "S600",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AbiError {
    #[error("[{}] image not in allowlist: {0}", SCode::ImageNotAllowed.as_str())]
    ImageNotAllowed(String),
    #[error("[{}] resource oversize: {0}", SCode::ResourceOversize.as_str())]
    ResourceOversize(String),
    #[error("[{}] host cannot boot: {0}", SCode::HostCannotBoot.as_str())]
    HostCannotBoot(String),
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
    #[error("[{}] unknown provider: {0}", SCode::UnknownProvider.as_str())]
    UnknownProvider(String),
    #[error("bad input: {0}")]
    BadInput(String),
}

impl AbiError {
    pub fn code(&self) -> SCode {
        match self {
            Self::ImageNotAllowed(_) => SCode::ImageNotAllowed,
            Self::ResourceOversize(_) => SCode::ResourceOversize,
            Self::HostCannotBoot(_) => SCode::HostCannotBoot,
            Self::ConcurrencyCapReached(_) => SCode::ConcurrencyCapReached,
            Self::CapabilityUnsupported(_) => SCode::CapabilityUnsupported,
            Self::RateLimited => SCode::RateLimited,
            Self::QuotaExhausted => SCode::QuotaExhausted,
            Self::ProviderUnavailable(_) => SCode::ProviderUnavailable,
            Self::AuthInvalid => SCode::AuthInvalid,
            Self::UnknownProvider(_) => SCode::UnknownProvider,
            Self::BadInput(_) => SCode::ProviderUnavailable,
        }
    }
}

/// Map an HTTP status from a provider REST API onto the canonical S-code.
pub fn map_http_status(status: u16, body: &str) -> AbiError {
    match status {
        401 | 403 => AbiError::AuthInvalid,
        402 => AbiError::QuotaExhausted,
        429 => AbiError::RateLimited,
        500..=599 => AbiError::ProviderUnavailable(format!("status {status}: {body}")),
        _ => AbiError::ProviderUnavailable(format!("unexpected status {status}: {body}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_shape() {
        assert_eq!(ids::provider("e2b", "create"), "sandbox::provider::e2b::create");
        assert_eq!(ids::provider("morph", "branch"), "sandbox::provider::morph::branch");
    }

    #[test]
    fn s_codes_stable() {
        assert_eq!(SCode::ImageNotAllowed.as_str(), "S100");
        assert_eq!(SCode::ConcurrencyCapReached.as_str(), "S400");
        assert_eq!(SCode::AuthInvalid.as_str(), "S503");
        assert_eq!(SCode::UnknownProvider.as_str(), "S600");
    }

    #[test]
    fn http_status_maps_to_scode() {
        assert!(matches!(map_http_status(401, ""), AbiError::AuthInvalid));
        assert!(matches!(map_http_status(429, ""), AbiError::RateLimited));
        assert!(matches!(map_http_status(503, ""), AbiError::ProviderUnavailable(_)));
    }
}
