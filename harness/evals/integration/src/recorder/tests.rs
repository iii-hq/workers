use std::path::Path;

use serde_json::json;

use super::service::strip_engine_fields;
use super::store::EventStore;
use crate::types::recorder::RecorderEventKind;

#[test]
fn event_store_persists_ordered_events_and_resets_durably() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("recorder.log.jsonl");
    let store = EventStore::new(log_path.clone());

    assert_eq!(store.reset("run-1").expect("reset"), 1);
    assert_eq!(
        store
            .append(
                RecorderEventKind::TargetCall,
                "run-1::target",
                json!({"x": 1})
            )
            .expect("first append"),
        1
    );
    assert_eq!(
        store
            .append(
                RecorderEventKind::Lifecycle,
                "integration-recorder::lifecycle",
                json!({"session_id": "session-1"}),
            )
            .expect("second append"),
        2
    );

    let snapshot = store.snapshot(None).expect("snapshot");
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].sequence, 1);
    assert_eq!(snapshot[1].sequence, 2);
    assert_eq!(
        store.snapshot(Some(1)).expect("filtered snapshot"),
        vec![snapshot[1].clone()]
    );

    let persisted: Vec<serde_json::Value> = std::fs::read_to_string(&log_path)
        .expect("read log")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event JSON"))
        .collect();
    assert_eq!(
        persisted,
        snapshot
            .iter()
            .map(|event| serde_json::to_value(event).expect("event value"))
            .collect::<Vec<_>>()
    );

    assert_eq!(store.reset("run-2").expect("second reset"), 1);
    assert!(store.snapshot(None).expect("empty snapshot").is_empty());
    assert_eq!(std::fs::read(&log_path).expect("read reset log"), b"");
    assert_eq!(
        store
            .append(RecorderEventKind::TargetCall, "run-2::target", json!({}))
            .expect("append after reset"),
        1
    );
}

#[test]
fn failed_open_is_returned_without_acknowledging_the_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let parent = temp.path().join("missing");
    let store = EventStore::new(parent.join("recorder.log.jsonl"));
    store.configure("run-1").expect("configure");

    let error = store
        .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
        .expect_err("open must fail");
    assert!(
        format!("{error:#}").contains("open recorder log for append"),
        "{error:#}"
    );
    assert!(store.snapshot(None).expect("snapshot").is_empty());

    std::fs::create_dir(&parent).expect("create log parent");
    assert_eq!(
        store
            .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
            .expect("retry append"),
        1,
        "a failed append must not advance the acknowledged sequence"
    );
}

#[test]
fn events_are_rejected_until_the_store_is_configured() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = EventStore::new(temp.path().join("recorder.log.jsonl"));

    let error = store
        .append(RecorderEventKind::Lifecycle, "lifecycle", json!({}))
        .expect_err("unconfigured append must fail");
    assert!(
        format!("{error:#}").contains("event store is not configured"),
        "{error:#}"
    );
    assert!(store.snapshot(None).expect("snapshot").is_empty());
}

#[test]
fn engine_fields_are_only_removed_from_the_payload_top_level() {
    let payload = json!({
        "_request_id": "internal",
        "value": "kept",
        "nested": {
            "_provider": "also-kept"
        }
    });
    assert_eq!(
        strip_engine_fields(payload),
        json!({
            "value": "kept",
            "nested": {
                "_provider": "also-kept"
            }
        })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn write_errors_are_returned_without_acknowledging_the_event() {
    let store = EventStore::new(Path::new("/dev/full").to_path_buf());
    store.configure("run-1").expect("configure");

    let error = store
        .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
        .expect_err("/dev/full must reject the write");
    assert!(
        format!("{error:#}").contains("write recorder log"),
        "{error:#}"
    );
    assert!(store.snapshot(None).expect("snapshot").is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn fsync_errors_are_returned_without_acknowledging_the_event() {
    let store = EventStore::new(Path::new("/dev/null").to_path_buf());
    store.configure("run-1").expect("configure");

    let error = store
        .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
        .expect_err("/dev/null cannot be fsynced");
    assert!(
        format!("{error:#}").contains("fsync recorder log"),
        "{error:#}"
    );
    assert!(store.snapshot(None).expect("snapshot").is_empty());
}
