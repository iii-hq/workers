mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mcp_client::server::{Backoff, Manager, State, retry_after_secs};
use serde_json::{Value, json};
use support::mcp_mock::McpMock;
use support::{
    ECHO, FakeHost, WAIT, bearer, connect, eventually, fast_timings, hi, server, tool, unregistered,
};

/// A mock with the tool `echo` answering `hi()`, a host holding
/// `secret://K`, and `srv` at `ready`.
async fn ready() -> (McpMock, Arc<FakeHost>, Arc<Manager>) {
    ready_with(bearer).await
}

/// The config with one http server `srv` and no credential.
fn plain(url: &str) -> Value {
    json!({"mcpServers": {"srv": {"type": "http", "url": url}}})
}

async fn ready_with(config: fn(&str) -> Value) -> (McpMock, Arc<FakeHost>, Arc<Manager>) {
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    mock.on_call("echo", hi()).await;
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    let manager = connect(&host, config(&mock.url())).await;
    settle(&manager, State::Ready).await;
    (mock, host, manager)
}

async fn settle(manager: &Arc<Manager>, state: State) -> Value {
    let reached = manager.wait_for("srv", state, WAIT).await;
    let row = server(manager, "srv").await;
    assert!(reached, "{row}");
    row
}

#[test]
fn backoff_doubles_from_min_to_max_and_resets() {
    let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    let waits: Vec<u64> = (0..8).map(|_| backoff.next().as_secs()).collect();
    assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60]);
    backoff.reset();
    assert_eq!(backoff.next(), Duration::from_secs(1));
}

#[test]
fn retry_after_is_whole_seconds_only() {
    assert_eq!(retry_after_secs(Some("7")), Some(Duration::from_secs(7)));
    assert_eq!(retry_after_secs(Some(" 7 ")), Some(Duration::from_secs(7)));
    for value in ["Wed, 21 Oct 2026 07:28:00 GMT", "1.5", "-1"] {
        assert_eq!(retry_after_secs(Some(value)), None, "{value}");
    }
    assert_eq!(retry_after_secs(None), None);
}

#[tokio::test]
async fn rate_limited_connect_waits_for_retry_after() {
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    mock.http_status("initialize", 429, &[("retry-after", "1")])
        .await;
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    let manager = connect(&host, bearer(&mock.url())).await;
    let row = settle(&manager, State::Unavailable).await;
    assert_eq!(row["error"]["code"], "rate_limited", "{row}");
    assert_eq!(row["stage"], "connect", "{row}");
    // The test backoff starts at 20 ms; the header says one second.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(mock.requests("initialize").await.len(), 1);
    mock.clear_http_status().await;
    settle(&manager, State::Ready).await;
}

#[tokio::test]
async fn outage_keeps_every_function_and_recovers_alone() {
    let (mock, host, manager) = ready().await;
    let ops = host.ops().len();
    mock.http_status("initialize", 503, &[]).await;
    mock.http_status("tools/call", 503, &[]).await;
    assert!(host.call(ECHO, json!({})).await.is_err());
    settle(&manager, State::Unavailable).await;
    let down = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        down.starts_with("`srv` MCP unavailable (`unavailable`), for ") && down.ends_with(" s"),
        "{down}"
    );
    assert_eq!(
        mock.requests("tools/call").await.len(),
        1,
        "no call is sent while down"
    );
    mock.clear_http_status().await;
    settle(&manager, State::Ready).await;
    assert_eq!(host.call(ECHO, json!({})).await, Ok(hi()));
    assert_eq!(
        host.ops().len(),
        ops,
        "an outage registers and unregisters nothing"
    );
}

#[tokio::test]
async fn secrets_worker_down_retries_instead_of_config_error() {
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    mock.http_status("initialize", 503, &[]).await;
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Unavailable).await;
    assert!(host.registered_ids().is_empty());
    eventually("a failed connect is tried again and again", || async {
        mock.requests("initialize").await.len() >= 3
    })
    .await;
    // The 503 stays until the stage is seen. Cleared here, an attempt that
    // resolved its secret just before `set_unreachable` could reach `ready`,
    // and no later attempt would ever stop at `secret`.
    host.set_unreachable(true);
    eventually("the retry stops at the secret stage", || async {
        let view = server(&manager, "srv").await;
        view["stage"] == "secret" && view["error"]["code"] == "connect_failed"
    })
    .await;
    assert_eq!(server(&manager, "srv").await["state"], "unavailable");
    mock.clear_http_status().await;
    // No config event and no secret event: the retry alone recovers.
    host.set_unreachable(false);
    settle(&manager, State::Ready).await;
    assert_eq!(host.registered_ids(), [ECHO]);
}

#[tokio::test]
async fn rate_limit_and_missing_scope_fail_one_call_and_stay_ready() {
    let (mock, host, manager) = ready().await;
    mock.http_status("tools/call", 429, &[("retry-after", "17")])
        .await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "rate limited by srv; retry after 17"
    );
    assert_eq!(
        mock.requests("tools/call").await.len(),
        1,
        "a 429 is not retried"
    );
    assert_eq!(server(&manager, "srv").await["state"], "ready");
    mock.http_status(
        "tools/call",
        403,
        &[(
            "www-authenticate",
            r#"Bearer error="insufficient_scope", scope="write""#,
        )],
    )
    .await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "insufficient scope: srv requires scope `write`"
    );
    assert_eq!(server(&manager, "srv").await["state"], "ready");
    assert!(unregistered(&host).is_empty());
}

#[tokio::test]
async fn rejected_credential_on_a_call_sets_auth_failed_and_keeps_functions() {
    let (mock, host, manager) = ready().await;
    mock.http_status(
        "tools/call",
        401,
        &[("www-authenticate", r#"Bearer error="invalid_token""#)],
    )
    .await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "credential rejected, an operator must fix `secret://K`"
    );
    let row = settle(&manager, State::AuthFailed).await;
    assert_eq!(row["error"]["code"], "credential_rejected", "{row}");
    let down = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        down.starts_with("`srv` MCP unavailable (`auth_failed`), for "),
        "{down}"
    );
    tokio::time::sleep(fast_timings().backoff_max * 2).await;
    assert_eq!(
        mock.requests("initialize").await.len(),
        1,
        "auth_failed is not retried"
    );
    assert!(unregistered(&host).is_empty());
}

/// Answers `method` with `status`, no `WWW-Authenticate` and a body:
/// McpMock's own answers have an empty body.
async fn answer_with_a_body(mock: &McpMock, method: &str, status: u16) {
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "method": method }),
        ))
        .respond_with(wiremock::ResponseTemplate::new(status).set_body_string("BODY-SENTINEL"))
        .with_priority(1)
        .mount(mock.server())
        .await;
}

#[tokio::test]
async fn a_401_without_a_challenge_on_a_call_never_shows_its_body() {
    let (mock, host, manager) = ready().await;
    answer_with_a_body(&mock, "tools/call", 401).await;
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert_eq!(
        error,
        "credential rejected, an operator must fix `secret://K`"
    );
    let row = settle(&manager, State::AuthFailed).await;
    assert!(!row.to_string().contains("BODY-SENTINEL"), "{row}");
}

#[tokio::test]
async fn a_403_without_a_challenge_on_a_call_stays_ready_and_never_shows_its_body() {
    let (mock, host, manager) = ready().await;
    answer_with_a_body(&mock, "tools/call", 403).await;
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert_eq!(error, "insufficient scope: srv refused the call");
    let listed = serde_json::to_string(&manager.list().await).unwrap();
    assert!(!listed.contains("BODY-SENTINEL"), "{listed}");
    assert_eq!(server(&manager, "srv").await["state"], "ready");
    assert!(unregistered(&host).is_empty());
}

#[tokio::test]
async fn a_4xx_on_a_call_fails_that_call_only_and_never_shows_its_body() {
    let (mock, host, manager) = ready().await;
    answer_with_a_body(&mock, "tools/call", 400).await;
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert!(
        error.contains("HTTP 400") && !error.contains("BODY-SENTINEL"),
        "{error}"
    );
    let listed = serde_json::to_string(&manager.list().await).unwrap();
    assert!(!listed.contains("BODY-SENTINEL"), "{listed}");
    tokio::time::sleep(fast_timings().backoff_min * 4).await;
    assert_eq!(server(&manager, "srv").await["state"], "ready");
    assert_eq!(mock.requests("initialize").await.len(), 1, "no reconnect");
}

#[tokio::test]
async fn a_challenged_401_on_a_call_without_a_credential_needs_login() {
    let (mock, host, manager) = ready_with(plain).await;
    let challenge =
        r#"Bearer resource_metadata="http://127.0.0.1/.well-known/oauth-protected-resource""#;
    mock.http_status("tools/call", 401, &[("www-authenticate", challenge)])
        .await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "sign-in required, an operator must sign in on the MCP servers page"
    );
    let row = settle(&manager, State::NeedsLogin).await;
    assert_eq!(row["error"]["code"], "login_required", "{row}");
    assert_eq!(row["auth"], "oauth", "{row}");
    assert!(unregistered(&host).is_empty());
}

#[tokio::test]
async fn a_bare_401_on_a_call_without_a_credential_is_auth_failed() {
    let (mock, host, manager) = ready_with(plain).await;
    answer_with_a_body(&mock, "tools/call", 401).await;
    let error = host.call(ECHO, json!({})).await.unwrap_err();
    assert_eq!(error, "srv answered 401 and offers no sign-in");
    let row = settle(&manager, State::AuthFailed).await;
    assert_eq!(
        row["error"],
        json!({"code": "credential_rejected", "message": "srv answered 401 and offers no sign-in"})
    );
    assert_eq!(row["auth"], "none", "{row}");
    assert!(unregistered(&host).is_empty());
}

/// Everything this crate logs, captured on the test's own thread (the
/// current-thread runtime runs the server task there too).
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
async fn a_failed_relist_never_logs_a_body() {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter("mcp_client=trace")
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (mock, host, manager) = ready().await;
    mock.on_call_error("echo", -32602, "team is required").await;
    answer_with_a_body(&mock, "tools/list", 400).await;
    assert!(host.call(ECHO, json!({})).await.is_err());
    eventually("the failed re-list is logged", || async {
        logs.text().contains("again failed")
    })
    .await;
    let text = logs.text();
    assert!(
        text.contains("HTTP 400") && !text.contains("BODY-SENTINEL"),
        "{text}"
    );
    assert_eq!(server(&manager, "srv").await["state"], "ready");
}

#[tokio::test]
async fn invalid_params_relists_at_most_once_per_interval() {
    let (mock, host, manager) = ready().await;
    mock.on_call_error("echo", -32602, "team is required").await;
    mock.tools(json!([tool("echo"), tool("extra")])).await;
    assert_eq!(
        host.call(ECHO, json!({})).await.unwrap_err(),
        "invalid_argument: team is required"
    );
    eventually("the re-list registered `extra`", || async {
        host.registered_ids()
            .contains(&"mcp::srv::extra".to_string())
    })
    .await;
    assert_eq!(mock.requests("tools/list").await.len(), 2);

    let interval = fast_timings().relist_min_interval;
    assert!(host.call(ECHO, json!({})).await.is_err());
    tokio::time::sleep(interval / 4).await;
    assert_eq!(
        mock.requests("tools/list").await.len(),
        2,
        "one re-list per interval"
    );
    tokio::time::sleep(interval).await;
    assert!(host.call(ECHO, json!({})).await.is_err());
    eventually("a re-list after the interval", || async {
        mock.requests("tools/list").await.len() == 3
    })
    .await;
    assert_eq!(server(&manager, "srv").await["state"], "ready");
}

#[tokio::test]
async fn tools_list_changed_adds_and_removes_functions() {
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo"), tool("old")])).await;
    let host = FakeHost::new();
    host.set_secret("secret://K", "k1");
    let manager = connect(&host, bearer(&mock.url())).await;
    settle(&manager, State::Ready).await;

    mock.tools(json!([tool("echo"), tool("new")])).await;
    mock.on_call_notifying("echo", hi(), "notifications/tools/list_changed")
        .await;
    assert_eq!(host.call(ECHO, json!({})).await.unwrap(), hi());
    eventually("`new` is registered", || async {
        host.registered_ids().contains(&"mcp::srv::new".to_string())
    })
    .await;
    // The only unregister this task may cause: a tool the list dropped.
    assert_eq!(unregistered(&host), ["mcp::srv::old"]);
}
