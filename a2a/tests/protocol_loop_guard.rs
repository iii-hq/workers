//! Phase 1 — protocol-loop structural guard for the A2A handler.
//!
//! `is_protocol_loop` in `iii_a2a::handler` rejects `mcp::*` and `a2a::*`
//! function IDs at `handle_send` time, BEFORE the Working task is built and
//! stored. This avoids the older Working→Failed double-write the v0.3
//! codepath did. RBAC for everything else is delegated to iii-worker-manager.

use std::sync::Arc;

use iii_a2a::handler::handle_a2a_request;
use iii_a2a::streaming::StreamRegistry;
use iii_a2a::types::{A2ARequest, TaskState};
use iii_sdk::III;
use serde_json::json;

fn unreachable_iii() -> III {
    III::new("ws://127.0.0.1:1")
}

#[tokio::test]
async fn message_send_with_mcp_handler_function_id_fails_with_protocol_loop_text() {
    let iii = unreachable_iii();
    let request = A2ARequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!("p1")),
        method: "message/send".to_string(),
        params: Some(json!({
            "message": {
                "messageId": "m1",
                "role": "user",
                "parts": [
                    { "data": { "function_id": "mcp::handler", "payload": {} } }
                ]
            }
        })),
    };

    let registry = Arc::new(StreamRegistry::new());
    let response = handle_a2a_request(&iii, request, &registry).await;
    let result = response.result.expect("expected success-shaped response");
    let task = result.get("task").expect("task in result");

    let state: TaskState =
        serde_json::from_value(task["status"]["state"].clone()).expect("decode state");
    assert_eq!(state, TaskState::Failed, "expected task state=failed");

    let text = task["status"]["message"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert!(
        text.contains("protocol entry point"),
        "expected 'protocol entry point' in rejection text, got: {}",
        text
    );
    assert!(
        text.contains("mcp::handler"),
        "expected the rejected function id 'mcp::handler' in text, got: {}",
        text
    );
}
