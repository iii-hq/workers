use a2ui::config::WorkerConfig;
use a2ui::protocol::{
    apply_messages, validate_renderable, ServerMessage, SessionState, SurfaceStatus, CATALOG_ID,
    PROTOCOL_VERSION,
};
use a2ui::store::Store;
use serde_json::json;

fn message(value: serde_json::Value) -> ServerMessage {
    serde_json::from_value(value).expect("valid fixture")
}

#[tokio::test]
async fn surface_lifecycle_is_atomic_per_session() {
    let store = Store::in_memory();
    let cfg = WorkerConfig::default();
    let mut state = store.load("session-a").await.unwrap();
    let created = apply_messages(
        &mut state,
        &[
            message(json!({
                "version": PROTOCOL_VERSION,
                "createSurface": {"surfaceId": "release", "catalogId": CATALOG_ID, "sendDataModel": true}
            })),
            message(json!({
                "version": PROTOCOL_VERSION,
                "updateComponents": {"surfaceId": "release", "components": [
                    {"id": "root", "component": "Card", "child": "body"},
                    {"id": "body", "component": "Column", "children": ["title", "approve"]},
                    {"id": "title", "component": "Text", "text": {"path": "/service"}, "variant": "h2"},
                    {"id": "approve-label", "component": "Text", "text": "Approve"},
                    {"id": "approve", "component": "Button", "child": "approve-label", "action": {"event": {"name": "approve"}}}
                ]}
            })),
            message(json!({
                "version": PROTOCOL_VERSION,
                "updateDataModel": {"surfaceId": "release", "path": "/", "value": {"service": "payments"}}
            })),
        ],
        Some("Release approval"),
        &cfg,
    )
    .unwrap();
    assert!(matches!(created.status, SurfaceStatus::Active));
    validate_renderable(state.get("release").unwrap()).unwrap();
    store.save(&state).await.unwrap();

    let restored = store.load("session-a").await.unwrap();
    let surface = restored.get("release").unwrap();
    assert_eq!(surface.title, "Release approval");
    assert_eq!(surface.data_model["service"], "payments");
    assert_eq!(surface.components.len(), 5);
    assert!(store.load("session-b").await.unwrap().surfaces.is_empty());
}

#[test]
fn invalid_batch_does_not_mutate_the_original_snapshot() {
    let cfg = WorkerConfig::default();
    let state = SessionState::empty("session-a");
    let mut candidate = state.clone();
    let result = apply_messages(
        &mut candidate,
        &[
            message(json!({
                "version": PROTOCOL_VERSION,
                "createSurface": {"surfaceId": "bad", "catalogId": CATALOG_ID}
            })),
            message(json!({
                "version": PROTOCOL_VERSION,
                "updateComponents": {"surfaceId": "bad", "components": [
                    {"id": "root", "component": "Column", "children": ["missing"]}
                ]}
            })),
        ],
        None,
        &cfg,
    );
    assert!(result.is_err());
    assert!(state.surfaces.is_empty());
}

#[test]
fn progressive_component_messages_validate_after_the_atomic_batch() {
    let cfg = WorkerConfig::default();
    let mut state = SessionState::empty("session-a");
    apply_messages(
        &mut state,
        &[
            message(json!({
                "version": PROTOCOL_VERSION,
                "createSurface": {"surfaceId": "progressive", "catalogId": CATALOG_ID}
            })),
            message(json!({
                "version": PROTOCOL_VERSION,
                "updateComponents": {"surfaceId": "progressive", "components": [
                    {"id": "root", "component": "Column", "children": ["late-child"]}
                ]}
            })),
            message(json!({
                "version": PROTOCOL_VERSION,
                "updateComponents": {"surfaceId": "progressive", "components": [
                    {"id": "late-child", "component": "Text", "text": "arrived later"}
                ]}
            })),
        ],
        None,
        &cfg,
    )
    .expect("A2UI progressive references may resolve later in one batch");
    validate_renderable(state.get("progressive").unwrap()).unwrap();
}
