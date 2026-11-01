//! `storage::on-object-created`: the event an object write produces.

use super::normalize::ObjectEventNormalized;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ON_OBJECT_CREATED_FN: &str = "storage::on-object-created";

pub const ON_OBJECT_CREATED_DESCRIPTION: &str = "Called once per object written to a configured bucket whose notifications source is wired up; returns the event { bucket, key, size, etag, content_type, created_at, raw_event_id }. Observers pass args { bucket, handler_timeout_ms? } (default 60000); each target receives the event and acks it by returning null or { ack: true }. The caller gets { acked }: true when every matching target acked, and the notifications source acks the upstream message only then. Only the storage worker calls it.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectCreatedEvent {
    pub bucket: String,
    pub key: String,
    pub size: u64,
    pub etag: String,
    pub content_type: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub raw_event_id: String,
}

impl From<ObjectEventNormalized> for ObjectCreatedEvent {
    fn from(n: ObjectEventNormalized) -> Self {
        Self {
            bucket: n.bucket,
            key: n.key,
            size: n.size,
            etag: n.etag,
            content_type: n.content_type,
            created_at: n.created_at.unwrap_or_else(chrono::Utc::now),
            raw_event_id: n.raw_event_id,
        }
    }
}
