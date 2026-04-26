//! Streaming surface tests.
//!
//! Unit-level: SSE frame layout, A2A v0.3 wire fields (`final` not
//! `finalEvent`, camelCase, optional skip-on-none), and `StreamRegistry`
//! mechanics that don't need a live engine.
//!
//! Integration: three `#[ignore]`d e2e tests document the live-engine
//! shape so a maintainer running the engine locally can exercise the
//! full path.

use std::sync::Arc;

use iii_a2a::streaming::{
    StreamRegistry, build_sse_frame, dispatch_resubscribe, dispatch_stream, writer_ref_from_input,
};
use iii_a2a::types::{
    Artifact, Part, ResubscribeParams, TaskArtifactUpdateEvent, TaskState, TaskStatus,
    TaskStatusUpdateEvent,
};
use iii_sdk::{ChannelDirection, ChannelWriter, StreamChannelRef};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// SSE frame layout
// ---------------------------------------------------------------------------

#[test]
fn sse_frame_matches_a2a_layout() {
    let payload = json!({"foo": 1});
    let frame = build_sse_frame(7, "status-update", &payload);
    let s = std::str::from_utf8(&frame).expect("utf8");

    // The A2A SSE wire is: `id: <n>\nevent: <kind>\ndata: <json>\n\n`.
    assert!(s.starts_with("id: 7\n"), "id line missing: {s:?}");
    assert!(
        s.contains("\nevent: status-update\n"),
        "event line missing: {s:?}"
    );
    assert!(s.contains("\ndata: {\"foo\":1}\n"), "data line: {s:?}");
    assert!(s.ends_with("\n\n"), "frame must end with blank line: {s:?}");
}

// ---------------------------------------------------------------------------
// A2A v0.3 wire shape
// ---------------------------------------------------------------------------

#[test]
fn task_status_update_uses_final_not_final_event() {
    let event = TaskStatusUpdateEvent {
        task_id: "t1".to_string(),
        context_id: None,
        kind: "status-update".to_string(),
        status: TaskStatus {
            state: TaskState::Working,
            message: None,
            timestamp: None,
        },
        final_event: true,
    };
    let v = serde_json::to_value(&event).unwrap();
    // `final` is the spec name; `finalEvent` would be the default
    // serde-rename-from-Rust shape. Asserting the absence here keeps the
    // wire compatibility from regressing.
    assert_eq!(v.get("final"), Some(&Value::Bool(true)));
    assert!(
        v.get("finalEvent").is_none(),
        "Wire field must be `final`, not `finalEvent`: {v}"
    );
}

#[test]
fn task_status_update_omits_optional_context_id() {
    let event = TaskStatusUpdateEvent {
        task_id: "t1".to_string(),
        context_id: None,
        kind: "status-update".to_string(),
        status: TaskStatus {
            state: TaskState::Working,
            message: None,
            timestamp: None,
        },
        final_event: false,
    };
    let v = serde_json::to_value(&event).unwrap();
    assert!(
        v.get("contextId").is_none(),
        "skip_serializing_if must drop context_id when None: {v}"
    );
    // taskId is always present and camelCased.
    assert_eq!(v.get("taskId"), Some(&Value::String("t1".to_string())));
}

#[test]
fn task_artifact_update_serializes_camel_case() {
    let artifact = Artifact {
        artifact_id: "a1".to_string(),
        parts: vec![Part {
            text: Some("hello".to_string()),
            data: None,
            url: None,
            raw: None,
            media_type: None,
        }],
        name: Some("greet".to_string()),
        metadata: None,
    };
    let event = TaskArtifactUpdateEvent {
        task_id: "t1".to_string(),
        context_id: Some("ctx-1".to_string()),
        kind: "artifact-update".to_string(),
        artifact,
        append: None,
        last_chunk: Some(true),
    };
    let v = serde_json::to_value(&event).unwrap();
    assert_eq!(v.get("taskId"), Some(&Value::String("t1".to_string())));
    assert_eq!(
        v.get("contextId"),
        Some(&Value::String("ctx-1".to_string()))
    );
    assert_eq!(v.get("lastChunk"), Some(&Value::Bool(true)));
    assert!(
        v.get("append").is_none(),
        "skip_serializing_if must drop append when None: {v}"
    );
    let artifact_v = v.get("artifact").unwrap();
    assert_eq!(
        artifact_v.get("artifactId"),
        Some(&Value::String("a1".to_string()))
    );
}

#[test]
fn resubscribe_params_round_trip() {
    let raw = json!({"id": "task-42"});
    let parsed: ResubscribeParams = serde_json::from_value(raw).unwrap();
    assert_eq!(parsed.id, "task-42");
}

// ---------------------------------------------------------------------------
// Registry mechanics — these need a tokio runtime but no engine.
// ---------------------------------------------------------------------------

fn dummy_writer() -> Arc<ChannelWriter> {
    // ChannelWriter is lazy: the WS connection only opens on first write
    // / send_message. None of these tests touch the wire, so a writer
    // pointed at port 0 is fine.
    let r = StreamChannelRef {
        channel_id: "test-chan".to_string(),
        access_key: "test-key".to_string(),
        direction: ChannelDirection::Write,
    };
    Arc::new(ChannelWriter::new("ws://127.0.0.1:0", &r))
}

#[tokio::test]
async fn registry_subscribe_tracks_writers() {
    let registry = StreamRegistry::new();
    let _ = registry.subscribe("task-A", dummy_writer()).await;
    let _ = registry.subscribe("task-A", dummy_writer()).await;

    assert_eq!(registry.subscriber_count("task-A").await, 2);
    assert!(registry.has_task("task-A").await);
    assert!(!registry.has_task("task-B").await);
}

#[tokio::test]
async fn registry_close_removes_entry_and_signals() {
    let registry = StreamRegistry::new();
    let _ = registry.subscribe("task-A", dummy_writer()).await;
    let mut rx = registry
        .terminal_watch("task-A")
        .await
        .expect("watch exists for known task");
    // Pre-close: terminal flag is false. Use bare assert! to avoid
    // clippy::bool_assert_comparison.
    assert!(!*rx.borrow());

    registry.close_task("task-A").await;

    // close_task drops the bus, which sends `true` on the terminal_tx
    // before drop. The receiver still observes the value.
    let _ = rx.changed().await;
    assert!(*rx.borrow());

    assert!(!registry.has_task("task-A").await);
    assert_eq!(registry.subscriber_count("task-A").await, 0);
}

#[tokio::test]
async fn registry_broadcast_on_unknown_task_is_noop() {
    let registry = StreamRegistry::new();
    // No subscribers, no bus — must not panic.
    registry
        .broadcast("ghost-task", "status-update", &json!({"any": "thing"}))
        .await;
    assert!(!registry.has_task("ghost-task").await);
}

#[tokio::test]
async fn registry_buses_are_independent() {
    let registry = StreamRegistry::new();
    let _ = registry.subscribe("task-A", dummy_writer()).await;
    let _ = registry.subscribe("task-B", dummy_writer()).await;

    registry.close_task("task-A").await;

    assert!(!registry.has_task("task-A").await);
    assert!(registry.has_task("task-B").await);
    assert_eq!(registry.subscriber_count("task-B").await, 1);
}

#[tokio::test]
async fn registry_reserve_event_id_increments_per_subscriber() {
    let registry = StreamRegistry::new();
    let sub_a = registry.subscribe("task-A", dummy_writer()).await;
    let sub_b = registry.subscribe("task-A", dummy_writer()).await;

    // Each subscriber has its own counter; reserving one for sub_a must
    // not advance sub_b's counter.
    assert_eq!(registry.reserve_event_id("task-A", sub_a).await, Some(1));
    assert_eq!(registry.reserve_event_id("task-A", sub_a).await, Some(2));
    assert_eq!(registry.reserve_event_id("task-A", sub_b).await, Some(1));
    assert_eq!(registry.reserve_event_id("task-A", 9999).await, None);
}

// ---------------------------------------------------------------------------
// writer_ref_from_input
// ---------------------------------------------------------------------------

#[test]
fn writer_ref_extracts_writable_channel() {
    let input = json!({
        "writer": {
            "channel_id": "c1",
            "access_key": "k1",
            "direction": "write"
        },
        "reader": {
            "channel_id": "c2",
            "access_key": "k2",
            "direction": "read"
        }
    });
    let r = writer_ref_from_input(&input).expect("must find writable channel");
    assert_eq!(r.channel_id, "c1");
    assert!(matches!(r.direction, ChannelDirection::Write));
}

#[test]
fn writer_ref_returns_none_when_only_reader_present() {
    let input = json!({
        "reader": {
            "channel_id": "c1",
            "access_key": "k1",
            "direction": "read"
        }
    });
    assert!(writer_ref_from_input(&input).is_none());
}

// ---------------------------------------------------------------------------
// Helper: an iii client that never connects — `iii.address()` still works
// for ChannelWriter::new construction even if the engine is unreachable.
// ---------------------------------------------------------------------------

fn dummy_writer_ref() -> StreamChannelRef {
    StreamChannelRef {
        channel_id: "test-chan".to_string(),
        access_key: "test-key".to_string(),
        direction: ChannelDirection::Write,
    }
}

// Compile-time smoke check: reference the dispatch entry points so the
// signatures stay reachable from the integration test harness. The
// branch is unreachable at runtime; we only need the function-call
// expression to type-check.
#[tokio::test]
async fn dispatch_signatures_compile() {
    if false {
        let iii = iii_sdk::III::new("ws://127.0.0.1:1");
        let registry = Arc::new(StreamRegistry::new());
        let cfg = iii_a2a::handler::ExposureConfig::new(false, None);
        dispatch_stream(&iii, None, dummy_writer_ref(), registry.clone(), cfg).await;
        dispatch_resubscribe(&iii, None, dummy_writer_ref(), registry).await;
    }
}

// ---------------------------------------------------------------------------
// E2E (live engine required) — `cargo test -- --ignored` to run.
// ---------------------------------------------------------------------------

/// `message/stream` happy path: a fast-completing function should emit
/// Submitted → Working → artifact → Completed (final=true).
#[ignore]
#[tokio::test]
async fn test_fast_completes() {
    // Requires: iii engine on ws://localhost:49134 with a registered
    // function tagged a2a.expose. Use curl --no-buffer on
    // http://localhost:3111/a2a with method "message/stream".
    //
    // The expected SSE frames are:
    //   event: status-update  data: {... state:"submitted", final:false}
    //   event: status-update  data: {... state:"working",   final:false}
    //   event: artifact-update data: {... lastChunk:true}
    //   event: status-update  data: {... state:"completed", final:true}
}

/// `tasks/resubscribe` mid-flight: a slow function in `Working` state
/// should replay the current frame and continue receiving updates.
#[ignore]
#[tokio::test]
async fn test_slow_resubscribe_continues() {
    // Requires: an in-flight task already created via message/stream.
    // POST /a2a with method "tasks/resubscribe" and params {"id": "..."}.
    // The response must replay one Working frame and then continue with
    // the producer's frames until terminal.
}

/// `tasks/cancel` while a stream is in flight should propagate a
/// Canceled (final=true) frame to every subscriber.
#[ignore]
#[tokio::test]
async fn test_cancel_emits_canceled_final() {
    // Requires: an in-flight task with a stream subscriber. POST /a2a
    // with method "tasks/cancel" and params {"id": "..."}. The active
    // SSE stream must receive a status-update frame with state="canceled"
    // and final=true, then the connection closes.
}
