use serde_json::json;

// These tests exercise the JSON-RPC envelope shape and ACP method
// dispatch contract without bringing up an iii engine. Anything
// touching state::* or iii.trigger lives in handler.rs and runs
// against a real engine in CI integration runs.

#[test]
fn jsonrpc_response_success_serializes_with_required_fields() {
    let r = iii_acp::types::JsonRpcResponse::success(Some(json!(1)), json!({ "ok": true }));
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["result"], json!({ "ok": true }));
    assert!(v.get("error").is_none());
}

#[test]
fn jsonrpc_response_error_omits_result() {
    let r = iii_acp::types::JsonRpcResponse::error(Some(json!("abc")), -32601, "missing");
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], "abc");
    assert!(v.get("result").is_none());
    assert_eq!(v["error"]["code"], -32601);
    assert_eq!(v["error"]["message"], "missing");
}

#[test]
fn session_new_params_accepts_minimal() {
    let raw = json!({ "cwd": "/tmp" });
    let p: iii_acp::types::SessionNewParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.cwd, "/tmp");
    assert!(p.mcp_servers.is_empty());
}

#[test]
fn session_new_params_passes_through_stdio_mcp_server() {
    let raw = json!({
        "cwd": "/tmp",
        "mcpServers": [
            { "name": "fs", "command": "/bin/foo", "args": ["--stdio"] }
        ]
    });
    let p: iii_acp::types::SessionNewParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.mcp_servers.len(), 1);
    assert_eq!(p.mcp_servers[0]["name"], "fs");
    assert_eq!(p.mcp_servers[0]["command"], "/bin/foo");
}

#[test]
fn session_new_params_passes_through_http_mcp_server() {
    let raw = json!({
        "cwd": "/tmp",
        "mcpServers": [
            { "type": "http", "name": "remote", "url": "https://example.com/mcp" }
        ]
    });
    let p: iii_acp::types::SessionNewParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.mcp_servers.len(), 1);
    assert_eq!(p.mcp_servers[0]["type"], "http");
    assert_eq!(p.mcp_servers[0]["url"], "https://example.com/mcp");
}

#[test]
fn session_prompt_params_round_trips() {
    let raw = json!({
        "sessionId": "sess_abc",
        "prompt": [{ "type": "text", "text": "hi" }]
    });
    let p: iii_acp::types::SessionPromptParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.session_id, "sess_abc");
    assert_eq!(p.prompt.len(), 1);
}

#[test]
fn parse_returns_error_on_missing_params() {
    let r: Result<iii_acp::types::SessionPromptParams, _> = iii_acp::types::parse(None);
    assert!(r.is_err());
}
