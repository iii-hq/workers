use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use iii_mcp_client::session::{Session, SessionSpec};
use iii_mcp_client::transport::Transport;
use serde_json::{json, Value};
use tokio::io::{duplex, DuplexStream};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

/// Mock MCP server: handles initialize, tools/list, tools/call(ping).
async fn run_mock_server(read: DuplexStream, write: DuplexStream) {
    let mut reader = FramedRead::new(read, LinesCodec::new());
    let mut writer = FramedWrite::new(write, LinesCodec::new());

    while let Some(Ok(line)) = reader.next().await {
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = value.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = value.get("id").cloned();

        match method {
            "initialize" => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {
                            "tools": { "listChanged": false }
                        },
                        "serverInfo": { "name": "mock", "version": "0.0.0" }
                    }
                });
                writer.send(resp.to_string()).await.ok();
            }
            "notifications/initialized" => {
                // no response for notifications
            }
            "tools/list" => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [{
                            "name": "ping",
                            "description": "ping the mock",
                            "inputSchema": { "type": "object" }
                        }]
                    }
                });
                writer.send(resp.to_string()).await.ok();
            }
            "tools/call" => {
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let result = if tool_name == "ping" {
                    json!({ "pong": true })
                } else {
                    json!({ "error": "unknown tool" })
                };
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result
                });
                writer.send(resp.to_string()).await.ok();
            }
            _ => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("unknown method {method}") }
                });
                writer.send(resp.to_string()).await.ok();
            }
        }
    }
}

async fn connect_to_mock() -> Arc<Session> {
    let (client_read_end, server_write_end) = duplex(64 * 1024);
    let (server_read_end, client_write_end) = duplex(64 * 1024);

    tokio::spawn(run_mock_server(server_read_end, server_write_end));

    let transport = Arc::new(Transport::from_duplex(client_read_end, client_write_end));
    Session::connect_with_transport("mock", transport)
        .await
        .expect("session connect")
}

#[tokio::test]
async fn session_connect_succeeds() {
    let session = connect_to_mock().await;
    let caps = session.capabilities.read().await.clone();
    assert!(caps.is_some(), "capabilities should be populated post-init");
    assert!(caps.unwrap().tools.is_some());
}

#[tokio::test]
async fn list_tools_returns_ping() {
    let session = connect_to_mock().await;
    let tools = session.list_tools().await.expect("list_tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "ping");
}

#[tokio::test]
async fn tools_call_ping_returns_pong() {
    let session = connect_to_mock().await;
    let result = session
        .tools_call("ping", json!({}))
        .await
        .expect("tools_call ping");
    assert_eq!(result, json!({ "pong": true }));
}

#[test]
fn parse_stdio_spec() {
    let spec = SessionSpec::parse("stdio:fs:npx:-y:server-fs:/tmp").unwrap();
    match spec {
        SessionSpec::Stdio { name, bin, args } => {
            assert_eq!(name, "fs");
            assert_eq!(bin, "npx");
            assert_eq!(args, vec!["-y", "server-fs", "/tmp"]);
        }
        _ => panic!("expected stdio"),
    }
}

#[test]
fn parse_http_spec() {
    let spec = SessionSpec::parse("http:gh:https://example.com/mcp").unwrap();
    match spec {
        SessionSpec::Http { name, url } => {
            assert_eq!(name, "gh");
            assert_eq!(url, "https://example.com/mcp");
        }
        _ => panic!("expected http"),
    }
}

#[tokio::test]
#[ignore]
async fn live_engine_register_and_trigger() {
    use iii_mcp_client::registration::register_all;
    use iii_sdk::{register_worker, InitOptions};

    let session = connect_to_mock().await;
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    register_all(&iii, session.clone(), "mcp")
        .await
        .expect("register_all");

    // Round-trip via the engine: trigger mcp.mock::ping
    let req = iii_sdk::TriggerRequest {
        function_id: "mcp.mock::ping".to_string(),
        payload: json!({}),
        action: None,
        timeout_ms: Some(5000),
    };
    let result = iii.trigger(req).await.expect("trigger");
    assert_eq!(result, json!({ "pong": true }));

    iii.shutdown_async().await;
}
