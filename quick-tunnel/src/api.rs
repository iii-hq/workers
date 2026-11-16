use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn default_tunnel() -> String {
    "webhooks".to_owned()
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcquireRequest {
    /// Stable opaque consumer identity; not a credential.
    pub consumer_id: String,
    #[serde(default = "default_tunnel")]
    pub tunnel_id: String,
    /// Absolute lease expiry in RFC3339, strictly in the future.
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRequest {
    pub lease_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ReleaseResponse {
    pub released: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusRequest {
    #[serde(default = "default_tunnel")]
    pub tunnel_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    Ready,
    Reconnecting,
    Failed,
    Stopped,
}

/// A generation is opaque. Compare equality, never lexical ordering.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Snapshot {
    pub tunnel_id: String,
    pub status: Status,
    pub public_url: Option<String>,
    pub generation: String,
    /// Sanitized lifecycle failure, never raw cloudflared output.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct AcquireResponse {
    pub lease_id: String,
    #[serde(flatten)]
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub lease_id: String,
    pub consumer_id: String,
    pub tunnel_id: String,
    pub expires_at: DateTime<Utc>,
    /// The PROXY mode the lease was granted under; a boot with another mode drops it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_protocol: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct StatusResponse {
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub leases: Vec<Lease>,
    /// Omitted for cloudflared, so its JSON is what older versions sent.
    #[serde(default, skip_serializing_if = "is_cloudflared")]
    pub provider: crate::config::ProviderKind,
    /// Whether the operator-provided cloudflared is usable, and where to get it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<crate::prerequisites::Prerequisites>,
}

fn is_cloudflared(p: &crate::config::ProviderKind) -> bool {
    *p == crate::config::ProviderKind::Cloudflared
}
