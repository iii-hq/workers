//! Trigger registration for `storage::object-deleted`.

use super::normalize::ObjectEventNormalized;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TriggerConfig {
    pub bucket: String,
    #[serde(default = "default_handler_timeout")]
    pub handler_timeout_ms: u64,
}

fn default_handler_timeout() -> u64 {
    60_000
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Event {
    pub bucket: String,
    pub key: String,
    pub deleted_at: chrono::DateTime<chrono::Utc>,
    pub raw_event_id: String,
}

impl From<ObjectEventNormalized> for Event {
    fn from(n: ObjectEventNormalized) -> Self {
        Self {
            bucket: n.bucket,
            key: n.key,
            deleted_at: n.deleted_at.unwrap_or_else(chrono::Utc::now),
            raw_event_id: n.raw_event_id,
        }
    }
}
