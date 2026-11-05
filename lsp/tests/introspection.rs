//! Guards the engine-introspection deserialization contract.
//!
//! These tests pin the row shapes of `engine::functions::list` and
//! `engine::observers::list`, so a future engine/SDK bump that drifts the
//! payload fails here instead of silently leaving the LSP caches empty.

use lsp::engine_introspection::{FunctionInfo, ObserverInfo};
use serde_json::json;

/// `engine::functions::list` rows carry `observer_count` always, and
/// `args_format`, `private: true` and `observable: false` only when set.
#[test]
fn function_info_parses_an_event_source_row() {
    let row = json!({
        "function_id": "cron::on-tick",
        "namespace": "default",
        "worker_name": "iii-cron",
        "description": "Called once per tick of an observer's expression.",
        "args_format": {
            "type": "object",
            "properties": { "expression": { "type": "string" } },
            "required": ["expression"]
        },
        "private": true,
        "observer_count": 2,
    });

    let info: FunctionInfo = serde_json::from_value(row).expect("row must deserialize");

    assert_eq!(info.function_id, "cron::on-tick");
    assert!(info.private);
    assert!(info.observable);
    assert_eq!(info.observer_count, 2);
    assert_eq!(info.args_format.unwrap()["required"][0], "expression");
}

/// A plain function's row leaves the defaults out: public, observable, no
/// args.
#[test]
fn function_info_defaults_a_plain_row() {
    let row = json!({
        "function_id": "todos::create",
        "namespace": "default",
        "worker_name": "todos",
        "observer_count": 0,
    });

    let info: FunctionInfo = serde_json::from_value(row).expect("row must deserialize");

    assert!(!info.private);
    assert!(info.observable);
    assert!(info.args_format.is_none());
    assert!(info.request_format.is_none());

    let secret: FunctionInfo = serde_json::from_value(json!({
        "function_id": "secrets::resolve",
        "observable": false,
        "observer_count": 0,
    }))
    .expect("row must deserialize");
    assert!(!secret.observable);
}

/// `engine::observers::list` returns `{ "observers": [{ id, function_id,
/// namespace, args, filters, target: { function_id, namespace }, capture,
/// on, deliver, worker, state, fires, … }] }`. `ObserverInfo` reads what the
/// server uses and ignores the rest.
#[test]
fn observer_info_parses_observers_list_row() {
    let row = json!({
        "id": "5b0c7d1e",
        "function_id": "http::on-request",
        "namespace": "default",
        "args": { "method": "POST", "path": "/hooks" },
        "filters": { "$and": [{ "input.headers.x-kind": { "$eq": "push" } }] },
        "target": { "function_id": "example::handler", "namespace": "default" },
        "capture": ["input", "output", "caller"],
        "on": "success",
        "deliver": "envelope",
        "worker": { "worker_id": "0d1f", "worker_name": "example" },
        "state": "active",
        "fires": 3,
        "dropped": {},
        "activation_failures": {},
        "max_in_flight": 64,
        "created_at_ms": 1_761_700_000_000_u64,
    });

    let info: ObserverInfo = serde_json::from_value(row).expect("row must deserialize");

    assert_eq!(info.id, "5b0c7d1e");
    assert_eq!(info.function_id, "http::on-request");
    assert_eq!(info.args["path"], "/hooks");
    assert_eq!(info.target.function_id, "example::handler");
    assert_eq!(info.state.as_deref(), Some("active"));
}
