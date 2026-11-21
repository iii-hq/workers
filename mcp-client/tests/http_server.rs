mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::config::RawConfig;
use mcp_client::reconcile::tool_response_format;
use mcp_client::server::{Manager, State};
use serde_json::{Value, json};
use support::mcp_mock::McpMock;
use support::{FakeHost, WAIT, connect, server};

fn cfg(entries: Value) -> Value {
    json!({ "mcpServers": entries })
}

fn tools() -> Value {
    json!([
        { "name": "echo", "description": "Echo the text. Second sentence.", "annotations": { "readOnlyHint": true },
          "inputSchema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object",
                           "properties": { "text": { "type": "string" } }, "required": ["text"] } },
        { "name": "save_thing", "description": "Save a thing.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "fail", "description": "Always fails.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "bad.name", "description": "x", "inputSchema": { "type": "object" } }
    ])
}

async fn ready(mock: &McpMock) -> (Arc<FakeHost>, Arc<Manager>) {
    let host = FakeHost::new();
    let config = cfg(json!({"mock": {"type": "http", "url": mock.url()}}));
    let manager = connect(&host, config).await;
    assert!(
        manager.wait_for("mock", State::Ready, WAIT).await,
        "{}",
        server(&manager, "mock").await
    );
    (host, manager)
}

#[tokio::test]
async fn connects_lists_and_registers_tools() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    mock.instructions("Use echo first.").await;
    let (host, _manager) = ready(&mock).await;

    assert_eq!(
        host.registered_ids(),
        [
            "mcp::mock::echo",
            "mcp::mock::fail",
            "mcp::mock::instructions",
            "mcp::mock::save_thing"
        ]
    );
    let echo = host.def("mcp::mock::echo").unwrap();
    assert_eq!(echo.description, "Echo the text. Second sentence.");
    assert_eq!(
        echo.request_format,
        json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
    );
    assert_eq!(echo.response_format, tool_response_format());
    assert_eq!(
        echo.metadata,
        json!({"mcp": {"server": "mock", "tool": "echo", "annotations": {"readOnlyHint": true}}})
    );

    let init = &mock.requests("initialize").await[0]["params"];
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert_eq!(init["capabilities"], json!({}));
    assert_eq!(init["clientInfo"]["name"], "iii-mcp-client");
    assert_eq!(mock.requests("notifications/initialized").await.len(), 1);
    let listed = &mock.headers_of("tools/list").await[0];
    assert_eq!(listed["mcp-protocol-version"], "2025-11-25");
    assert!(
        listed["accept"].contains("text/event-stream")
            && listed["accept"].contains("application/json")
    );
    assert!(!listed.contains_key("authorization"));
}

#[tokio::test]
async fn servers_list_has_the_final_shape() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    mock.instructions("Use echo first.").await;
    let (host, manager) = ready(&mock).await;

    let list = serde_json::to_value(manager.list().await).unwrap();
    assert_eq!(list["stdio_enabled"], true);
    assert_eq!(
        list["callback_url"],
        "http://127.0.0.1:3111/mcp-client/oauth/callback"
    );
    assert_eq!(list["servers"].as_array().unwrap().len(), 1);
    let mut row = list["servers"][0].clone();
    let fields = row.as_object_mut().unwrap();
    let since = fields.remove("since").unwrap();
    assert!(since.as_u64().unwrap() > 1_700_000_000_000, "{since}");
    let fingerprint = fields.remove("fingerprint").unwrap();
    assert_eq!(fingerprint.as_str().unwrap().len(), 64);
    let reason = row["skipped"][0].as_object_mut().unwrap().remove("reason");
    assert!(reason.unwrap().is_string());
    assert_eq!(
        row,
        json!({
            "name": "mock",
            "transport": "http",
            "target": mock.url(),
            "auth": "none",
            "state": "ready",
            "stage": null,
            "error": null,
            "approved": true,
            "protocol_version": "2025-11-25",
            "server_info": {"name": "mock", "title": null, "version": "1.0.0"},
            "tools_listed": true,
            "tools": [
                {"name": "echo", "function_id": "mcp::mock::echo", "registered": true, "annotations": {"readOnlyHint": true}},
                {"name": "fail", "function_id": "mcp::mock::fail", "registered": true, "annotations": null},
                {"name": "save_thing", "function_id": "mcp::mock::save_thing", "registered": true, "annotations": null}
            ],
            "skipped": [{"kind": "tool", "name": "bad.name"}],
            "has_instructions": true,
            "capabilities": {"resources": false, "prompts": false},
            "resource_functions": [],
            "prompts": [],
            "list_errors": {"prompts": null, "templates": null},
            "login": null,
            "process": null
        })
    );

    host.set_http_port(Some(4555));
    host.drop_registration("mcp::mock::echo");
    let list = serde_json::to_value(manager.list().await).unwrap();
    assert_eq!(
        list["callback_url"],
        "http://127.0.0.1:4555/mcp-client/oauth/callback"
    );
    assert_eq!(list["servers"][0]["tools"][0]["name"], "echo");
    assert_eq!(list["servers"][0]["tools"][0]["registered"], false);
}

#[tokio::test]
async fn disabled_and_invalid_entries_do_not_connect_and_do_not_stop_the_others() {
    let idle = McpMock::start().await;
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    let host = FakeHost::new();
    let config = cfg(json!({
        "off": {"url": idle.url(), "disabled": true},
        "bad": {"url": "ftp://example.com/mcp"},
        "dollar": {"url": "https://example.com/${TENANT}/mcp"},
        "ok": {"url": mock.url()}
    }));
    let manager = connect(&host, config).await;
    assert!(
        manager.wait_for("ok", State::Ready, WAIT).await,
        "{}",
        server(&manager, "ok").await
    );

    let off = server(&manager, "off").await;
    assert_eq!(off["state"], "disabled", "{off}");
    assert_eq!(off["stage"], Value::Null, "{off}");
    assert_eq!(off["error"], Value::Null, "{off}");
    for name in ["bad", "dollar"] {
        let r = server(&manager, name).await;
        assert_eq!(r["state"], "config_error", "{r}");
        assert_eq!(r["stage"], "config", "{r}");
        assert_eq!(r["error"]["code"], "invalid_config", "{r}");
        assert_eq!(r["tools_listed"], false, "{r}");
    }
    let dollar = server(&manager, "dollar").await;
    assert!(
        dollar["error"]["message"]
            .as_str()
            .unwrap()
            .contains("env://"),
        "{dollar}"
    );
    assert_eq!(dollar["target"], "https://example.com/${TENANT}/mcp");
    assert_eq!(idle.request_count().await, 0);
    assert!(!host.registered_ids().is_empty());
    assert!(
        host.registered_ids()
            .iter()
            .all(|id| id.starts_with("mcp::ok::"))
    );
}

#[tokio::test]
async fn applying_the_same_config_twice_does_not_reconnect() {
    let mock = McpMock::start().await;
    let (_host, manager) = ready(&mock).await;
    let config = cfg(json!({"mock": {"type": "http", "url": mock.url()}}));
    manager.apply_config(RawConfig::from_value(&config)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(mock.requests("initialize").await.len(), 1);
    assert_eq!(server(&manager, "mock").await["state"], "ready");
}

#[tokio::test]
async fn call_strips_engine_keys_and_translates_the_result() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    mock.instructions("Use echo first.").await;
    mock.on_call("echo", json!({"content": [{"type": "text", "text": "hi"}]}))
        .await;
    let (host, _manager) = ready(&mock).await;

    let out = host
        .call(
            "mcp::mock::echo",
            json!({ "text": "hi", "_caller_worker_id": "w1" }),
        )
        .await;
    assert_eq!(
        out,
        Ok(json!({ "content": [{ "type": "text", "text": "hi" }] }))
    );
    let sent = &mock.requests("tools/call").await[0]["params"];
    assert_eq!(sent["name"], "echo");
    assert_eq!(sent["arguments"], json!({ "text": "hi" }));

    host.call("mcp::mock::echo", json!("not an object"))
        .await
        .unwrap();
    let sent = &mock.requests("tools/call").await[1]["params"];
    assert_eq!(sent["arguments"], json!({}));

    assert_eq!(
        host.call("mcp::mock::instructions", json!({})).await,
        Ok(json!({ "content": [{ "type": "text", "text": "Use echo first." }] }))
    );
}

#[tokio::test]
async fn tool_errors_come_back_as_err() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    mock.on_call(
        "fail",
        json!({"content": [{"type": "text", "text": "boom"}], "isError": true}),
    )
    .await;
    mock.on_call_error("echo", -32602, "team is required").await;
    mock.on_call_error("save_thing", -32000, "nope").await;
    let (host, manager) = ready(&mock).await;

    assert_eq!(
        host.call("mcp::mock::fail", json!({})).await,
        Err("boom".into())
    );
    assert_eq!(
        host.call("mcp::mock::echo", json!({"text": "x"})).await,
        Err("invalid_argument: team is required".into())
    );
    assert_eq!(
        host.call("mcp::mock::save_thing", json!({})).await,
        Err("MCP error -32000: nope".into())
    );
    assert_eq!(server(&manager, "mock").await["state"], "ready");
}

#[tokio::test]
async fn a_call_past_the_limit_says_outcome_unknown() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    mock.on_call("echo", json!({"content": []})).await;
    mock.delay("tools/call", Duration::from_secs(3)).await;
    let (host, manager) = ready(&mock).await;

    let error = host
        .call("mcp::mock::echo", json!({"text": "x"}))
        .await
        .unwrap_err();
    assert!(
        error.contains("outcome unknown: check before retrying a write"),
        "{error}"
    );
    assert_eq!(server(&manager, "mock").await["state"], "ready");
}

fn bearer(mock: &McpMock) -> Value {
    json!({"url": mock.url(), "auth": {"type": "bearer", "token": "secret://MOCK_KEY"}})
}

async fn settle(manager: &Arc<Manager>, name: &str, state: State) -> Value {
    let reached = manager.wait_for(name, state, WAIT).await;
    let row = server(manager, name).await;
    assert!(reached, "{row}");
    row
}

#[tokio::test]
async fn bearer_and_headers_are_sent_and_never_listed() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    mock.on_call("echo", json!({"content": []})).await;
    let host = FakeHost::new();
    host.set_secret("secret://MOCK_KEY", "tok-123");
    host.set_secret("env://WS_KEY", "k-9");
    let mut entry = bearer(&mock);
    entry["headers"] = json!({"X-API-Key": "env://WS_KEY", "X-Workspace": "acme"});
    let manager = connect(&host, cfg(json!({ "mock": entry }))).await;
    settle(&manager, "mock", State::Ready).await;
    host.call("mcp::mock::echo", json!({"text": "x"}))
        .await
        .unwrap();

    for method in ["initialize", "tools/list", "tools/call"] {
        let headers = &mock.headers_of(method).await[0];
        assert_eq!(headers["authorization"], "Bearer tok-123", "{method}");
        assert_eq!(headers["x-api-key"], "k-9", "{method}");
        assert_eq!(headers["x-workspace"], "acme", "{method}");
    }
    let listed = serde_json::to_string(&manager.list().await).unwrap();
    assert!(
        !listed.contains("tok-123") && !listed.contains("k-9"),
        "{listed}"
    );
    assert_eq!(server(&manager, "mock").await["auth"], "bearer");
}

#[tokio::test]
async fn secret_errors_map_to_states_and_send_nothing() {
    let mock = McpMock::start().await;
    let config = cfg(json!({ "mock": bearer(&mock) }));

    let host = FakeHost::new();
    let manager = connect(&host, config.clone()).await;
    let r = settle(&manager, "mock", State::ConfigError).await;
    assert_eq!(r["stage"], "secret", "{r}");
    assert_eq!(r["error"]["code"], "secret_missing", "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(message.contains("`secret://MOCK_KEY`"), "{message}");

    let host = FakeHost::new();
    host.set_secret("secret://MOCK_KEY", "tok-123");
    host.forbid("secret://MOCK_KEY");
    let manager = connect(&host, config.clone()).await;
    let r = settle(&manager, "mock", State::ConfigError).await;
    assert_eq!(r["stage"], "secret", "{r}");
    assert_eq!(r["error"]["code"], "secret_not_shared", "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(message.contains("`mcp-client`"), "{message}");

    let host = FakeHost::new();
    host.set_unreachable(true);
    let manager = connect(&host, config).await;
    let r = settle(&manager, "mock", State::Unavailable).await;
    assert_eq!(r["stage"], "secret", "{r}");
    assert_eq!(r["error"]["code"], "connect_failed", "{r}");

    assert_eq!(mock.request_count().await, 0);
}

#[tokio::test]
async fn a_401_is_needs_login_without_auth_and_auth_failed_with_a_credential() {
    let mock = McpMock::start().await;
    mock.http_status(
        "initialize",
        401,
        &[(
            "WWW-Authenticate",
            r#"Bearer resource_metadata="http://127.0.0.1/.well-known/oauth-protected-resource""#,
        )],
    )
    .await;
    let host = FakeHost::new();
    host.set_secret("secret://MOCK_KEY", "tok-123");
    let manager = connect(
        &host,
        cfg(json!({
            "auto": {"url": mock.url()},
            "keyed": bearer(&mock),
            "hdr": {"url": mock.url(), "headers": {"X-API-Key": "secret://MOCK_KEY"}}
        })),
    )
    .await;

    let auto = settle(&manager, "auto", State::NeedsLogin).await;
    assert_eq!(auto["stage"], "authorize", "{auto}");
    assert_eq!(auto["auth"], "oauth", "{auto}");
    assert_eq!(
        auto["error"],
        json!({"code": "login_required", "message": "sign-in required, an operator must sign in on the MCP servers page"})
    );
    let rejected = json!({"code": "credential_rejected", "message": "credential rejected, an operator must fix `secret://MOCK_KEY`"});
    let keyed = settle(&manager, "keyed", State::AuthFailed).await;
    assert_eq!(keyed["stage"], "authorize", "{keyed}");
    assert_eq!(keyed["auth"], "bearer", "{keyed}");
    assert_eq!(keyed["error"], rejected);
    let hdr = settle(&manager, "hdr", State::AuthFailed).await;
    assert_eq!(hdr["auth"], "header", "{hdr}");
    assert_eq!(hdr["error"], rejected);
    assert!(host.registered_ids().is_empty());

    mock.http_status("initialize", 401, &[]).await;
    let host = FakeHost::new();
    let manager = connect(&host, cfg(json!({"plain": {"url": mock.url()}}))).await;
    let plain = settle(&manager, "plain", State::AuthFailed).await;
    assert_eq!(plain["auth"], "none", "{plain}");
    assert_eq!(
        plain["error"],
        json!({"code": "credential_rejected", "message": "plain answered 401 and offers no sign-in"})
    );
}

#[tokio::test]
async fn a_connect_error_never_carries_a_resolved_value() {
    let host = FakeHost::new();
    host.set_secret(
        "secret://MOCK_URL",
        "http://127.0.0.1:1/mcp?key=tok-123-sentinel",
    );
    let manager = connect(&host, cfg(json!({"m": {"url": "secret://MOCK_URL"}}))).await;
    let r = settle(&manager, "m", State::Unavailable).await;
    assert_eq!(r["stage"], "connect", "{r}");
    assert_eq!(r["error"]["code"], "connect_failed", "{r}");
    assert_eq!(r["target"], "secret://MOCK_URL", "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("could not reach secret://MOCK_URL: "),
        "{message}"
    );
    assert!(
        !r.to_string().contains("sentinel") && !r.to_string().contains("127.0.0.1:1"),
        "{r}"
    );
}

#[tokio::test]
async fn a_url_reference_that_resolves_to_plain_http_is_refused() {
    let host = FakeHost::new();
    host.set_secret("secret://MOCK_URL", "http://mcp.example.com/mcp");
    let manager = connect(&host, cfg(json!({"m": {"url": "secret://MOCK_URL"}}))).await;
    let r = settle(&manager, "m", State::ConfigError).await;
    assert_eq!(r["stage"], "secret", "{r}");
    assert_eq!(r["error"]["code"], "invalid_config", "{r}");
    assert_eq!(
        r["error"]["message"],
        "the address stored in `secret://MOCK_URL` must be https, or http on a loopback host"
    );
    assert!(!r.to_string().contains("mcp.example.com"), "{r}");
}

#[tokio::test]
async fn a_url_secret_is_scrubbed_in_its_normalized_form() {
    let mock = McpMock::start().await;
    mock.tools(tools()).await;
    let base = mock.url().trim_end_matches("/mcp").to_string();
    let host = FakeHost::new();
    // reqwest prints it as `<base>/?api_key=…`, which the stored text is not.
    host.set_secret("secret://MOCK_URL", &format!("{base}?api_key=SENTINEL-KEY"));
    let manager = connect(&host, cfg(json!({"m": {"url": "secret://MOCK_URL"}}))).await;
    settle(&manager, "m", State::Ready).await;

    drop(mock);
    let error = host
        .call("mcp::m::echo", json!({"text": "x"}))
        .await
        .unwrap_err();
    assert!(!error.contains("SENTINEL-KEY"), "{error}");
    let listed = serde_json::to_string(&manager.list().await).unwrap();
    assert!(!listed.contains("SENTINEL-KEY"), "{listed}");
}
