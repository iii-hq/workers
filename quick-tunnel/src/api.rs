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
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct StatusResponse {
    #[serde(flatten)]
    pub snapshot: Snapshot,
    pub leases: Vec<Lease>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedConfig {
    /// Omit to subscribe to all authorized targets.
    pub tunnel_id: Option<String>,
}
