//! Phase 1 — protocol-loop structural guard.
//!
//! `is_protocol_loop` is the single defense the handler keeps after RBAC
//! moved to iii-worker-manager. It blocks `mcp::*` and `a2a::*` function IDs
//! at every dispatch entry point — `tools/call` direct, `iii_trigger_void`,
//! and `iii_trigger_enqueue`. These tests use an unreachable iii engine so
//! `iii.trigger` never runs; the rejection happens before that.

use iii_mcp::handler::McpHandler;
use iii_sdk::III;
use serde_json::{Value, json};

fn unreachable_iii() -> III {
    // Port 1 is reserved/unbound; the SDK reconnect loop runs in the
    // background, but every `iii.trigger` call returns Err immediately.
    // The guards we're testing reject BEFORE any trigger fires, so the
    // engine being unreachable is fine.
    III::new("ws://127.0.0.1:1")
}

async fn handler() -> std::sync::Arc<McpHandler> {
    let h = std::sync::Arc::new(McpHandler::new(
        unreachable_iii(),
        "ws://127.0.0.1:1".to_string(),
        true, // no_builtins=true so builtin tools are out of the way
    ));
    h.handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await;
    h.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    h
}

async fn handler_with_builtins() -> std::sync::Arc<McpHandler> {
    let h = std::sync::Arc::new(McpHandler::new(
        unreachable_iii(),
        "ws://127.0.0.1:1".to_string(),
        false, // no_builtins=false so iii_trigger_* dispatchers are reachable
    ));
    h.handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await;
    h.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    h
}

fn assert_protocol_loop_error(resp: Option<Value>) {
    let v = resp.expect("response");
    let result = &v["result"];
    assert_eq!(result["isError"], true, "expected isError=true, got: {}", v);
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("protocol entry point"),
        "expected 'protocol entry point' in error message, got: {}",
        text
    );
}

#[tokio::test]
async fn tools_call_mcp_handler_rejected() {
    let h = handler().await;
    let resp = h
        .handle(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"mcp__handler","arguments":{}}
        }))
        .await;
    assert_protocol_loop_error(resp);
}

#[tokio::test]
async fn tools_call_a2a_jsonrpc_rejected() {
    let h = handler().await;
    let resp = h
        .handle(json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"a2a__jsonrpc","arguments":{}}
        }))
        .await;
    assert_protocol_loop_error(resp);
}

#[tokio::test]
async fn iii_trigger_void_with_mcp_function_id_rejected() {
    let h = handler_with_builtins().await;
    let resp = h
        .handle(json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{
                "name":"iii_trigger_void",
                "arguments":{"function_id":"mcp::handler","payload":{}}
            }
        }))
        .await;
    assert_protocol_loop_error(resp);
}

#[tokio::test]
async fn iii_trigger_enqueue_with_a2a_function_id_rejected() {
    let h = handler_with_builtins().await;
    let resp = h
        .handle(json!({
            "jsonrpc":"2.0","id":5,"method":"tools/call",
            "params":{
                "name":"iii_trigger_enqueue",
                "arguments":{"function_id":"a2a::jsonrpc","payload":{},"queue":"default"}
            }
        }))
        .await;
    assert_protocol_loop_error(resp);
}
