//! `storage::on-object-deleted`: the event an object removal produces.

use super::normalize::ObjectEventNormalized;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ON_OBJECT_DELETED_FN: &str = "storage::on-object-deleted";

pub const ON_OBJECT_DELETED_DESCRIPTION: &str = "Called once per object removed from a configured bucket whose notifications source is wired up; returns the event { bucket, key, deleted_at, raw_event_id }. Observers pass args { bucket, handler_timeout_ms? } (default 60000); each target receives the event and acks it by returning null or { ack: true }. The caller gets { acked }: true when every matching target acked, and the notifications source acks the upstream message only then. Only the storage worker calls it.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectDeletedEvent {
    pub bucket: String,
    pub key: String,
    pub deleted_at: chrono::DateTime<chrono::Utc>,
    pub raw_event_id: String,
}

impl From<ObjectEventNormalized> for ObjectDeletedEvent {
    fn from(n: ObjectEventNormalized) -> Self {
        Self {
            bucket: n.bucket,
            key: n.key,
            deleted_at: n.deleted_at.unwrap_or_else(chrono::Utc::now),
            raw_event_id: n.raw_event_id,
        }
    }
}
