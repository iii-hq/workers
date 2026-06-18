use serde_json::json;

// These tests exercise the JSON-RPC envelope shape and ACP method
// dispatch contract without bringing up an iii engine. Anything
// touching state::* or iii.trigger lives in handler.rs and runs
// against a real engine in CI integration runs.

#[test]
fn jsonrpc_response_success_serializes_with_required_fields() {
    let r = acp::types::JsonRpcResponse::success(Some(json!(1)), json!({ "ok": true }));
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["result"], json!({ "ok": true }));
    assert!(v.get("error").is_none());
}

#[test]
fn jsonrpc_response_error_omits_result() {
    let r = acp::types::JsonRpcResponse::error(Some(json!("abc")), -32601, "missing");
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
    let p: acp::types::SessionNewParams = serde_json::from_value(raw).unwrap();
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
    let p: acp::types::SessionNewParams = serde_json::from_value(raw).unwrap();
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
    let p: acp::types::SessionNewParams = serde_json::from_value(raw).unwrap();
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
    let p: acp::types::SessionPromptParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.session_id, "sess_abc");
    assert_eq!(p.prompt.len(), 1);
}

#[test]
fn parse_returns_error_on_missing_params() {
    let r: Result<acp::types::SessionPromptParams, _> = acp::types::parse(None);
    assert!(r.is_err());
}

#[test]
fn session_resume_params_round_trips() {
    let raw = json!({
        "sessionId": "sess_abc",
        "cwd": "/home/user/project",
        "mcpServers": []
    });
    let p: acp::types::SessionResumeParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.session_id, "sess_abc");
    assert_eq!(p.cwd, "/home/user/project");
    assert!(p.mcp_servers.is_empty());
}

#[test]
fn session_set_mode_params_round_trips() {
    let raw = json!({ "sessionId": "sess_x", "modeId": "code" });
    let p: acp::types::SessionSetModeParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.session_id, "sess_x");
    assert_eq!(p.mode_id, "code");
}

#[test]
fn session_set_config_option_params_round_trips() {
    let raw = json!({
        "sessionId": "sess_x",
        "configId": "thinking_level",
        "value": "high"
    });
    let p: acp::types::SessionSetConfigOptionParams = serde_json::from_value(raw).unwrap();
    assert_eq!(p.session_id, "sess_x");
    assert_eq!(p.config_id, "thinking_level");
    assert_eq!(p.value, json!("high"));
}
