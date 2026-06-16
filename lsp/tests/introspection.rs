//! Guards the engine-introspection deserialization contract.
//!
//! The discovery rework changed which builtin each introspection helper
//! must call and the envelope each returns. These tests pin the row shapes
//! so a future engine/SDK bump that drifts the payload fails here instead
//! of silently leaving the LSP caches empty.

use lsp::engine_introspection::{TriggerInfo, TriggerTypeInfo};
use serde_json::json;

/// `engine::triggers::list` returns trigger TYPES:
/// `{ "triggers": [{ id, worker_name, description }] }`.
/// `TriggerTypeInfo` must parse a row of that shape; the optional
/// `*_format` fields are absent and must default to `None`.
#[test]
fn trigger_type_info_parses_triggers_list_row() {
    let row = json!({
        "id": "cron",
        "worker_name": "iii-cron",
        "description": "Cron trigger type",
    });

    let info: TriggerTypeInfo = serde_json::from_value(row).expect("row must deserialize");

    assert_eq!(info.id, "cron");
    assert_eq!(info.description, "Cron trigger type");
    assert!(info.trigger_request_format.is_none());
    assert!(info.call_request_format.is_none());
}

/// `engine::registered-triggers::list` returns trigger INSTANCES:
/// `{ "registered_triggers": [{ id, trigger_type, function_id, worker_name,
/// config, config_summary }] }`. `TriggerInfo` must parse a row of that
/// shape — extra engine fields (`worker_name`, `config_summary`) are
/// ignored and the absent `metadata` defaults to `None`.
#[test]
fn trigger_info_parses_registered_triggers_list_row() {
    let row = json!({
        "id": "70c0e45d",
        "trigger_type": "http",
        "function_id": "example::handler",
        "worker_name": "example",
        "config": { "api_path": "/hooks", "http_method": "POST" },
        "config_summary": "POST /hooks",
    });

    let info: TriggerInfo = serde_json::from_value(row).expect("row must deserialize");

    assert_eq!(info.id, "70c0e45d");
    assert_eq!(info.trigger_type, "http");
    assert_eq!(info.function_id, "example::handler");
    assert_eq!(info.config["api_path"], "/hooks");
    assert!(info.metadata.is_none());
}
