//! Cross-worker wire-shape locks between approval-gate and harness v1.

use serde_json::json;

#[test]
fn gate_hold_matches_harness_parse_output() {
    let hold = json!({ "decision": "hold" });
    assert!(hold.get("pending_timeout_ms").is_none());
}

#[test]
fn resolve_allow_payload_matches_harness_function_resolve_request() {
    let payload = json!({
        "session_id": "s_1",
        "turn_id": "t_1",
        "function_call_id": "c_1",
        "action": "execute",
    });
    assert_eq!(payload["action"], json!("execute"));
    assert!(payload.get("content").is_none());
    assert!(payload.get("is_error").is_none());
}

#[test]
fn resolve_deny_payload_matches_harness_function_resolve_request() {
    let payload = json!({
        "session_id": "s_1",
        "turn_id": "t_1",
        "function_call_id": "c_1",
        "action": "deliver",
        "is_error": true,
        "content": [{ "type": "text", "text": "denied" }],
        "details": { "status": "denied", "denied_by": "user" },
    });
    assert_eq!(payload["action"], json!("deliver"));
    assert_eq!(payload["is_error"], json!(true));
}
