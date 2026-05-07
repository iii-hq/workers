//! Map per-provider event JSON to the unified envelope dispatched to handlers.

use chrono::{DateTime, Utc};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

fn decode_s3_key(raw: &str) -> String {
    // S3 event notifications use application/x-www-form-urlencoded semantics
    // for the key: spaces become `+`, other special chars are %XX. We must
    // replace `+` first, BEFORE percent_decode_str runs, otherwise a literal
    // `+` in a percent-encoded form (e.g. `%2B`) would be decoded to `+` and
    // we'd incorrectly turn it into a space afterward.
    let plus_decoded = raw.replace('+', " ");
    percent_decode_str(&plus_decoded)
        .decode_utf8_lossy()
        .into_owned()
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub enum EventKind {
    Created,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectEventNormalized {
    pub bucket: String,
    pub key: String,
    pub size: u64,
    pub etag: String,
    pub content_type: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub event_kind: EventKind,
    pub raw_event_id: String,
}

#[derive(Debug, Error)]
pub enum NormalizeError {
    #[error("ignored event type: {0}")]
    Ignored(String),
    #[error("unmapped underlying bucket: {0}")]
    UnmappedBucket(String),
    #[error("malformed event: {0}")]
    Malformed(String),
}

pub fn normalize_s3(
    event: &Value,
    bucket_reverse: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<ObjectEventNormalized>, NormalizeError> {
    let records = event
        .get("Records")
        .and_then(|v| v.as_array())
        .ok_or_else(|| NormalizeError::Malformed("missing Records".into()))?;
    let mut out = Vec::new();
    for r in records {
        let event_name = r
            .get("eventName")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NormalizeError::Malformed("missing eventName".into()))?;
        let kind = if event_name.starts_with("ObjectCreated:") {
            EventKind::Created
        } else if event_name.starts_with("ObjectRemoved:") {
            EventKind::Deleted
        } else {
            continue;
        };
        let underlying = r
            .pointer("/s3/bucket/name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NormalizeError::Malformed("missing bucket name".into()))?;
        let bucket = bucket_reverse(underlying)
            .ok_or_else(|| NormalizeError::UnmappedBucket(underlying.to_string()))?;
        let key = r
            .pointer("/s3/object/key")
            .and_then(|v| v.as_str())
            .map(decode_s3_key)
            .unwrap_or_default();
        let size = r
            .pointer("/s3/object/size")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let etag = r
            .pointer("/s3/object/eTag")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let event_time_str = r.get("eventTime").and_then(|v| v.as_str()).unwrap_or("");
        let event_time = DateTime::parse_from_rfc3339(event_time_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
        let raw_event_id = r
            .pointer("/responseElements/x-amz-request-id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push(ObjectEventNormalized {
            bucket,
            key,
            size,
            etag,
            content_type: None,
            created_at: matches!(kind, EventKind::Created)
                .then_some(event_time)
                .flatten(),
            deleted_at: matches!(kind, EventKind::Deleted)
                .then_some(event_time)
                .flatten(),
            event_kind: kind,
            raw_event_id,
        });
    }
    Ok(out)
}

pub fn normalize_gcs(
    event: &Value,
    bucket_reverse: &dyn Fn(&str) -> Option<String>,
) -> Result<ObjectEventNormalized, NormalizeError> {
    let event_type = event
        .pointer("/attributes/eventType")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NormalizeError::Malformed("missing eventType".into()))?;
    let kind = match event_type {
        "OBJECT_FINALIZE" => EventKind::Created,
        "OBJECT_DELETE" | "OBJECT_ARCHIVE" => EventKind::Deleted,
        other => return Err(NormalizeError::Ignored(other.to_string())),
    };
    let underlying = event
        .pointer("/attributes/bucketId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NormalizeError::Malformed("missing bucketId".into()))?;
    let bucket = bucket_reverse(underlying)
        .ok_or_else(|| NormalizeError::UnmappedBucket(underlying.to_string()))?;
    let key = event
        .pointer("/attributes/objectId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let size = event
        .pointer("/data/size")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let etag = event
        .pointer("/data/etag")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let content_type = event
        .pointer("/data/contentType")
        .and_then(|v| v.as_str())
        .map(String::from);
    let now = Utc::now();
    let raw_event_id = event
        .get("messageId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(ObjectEventNormalized {
        bucket,
        key,
        size,
        etag,
        content_type,
        created_at: matches!(kind, EventKind::Created).then_some(now),
        deleted_at: matches!(kind, EventKind::Deleted).then_some(now),
        event_kind: kind,
        raw_event_id,
    })
}

pub fn normalize_r2(
    event: &Value,
    bucket_reverse: &dyn Fn(&str) -> Option<String>,
) -> Result<ObjectEventNormalized, NormalizeError> {
    let action = event
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NormalizeError::Malformed("missing action".into()))?;
    let kind = match action {
        "PutObject" | "CompleteMultipartUpload" | "CopyObject" => EventKind::Created,
        "DeleteObject" | "DeleteObjects" => EventKind::Deleted,
        other => return Err(NormalizeError::Ignored(other.to_string())),
    };
    let underlying = event
        .get("bucket")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NormalizeError::Malformed("missing bucket".into()))?;
    let bucket = bucket_reverse(underlying)
        .ok_or_else(|| NormalizeError::UnmappedBucket(underlying.to_string()))?;
    let key = event
        .get("object")
        .and_then(|v| v.get("key"))
        .and_then(|v| v.as_str())
        .map(decode_s3_key)
        .unwrap_or_default();
    let size = event
        .get("object")
        .and_then(|v| v.get("size"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let etag = event
        .get("object")
        .and_then(|v| v.get("eTag"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now = Utc::now();
    let raw_event_id = event
        .get("messageId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(ObjectEventNormalized {
        bucket,
        key,
        size,
        etag,
        content_type: None,
        created_at: matches!(kind, EventKind::Created).then_some(now),
        deleted_at: matches!(kind, EventKind::Deleted).then_some(now),
        event_kind: kind,
        raw_event_id,
    })
}

pub fn normalize_rustfs(
    event: &Value,
    bucket_reverse: &dyn Fn(&str) -> Option<String>,
) -> Result<ObjectEventNormalized, NormalizeError> {
    let event_name = event
        .get("EventName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NormalizeError::Malformed("missing EventName".into()))?;
    let kind = if event_name.contains("ObjectCreated") {
        EventKind::Created
    } else if event_name.contains("ObjectRemoved") {
        EventKind::Deleted
    } else {
        return Err(NormalizeError::Ignored(event_name.to_string()));
    };
    let underlying = event
        .pointer("/Records/0/s3/bucket/name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NormalizeError::Malformed("missing bucket".into()))?;
    let bucket = bucket_reverse(underlying)
        .ok_or_else(|| NormalizeError::UnmappedBucket(underlying.to_string()))?;
    let key = event
        .pointer("/Records/0/s3/object/key")
        .and_then(|v| v.as_str())
        .map(decode_s3_key)
        .unwrap_or_default();
    let size = event
        .pointer("/Records/0/s3/object/size")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let etag = event
        .pointer("/Records/0/s3/object/eTag")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let now = Utc::now();
    let raw_event_id = format!(
        "{}:{}:{}",
        bucket,
        key,
        now.timestamp_nanos_opt().unwrap_or(0)
    );
    Ok(ObjectEventNormalized {
        bucket,
        key,
        size,
        etag,
        content_type: None,
        created_at: matches!(kind, EventKind::Created).then_some(now),
        deleted_at: matches!(kind, EventKind::Deleted).then_some(now),
        event_kind: kind,
        raw_event_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!(
            "{}/tests/fixtures/events/{}.json",
            env!("CARGO_MANIFEST_DIR"),
            name
        );
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn s3_object_created_normalizes() {
        let ev = fixture("s3_object_created");
        let n = normalize_s3(&ev, &|under| {
            (under == "my-app-uploads").then(|| "uploads".to_string())
        })
        .unwrap();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].bucket, "uploads");
        assert_eq!(n[0].key, "u/1/profile.jpg");
        assert_eq!(n[0].size, 1024);
        assert!(matches!(n[0].event_kind, EventKind::Created));
        assert!(n[0].created_at.is_some());
        assert!(n[0].deleted_at.is_none());
    }

    #[test]
    fn s3_object_removed_normalizes() {
        let ev = fixture("s3_object_removed");
        let n = normalize_s3(&ev, &|under| {
            (under == "my-app-uploads").then(|| "uploads".to_string())
        })
        .unwrap();
        assert_eq!(n.len(), 1);
        assert!(matches!(n[0].event_kind, EventKind::Deleted));
        assert!(n[0].deleted_at.is_some());
    }

    #[test]
    fn gcs_finalize_normalizes() {
        let ev = fixture("gcs_object_finalize");
        let n = normalize_gcs(&ev, &|under| {
            (under == "my-app-documents").then(|| "documents".to_string())
        })
        .unwrap();
        assert_eq!(n.bucket, "documents");
        assert_eq!(n.size, 204800);
        assert!(matches!(n.event_kind, EventKind::Created));
        assert_eq!(n.content_type.as_deref(), Some("application/pdf"));
    }

    #[test]
    fn gcs_delete_normalizes() {
        let ev = fixture("gcs_object_delete");
        let n = normalize_gcs(&ev, &|under| {
            (under == "my-app-documents").then(|| "documents".to_string())
        })
        .unwrap();
        assert!(matches!(n.event_kind, EventKind::Deleted));
    }

    #[test]
    fn r2_put_normalizes() {
        let ev = fixture("r2_put_object");
        let n = normalize_r2(&ev, &|under| {
            (under == "avatars").then(|| "avatars".to_string())
        })
        .unwrap();
        assert!(matches!(n.event_kind, EventKind::Created));
        assert_eq!(n.size, 51200);
    }

    #[test]
    fn r2_delete_normalizes() {
        let ev = fixture("r2_delete_object");
        let n = normalize_r2(&ev, &|under| {
            (under == "avatars").then(|| "avatars".to_string())
        })
        .unwrap();
        assert!(matches!(n.event_kind, EventKind::Deleted));
    }

    #[test]
    fn rustfs_created_normalizes() {
        let ev = fixture("rustfs_created");
        let n = normalize_rustfs(&ev, &|under| {
            (under == "scratch").then(|| "scratch".to_string())
        })
        .unwrap();
        assert!(matches!(n.event_kind, EventKind::Created));
        assert_eq!(n.size, 13);
    }

    #[test]
    fn rustfs_deleted_normalizes() {
        let ev = fixture("rustfs_deleted");
        let n = normalize_rustfs(&ev, &|under| {
            (under == "scratch").then(|| "scratch".to_string())
        })
        .unwrap();
        assert!(matches!(n.event_kind, EventKind::Deleted));
    }

    #[test]
    fn unmapped_bucket_returns_error_for_s3() {
        let ev = fixture("s3_object_created");
        match normalize_s3(&ev, &|_| None) {
            Err(NormalizeError::UnmappedBucket(_)) => {}
            other => panic!("expected UnmappedBucket, got {other:?}"),
        }
    }

    #[test]
    fn malformed_s3_event_returns_error() {
        let ev: Value = serde_json::json!({"NotRecords": []});
        let r = normalize_s3(&ev, &|_| None);
        assert!(matches!(r, Err(NormalizeError::Malformed(_))));
    }

    /// Regression: rustfs (and S3 / R2) percent-encode object keys in their
    /// event payloads. Without decoding, trigger handlers receive keys like
    /// `harness%2Ftriggers%2Ffoo` that never match the user-side `harness/triggers/foo`.
    #[test]
    fn rustfs_percent_encoded_key_is_decoded() {
        let ev: Value = serde_json::json!({
            "EventName": "s3:ObjectCreated:Put",
            "Records": [{
                "s3": {
                    "bucket": { "name": "scratch" },
                    "object": {
                        "key": "harness%2Ftriggers%2Fcreated-target",
                        "size": 7,
                        "eTag": "abc"
                    }
                }
            }]
        });
        let n = normalize_rustfs(&ev, &|under| {
            (under == "scratch").then(|| "scratch".to_string())
        })
        .unwrap();
        assert_eq!(n.key, "harness/triggers/created-target");
    }

    #[test]
    fn s3_percent_encoded_key_is_decoded() {
        let ev: Value = serde_json::json!({
            "Records": [{
                "eventName": "ObjectCreated:Put",
                "eventTime": "2024-01-01T00:00:00Z",
                "s3": {
                    "bucket": { "name": "my-app-uploads" },
                    "object": {
                        "key": "u/1/has%20space.jpg",
                        "size": 10,
                        "eTag": "x"
                    }
                },
                "responseElements": { "x-amz-request-id": "req-1" }
            }]
        });
        let n = normalize_s3(&ev, &|under| {
            (under == "my-app-uploads").then(|| "uploads".to_string())
        })
        .unwrap();
        assert_eq!(n[0].key, "u/1/has space.jpg");
    }

    #[test]
    fn r2_percent_encoded_key_is_decoded() {
        let ev: Value = serde_json::json!({
            "action": "PutObject",
            "bucket": "avatars",
            "object": { "key": "users%2F42%2Favatar.png", "size": 5, "eTag": "y" }
        });
        let n = normalize_r2(&ev, &|under| {
            (under == "avatars").then(|| "avatars".to_string())
        })
        .unwrap();
        assert_eq!(n.key, "users/42/avatar.png");
    }

    #[test]
    fn ignored_gcs_event_type_returns_error() {
        let ev: Value = serde_json::json!({
            "attributes": {
                "eventType": "OBJECT_METADATA_UPDATE",
                "bucketId": "x"
            }
        });
        let r = normalize_gcs(&ev, &|_| Some("x".into()));
        assert!(matches!(r, Err(NormalizeError::Ignored(_))));
    }
}
