//! Shared credential type used by provider workers.

use serde::{Deserialize, Serialize};

/// Stored credential for a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey {
        key: String,
    },
    OAuth {
        access_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at: Option<i64>,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        provider_extra: serde_json::Value,
    },
}
