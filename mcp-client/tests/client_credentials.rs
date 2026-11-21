mod support;

use std::sync::Arc;

use mcp_client::auth::{Minter, http_client, mint, token_request_body};
use mcp_client::server::{ErrorCode, Manager, State};
use serde_json::{Value, json};
use support::mcp_mock::{McpMock, echo_mock};
use support::{ECHO, FakeHost, RegOp, WAIT, connect, connect_unapproved, fast_timings, hi, server};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET_REJECTED: &str = "credential rejected, an operator must fix `secret://APP_SECRET`";

/// `srv` with client credentials whose secret is `secret://APP_SECRET`.
fn cc(mock: &McpMock, token_url: &str) -> Value {
    json!({"mcpServers": {"srv": {"type": "http", "url": mock.url(), "auth": {
        "type": "client_credentials", "token_url": token_url, "client_id": "app-1",
        "client_secret": "secret://APP_SECRET", "scope": "read,write"}}}})
}

/// `srv` sending `secret://KEY` as a bearer token.
fn keyed(mock: &McpMock) -> Value {
    json!({"mcpServers": {"srv": {"type": "http", "url": mock.url(),
        "auth": {"type": "bearer", "token": "secret://KEY"}}}})
}

async fn settle(manager: &Arc<Manager>, state: State) -> Value {
    let reached = manager.wait_for("srv", state, WAIT).await;
    let row = server(manager, "srv").await;
    assert!(reached, "{row}");
    row
}

async fn cc_ready() -> (McpMock, Arc<FakeHost>, Arc<Manager>) {
    cc_ready_with("tok-1").await
}

/// `srv` at `ready` with the token endpoint issuing `token` and the server
/// taking only it.
async fn cc_ready_with(token: &str) -> (McpMock, Arc<FakeHost>, Arc<Manager>) {
    let mock = echo_mock().await;
    let host = FakeHost::new();
    host.set_secret("secret://APP_SECRET", "shh");
    let token_url = mock.token_endpoint(token).await;
    mock.require_bearer(token).await;
    let manager = connect(&host, cc(&mock, &token_url)).await;
    settle(&manager, State::Ready).await;
    (mock, host, manager)
}

async fn keyed_ready() -> (McpMock, Arc<FakeHost>, Arc<Manager>) {
    let mock = echo_mock().await;
    let host = FakeHost::new();
    host.set_secret("secret://KEY", "k1");
    mock.require_bearer("k1").await;
    let manager = connect(&host, keyed(&mock)).await;
    settle(&manager, State::Ready).await;
    (mock, host, manager)
}

/// A bare token endpoint answering `status` with `body`; its URL.
async fn token_server(status: u16, body: &str) -> (MockServer, String) {
    let token_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&token_server)
        .await;
    let url = format!("{}/oauth/token", token_server.uri());
    (token_server, url)
}

fn rustls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[tokio::test]
async fn mints_at_connect_and_sends_the_token_as_bearer() {
    let (mock, _host, manager) = cc_ready().await;
    assert_eq!(
        mock.token_requests().await,
        ["grant_type=client_credentials&client_id=app-1&client_secret=shh&scope=read%2Cwrite"]
    );
    assert_eq!(
        mock.headers_of("tools/list").await[0]["authorization"],
        "Bearer tok-1"
    );
    assert_eq!(server(&manager, "srv").await["auth"], "client_credentials");
}

#[tokio::test]
async fn a_401_re_mints_reconnects_and_retries_the_call_once() {
    let (mock, host, manager) = cc_ready().await;
    mock.token_endpoint("tok-2").await;
    mock.require_bearer("tok-2").await;
    assert_eq!(host.call(ECHO, json!({"text": "x"})).await, Ok(hi()));
    assert_eq!(mock.token_requests().await.len(), 2);
    assert_eq!(
        mock.requests("tools/call").await.len(),
        2,
        "one refused, one retried"
    );
    assert!(
        !host
            .ops()
            .iter()
            .any(|op| matches!(op, RegOp::Unregister(_)))
    );
    assert_eq!(server(&manager, "srv").await["state"], "ready");
}

#[tokio::test]
async fn a_second_401_sets_auth_failed_and_names_the_reference() {
    let (mock, host, manager) = cc_ready().await;
    mock.require_bearer("never-issued").await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        SECRET_REJECTED
    );
    let row = settle(&manager, State::AuthFailed).await;
    assert_eq!(row["error"]["code"], "credential_rejected", "{row}");
    assert_eq!(
        mock.token_requests().await.len(),
        2,
        "connect, then exactly one re-mint"
    );
    assert!(host.registered_ids().contains(&ECHO.to_string()));
    let down = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        down.starts_with("`srv` MCP unavailable (`auth_failed`)"),
        "{down}"
    );
}

#[tokio::test]
async fn two_concurrent_401s_cause_one_re_mint() {
    let (mock, host, _manager) = cc_ready().await;
    mock.token_endpoint("tok-2").await;
    mock.require_bearer("tok-2").await;
    let (a, b) = tokio::join!(host.call(ECHO, json!({})), host.call(ECHO, json!({})));
    assert_eq!(a, Ok(hi()));
    assert_eq!(b, Ok(hi()));
    assert_eq!(mock.token_requests().await.len(), 2);
    assert_eq!(
        mock.requests("initialize").await.len(),
        2,
        "one reconnect, not two"
    );
}

#[tokio::test]
async fn a_timeout_is_not_retried() {
    let (mock, host, manager) = cc_ready().await;
    mock.delay("tools/call", fast_timings().call * 3).await;
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        error.contains("outcome unknown: check before retrying a write"),
        "{error}"
    );
    assert_eq!(mock.requests("tools/call").await.len(), 1);
    assert_eq!(mock.token_requests().await.len(), 1);
    assert_eq!(server(&manager, "srv").await["state"], "ready");
}

#[tokio::test]
async fn a_401_re_resolves_a_bearer_key_and_retries_once() {
    let (mock, host, _manager) = keyed_ready().await;
    host.set_secret("secret://KEY", "k2");
    mock.require_bearer("k2").await;
    assert_eq!(host.call(ECHO, json!({})).await, Ok(hi()));
    assert_eq!(host.resolve_count("secret://KEY"), 2);
    assert_eq!(mock.requests("tools/call").await.len(), 2);
}

#[tokio::test]
async fn a_rejected_bearer_key_fails_after_exactly_one_retry() {
    let (mock, host, manager) = keyed_ready().await;
    mock.http_status(
        "tools/call",
        401,
        &[("www-authenticate", "Bearer realm=\"x\"")],
    )
    .await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "credential rejected, an operator must fix `secret://KEY`"
    );
    assert_eq!(mock.requests("tools/call").await.len(), 2);
    assert_eq!(host.resolve_count("secret://KEY"), 2);
    settle(&manager, State::AuthFailed).await;
}

#[tokio::test]
async fn a_rejected_secret_header_fails_after_exactly_one_retry() {
    let mock = echo_mock().await;
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    let entry = json!({"type": "http", "url": mock.url(), "headers": {"X-API-Key": "secret://K"}});
    let manager = connect(&host, json!({"mcpServers": {"srv": entry}})).await;
    settle(&manager, State::Ready).await;
    mock.http_status("tools/call", 401, &[]).await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "credential rejected, an operator must fix `secret://K`"
    );
    assert_eq!(mock.requests("tools/call").await.len(), 2);
}

#[tokio::test]
async fn the_minter_shares_one_post_and_mints_again_after_invalidate() {
    rustls();
    let mock = McpMock::start().await;
    let minter = Minter::new(
        http_client(),
        mock.token_endpoint("tok-1").await,
        token_request_body("a", "b", None),
    );
    let (a, b) = tokio::join!(minter.token(), minter.token());
    assert_eq!(a.unwrap(), "tok-1");
    assert_eq!(b.unwrap(), "tok-1");
    assert_eq!(mock.token_requests().await.len(), 1);
    minter.invalidate().await;
    assert!(minter.token().await.is_ok());
    assert_eq!(mock.token_requests().await.len(), 2);
}

#[tokio::test]
async fn a_refused_mint_reports_the_endpoints_words_and_nothing_secret() {
    rustls();
    // (status, reply body, err.message)
    let rows = [
        (
            401,
            r#"{"error":"invalid_client","error_description":"Invalid client_secret"}"#,
            "token request rejected: Invalid client_secret",
        ),
        (
            400,
            r#"{"error":"invalid_scope"}"#,
            "token request rejected: invalid_scope",
        ),
        (
            500,
            "<html>upstream said shh-secret</html>",
            "token request rejected: HTTP 500",
        ),
        (
            200,
            r#"{"token_type":"Bearer"}"#,
            "token request rejected: the answer holds no access_token",
        ),
    ];
    let body = || token_request_body("app-1", "shh-secret", Some("read,write"));
    for (status, reply, message) in rows {
        let (_token_server, url) = token_server(status, reply).await;
        let err = mint(&http_client(), &url, body()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::CredentialRejected, "{status}");
        assert_eq!(err.message, message, "{status}");
    }
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let url = format!("http://127.0.0.1:{port}/oauth/token");
    let err = mint(&http_client(), &url, body()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ConnectFailed);
    assert_eq!(err.message, "token endpoint did not answer");
}

#[tokio::test]
async fn a_refused_mint_is_auth_failed_and_sends_nothing_to_the_mcp_server() {
    let mock = echo_mock().await;
    let reply = r#"{"error":"invalid_client","error_description":"Invalid client_secret"}"#;
    let (_token_server, url) = token_server(401, reply).await;
    let host = FakeHost::new();
    host.set_secret("secret://APP_SECRET", "shh");
    let manager = connect(&host, cc(&mock, &url)).await;
    let row = settle(&manager, State::AuthFailed).await;
    assert_eq!(row["stage"], "authorize", "{row}");
    assert_eq!(
        row["error"],
        json!({"code": "credential_rejected", "message": "token request rejected: Invalid client_secret"})
    );
    assert_eq!(mock.request_count().await, 0);
}

#[tokio::test]
async fn a_minted_token_echoed_by_the_server_is_scrubbed() {
    let (mock, host, _manager) = cc_ready_with("tok-echo-1").await;
    mock.on_call_error("echo", -32000, "bad token tok-echo-1")
        .await;
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        !error.contains("tok-echo-1") && error.contains("[redacted]"),
        "{error}"
    );
}

#[tokio::test]
async fn the_token_post_is_not_followed_to_another_origin() {
    rustls();
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/oauth/token", second.uri())),
        )
        .mount(&first)
        .await;
    let url = format!("{}/oauth/token", first.uri());
    let body = token_request_body("app-1", "shh-secret", None);
    let err = mint(&http_client(), &url, body).await.unwrap_err();
    assert_eq!(err.message, "token request rejected: HTTP 307");
    assert!(second.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_401_at_connect_mints_once_more_then_is_auth_failed() {
    let mock = echo_mock().await;
    let host = FakeHost::new();
    host.set_secret("secret://APP_SECRET", "shh");
    let token_url = mock.token_endpoint("tok-1").await;
    mock.require_bearer("tok-2").await;
    let manager = connect(&host, cc(&mock, &token_url)).await;
    let row = settle(&manager, State::AuthFailed).await;
    assert_eq!(
        row["error"],
        json!({"code": "credential_rejected", "message": SECRET_REJECTED})
    );
    assert_eq!(mock.token_requests().await.len(), 2);
    assert_eq!(mock.requests("initialize").await.len(), 2);
}

#[tokio::test]
async fn an_unapproved_entry_mints_nothing_until_approved() {
    let mock = echo_mock().await;
    let host = FakeHost::new();
    host.set_secret("secret://APP_SECRET", "shh");
    let token_url = mock.token_endpoint("tok-1").await;
    mock.require_bearer("tok-1").await;
    let manager = connect_unapproved(&host, cc(&mock, &token_url)).await;
    let row = settle(&manager, State::Unapproved).await;
    assert_eq!(row["stage"], "approval", "{row}");
    assert!(mock.token_requests().await.is_empty());
    assert_eq!(host.resolve_count("secret://APP_SECRET"), 0);
    manager.approve("srv", None).await.unwrap();
    settle(&manager, State::Ready).await;
    assert_eq!(mock.token_requests().await.len(), 1);
}
