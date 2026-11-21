mod support;

use std::sync::Arc;
use std::time::Duration;

use mcp_client::config::RawConfig;
use mcp_client::functions::SecretChange;
use mcp_client::host::EventKind;
use mcp_client::server::{Manager, State};
use serde_json::{Value, json};
use support::as_mock::{
    AsMock, age_stored_token, challenged, oauth, query, refreshes, sign_in, stored, with,
};
use support::mcp_mock::echo_mock;
use support::{
    ECHO, FakeHost, WAIT, connect, connect_unapproved, eventually, hi, server, unregistered,
};

const NONE_WAITING: &str = "no sign-in is waiting for this address; start again";
const SIGN_IN: &str = "sign-in required, an operator must sign in on the MCP servers page";
const ISS_REFUSED: &str =
    "sign-in refused: the answer did not come from the expected authorization server";
/// How long the mock holds back a refresh answer in the race tests.
const HOLD: Duration = Duration::from_millis(400);

async fn settle(m: &Arc<Manager>, name: &str, state: State) -> Value {
    let reached = m.wait_for(name, state, WAIT).await;
    let row = server(m, name).await;
    assert!(reached, "{row}");
    row
}

fn refresh_token(host: &FakeHost) -> String {
    let saved = stored(host);
    saved["credentials"]["token_response"]["refresh_token"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Starts a call whose refresh the mock holds back by `HOLD`; returns once
/// that refresh reached the mock.
async fn call_mid_refresh(
    host: &Arc<FakeHost>,
    as_mock: &AsMock,
) -> tokio::task::JoinHandle<Result<Value, String>> {
    as_mock.hold_refresh(HOLD).await;
    age_stored_token(host);
    let h = host.clone();
    let call = tokio::spawn(async move { h.call(ECHO, json!({})).await });
    eventually("the refresh reached the mock", || async {
        refreshes(&as_mock.token_requests().await) == 1
    })
    .await;
    call
}

#[tokio::test]
async fn a_challenge_asks_for_sign_in_and_starts_nothing() {
    let (_mock, as_mock, _host, m) = challenged().await;
    let row = server(&m, "srv").await;
    assert_eq!(row["auth"], "oauth", "{row}");
    assert_eq!(row["error"]["code"], "login_required", "{row}");
    assert_eq!(row["login"], Value::Null, "{row}");
    assert_eq!(as_mock.registrations().await, 0);
}

#[tokio::test]
async fn the_full_loop_through_login_complete() {
    let (mock, as_mock, host, m) = challenged().await;
    host.set_http_port(Some(4321));
    let start = m.login_start("srv").await.unwrap();
    assert_eq!(
        start.redirect_uri,
        "http://127.0.0.1:4321/mcp-client/oauth/callback"
    );
    let asked = query(&start.authorize_url);
    assert_eq!(asked["code_challenge_method"], "S256");
    assert_eq!(asked["redirect_uri"], start.redirect_uri);
    assert_eq!(asked["resource"], mock.url());
    assert_eq!(asked["client_id"], "mock-client");
    assert_eq!(
        server(&m, "srv").await["login"],
        json!({"pending": true, "expires_at": start.expires_at})
    );

    let good = as_mock.authorize_redirect(&start.authorize_url);
    assert_eq!(
        m.login_complete(&good).await.unwrap(),
        ("srv".to_string(), State::Ready)
    );
    assert_eq!(as_mock.registrations().await, 1);
    let bodies = as_mock.token_requests().await;
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    assert!(
        bodies[0].contains("grant_type=authorization_code") && bodies[0].contains("code_verifier="),
        "{}",
        bodies[0]
    );
    let saved = stored(&host);
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["resource"], mock.url());
    assert_eq!(saved["credentials"]["client_id"], "mock-client");
    assert_eq!(server(&m, "srv").await["login"], Value::Null);
    assert_eq!(host.call(ECHO, json!({})).await, Ok(hi()));
    let sent = mock.headers_of("tools/call").await;
    assert!(
        sent.last().unwrap()["authorization"].starts_with("Bearer at-1-"),
        "{sent:?}"
    );
    assert!(
        host.events()
            .iter()
            .any(|e| e.server == "srv" && e.kind == EventKind::Login)
    );
}

#[tokio::test]
async fn a_wrong_state_consumes_nothing_and_a_replay_is_refused() {
    let (_mock, as_mock, _host, m) = challenged().await;
    let start = m.login_start("srv").await.unwrap();
    let good = as_mock.authorize_redirect(&start.authorize_url);
    let wrong = with(&good, "state", Some("not-the-state"));
    assert_eq!(m.login_complete(&wrong).await.unwrap_err(), NONE_WAITING);
    assert_eq!(
        m.login_complete(&with(&good, "state", None))
            .await
            .unwrap_err(),
        "this address holds no sign-in state; paste the whole address the browser ended on"
    );
    assert!(as_mock.token_requests().await.is_empty());
    assert_eq!(server(&m, "srv").await["login"]["pending"], true);

    assert_eq!(
        m.login_complete(&good).await.unwrap(),
        ("srv".to_string(), State::Ready)
    );
    assert_eq!(m.login_complete(&good).await.unwrap_err(), NONE_WAITING);
    assert_eq!(as_mock.token_requests().await.len(), 1);
    assert_eq!(server(&m, "srv").await["state"], "ready");
}

#[tokio::test]
async fn a_wrong_or_missing_iss_is_refused() {
    let (_mock, as_mock, host, m) = challenged().await;
    for iss in [Some("https://evil.example"), None] {
        let start = m.login_start("srv").await.unwrap();
        let good = as_mock.authorize_redirect(&start.authorize_url);
        let answer = m.login_complete(&with(&good, "iss", iss)).await;
        assert_eq!(answer.unwrap_err(), ISS_REFUSED, "{iss:?}");
    }
    assert!(as_mock.token_requests().await.is_empty());
    assert_eq!(host.vault("MCP_CLIENT_SRV_OAUTH"), None);
    assert_eq!(server(&m, "srv").await["state"], "needs_login");
}

#[tokio::test]
async fn the_server_is_found_from_the_state_not_from_the_caller() {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    let entry = json!({"type": "http", "url": mock.url()});
    let m = connect(&host, json!({"mcpServers": {"a": entry, "b": entry}})).await;
    settle(&m, "a", State::NeedsLogin).await;
    settle(&m, "b", State::NeedsLogin).await;
    let a = m.login_start("a").await.unwrap();
    let b = m.login_start("b").await.unwrap();
    let done = m
        .login_complete(&as_mock.authorize_redirect(&b.authorize_url))
        .await;
    assert_eq!(done.unwrap(), ("b".to_string(), State::Ready));
    let row = server(&m, "a").await;
    assert_eq!(row["state"], "needs_login", "{row}");
    assert_eq!(row["login"]["expires_at"], a.expires_at, "{row}");
}

#[tokio::test]
async fn cancel_drops_the_pending_login() {
    let (_mock, as_mock, _host, m) = challenged().await;
    let start = m.login_start("srv").await.unwrap();
    let good = as_mock.authorize_redirect(&start.authorize_url);
    assert_eq!(m.login_cancel("srv").await, Ok(()));
    assert_eq!(m.login_cancel("srv").await, Ok(()), "nothing pending");
    assert_eq!(server(&m, "srv").await["login"], Value::Null);
    assert_eq!(m.login_complete(&good).await.unwrap_err(), NONE_WAITING);
    assert_eq!(
        m.login_cancel("nope").await.unwrap_err(),
        "no server named `nope`"
    );
}

#[tokio::test]
async fn an_expired_token_is_refreshed_once_for_two_concurrent_calls() {
    let (mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    age_stored_token(&host);
    let (one, two) = tokio::join!(host.call(ECHO, json!({})), host.call(ECHO, json!({})));
    assert_eq!((one, two), (Ok(hi()), Ok(hi())));
    assert_eq!(refreshes(&as_mock.token_requests().await), 1);
    assert!(refresh_token(&host).starts_with("rt-2-"));
    for sent in mock.headers_of("tools/call").await {
        assert!(
            sent["authorization"].starts_with("Bearer at-2-"),
            "{sent:?}"
        );
    }
}

#[tokio::test]
async fn a_token_the_server_rejects_is_refreshed_and_the_call_retried() {
    let (_mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    as_mock.expire_access_token().await;
    assert_eq!(host.call(ECHO, json!({})).await, Ok(hi()));
    assert_eq!(refreshes(&as_mock.token_requests().await), 1);
    assert_eq!(server(&m, "srv").await["state"], "ready");
}

#[tokio::test]
async fn a_rejected_refresh_token_needs_login_and_keeps_the_functions() {
    let (mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    as_mock.reject_refresh().await;
    age_stored_token(&host);
    assert_eq!(host.call(ECHO, json!({})).await.unwrap_err(), SIGN_IN);
    assert_eq!(server(&m, "srv").await["state"], "needs_login");
    assert_eq!(stored(&host)["credentials"], Value::Null);
    assert!(host.registered_ids().contains(&ECHO.to_string()));
    assert!(unregistered(&host).is_empty());
    // Counted on `tools/call`: the closing session's DELETE may land at any time.
    let calls = mock.requests("tools/call").await.len();
    assert_eq!(host.call(ECHO, json!({})).await.unwrap_err(), SIGN_IN);
    assert_eq!(
        mock.requests("tools/call").await.len(),
        calls,
        "answered by the worker"
    );

    sign_in(&m, &as_mock).await;
    assert_eq!(
        as_mock.registrations().await,
        2,
        "a new client: the old id went with the session"
    );
}

#[tokio::test]
async fn sign_out_clears_the_session_and_keeps_the_functions() {
    let (_mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    assert_eq!(m.sign_out("srv").await, Ok(State::NeedsLogin));
    assert_eq!(stored(&host)["credentials"], Value::Null);
    assert_eq!(host.call(ECHO, json!({})).await.unwrap_err(), SIGN_IN);
    assert!(host.registered_ids().contains(&ECHO.to_string()));
}

#[tokio::test]
async fn a_stored_session_survives_a_restart() {
    let (mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    m.shutdown().await;
    let m = connect(&host, support::as_mock::open(&mock)).await;
    settle(&m, "srv", State::Ready).await;
    assert_eq!(as_mock.registrations().await, 1);
    assert_eq!(as_mock.token_requests().await.len(), 1);
}

#[tokio::test]
async fn an_oauth_entry_with_no_session_needs_login_without_a_request() {
    let mock = echo_mock().await;
    let host = FakeHost::new();
    let m = connect(&host, oauth(&mock, json!({"type": "oauth"}))).await;
    let row = settle(&m, "srv", State::NeedsLogin).await;
    assert_eq!(row["stage"], "authorize", "{row}");
    assert_eq!(mock.request_count().await, 0);
}

#[tokio::test]
async fn a_configured_client_id_skips_registration() {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    host.set_secret("secret://APP", "shh");
    let auth = json!({"type": "oauth", "client_id": "pre-1", "client_secret": "secret://APP",
        "scopes": ["read"]});
    let m = connect(&host, oauth(&mock, auth)).await;
    settle(&m, "srv", State::NeedsLogin).await;
    let start = m.login_start("srv").await.unwrap();
    let asked = query(&start.authorize_url);
    assert_eq!(asked["client_id"], "pre-1");
    assert!(
        asked["scope"].split(' ').any(|s| s == "read"),
        "{}",
        asked["scope"]
    );
    assert_eq!(as_mock.registrations().await, 0);
}

#[tokio::test]
async fn removing_the_entry_clears_the_session_and_disabling_does_not() {
    let (mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    let disabled =
        json!({"mcpServers": {"srv": {"type": "http", "url": mock.url(), "disabled": true}}});
    m.apply_config(RawConfig::from_value(&disabled)).await;
    assert_eq!(server(&m, "srv").await["state"], "disabled");
    assert_ne!(stored(&host)["credentials"], Value::Null);
    m.apply_config(RawConfig::from_value(&json!({"mcpServers": {}})))
        .await;
    assert_eq!(stored(&host)["credentials"], Value::Null);
}

#[tokio::test]
async fn sign_in_is_refused_where_it_does_not_apply() {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    host.set_secret("secret://KEY", "k1");
    let config = json!({"mcpServers": {
        "key": {"type": "http", "url": mock.url(), "auth": {"type": "bearer", "token": "secret://KEY"}},
        "raw": {"type": "http", "url": mock.url()}
    }});
    let m = connect_unapproved(&host, config).await;
    settle(&m, "raw", State::Unapproved).await;
    let not_sign_in = "`key` does not use sign-in";
    assert_eq!(m.login_start("key").await.unwrap_err(), not_sign_in);
    assert_eq!(m.sign_out("key").await.unwrap_err(), not_sign_in);
    assert_eq!(
        m.login_start("nope").await.unwrap_err(),
        "no server named `nope`"
    );
    assert_eq!(
        m.login_start("raw").await.unwrap_err(),
        "approve `raw` before signing in"
    );
    assert_eq!(mock.request_count().await, 0);
    assert_eq!(as_mock.registrations().await, 0);

    let free = echo_mock().await;
    let other = FakeHost::new();
    let m = connect(
        &other,
        json!({"mcpServers": {"free": {"type": "http", "url": free.url()}}}),
    )
    .await;
    settle(&m, "free", State::Ready).await;
    assert_eq!(
        m.login_start("free").await.unwrap_err(),
        "`free` did not ask for sign-in"
    );
}

#[tokio::test]
async fn a_session_write_does_not_reconnect_the_server() {
    let (mock, as_mock, _host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    let before = mock.requests("initialize").await.len();
    m.on_secret_change(&SecretChange {
        name: Some("MCP_CLIENT_SRV_OAUTH".into()),
        reference: Some("secret://MCP_CLIENT_SRV_OAUTH".into()),
        action: Some("set".into()),
    })
    .await;
    assert_eq!(mock.requests("initialize").await.len(), before);
    assert_eq!(server(&m, "srv").await["state"], "ready");
}

#[tokio::test]
async fn a_session_does_not_follow_a_secret_url_to_another_host() {
    let first = echo_mock().await;
    let as_first = AsMock::start(&first).await;
    let second = echo_mock().await;
    let _as_second = AsMock::start(&second).await;
    let host = FakeHost::new();
    host.set_secret("secret://U", &first.url());
    let entry = json!({"mcpServers": {"srv": {"type": "http", "url": "secret://U"}}});
    let m = connect(&host, entry).await;
    settle(&m, "srv", State::NeedsLogin).await;
    sign_in(&m, &as_first).await;

    host.set_secret("secret://U", &second.url());
    m.on_secret_change(&SecretChange {
        name: Some("U".into()),
        reference: Some("secret://U".into()),
        action: Some("set".into()),
    })
    .await;
    settle(&m, "srv", State::NeedsLogin).await;
    let received = second.server().received_requests().await.unwrap();
    assert!(!received.is_empty(), "the second host was asked");
    for request in received {
        let sent = request.headers.get("authorization");
        assert!(sent.is_none(), "{} carried {sent:?}", request.url);
    }
}

#[tokio::test]
async fn an_unapproved_sign_in_entry_waits_for_approval_and_starts_nothing() {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    let m = connect_unapproved(&host, oauth(&mock, json!({"type": "oauth"}))).await;
    let row = settle(&m, "srv", State::Unapproved).await;
    assert_eq!(row["stage"], "approval", "{row}");
    assert_eq!(
        m.login_start("srv").await.unwrap_err(),
        "approve `srv` before signing in"
    );
    m.approve("srv", None).await.unwrap();
    let row = settle(&m, "srv", State::NeedsLogin).await;
    assert_eq!(row["stage"], "authorize", "{row}");
    assert_eq!(mock.request_count().await, 0);
    assert_eq!(as_mock.registrations().await, 0);
}

#[tokio::test]
async fn a_401_on_an_oauth_entry_is_sign_in_even_with_a_secret_header() {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    host.set_secret("secret://HK", "hk-1");
    let config = json!({"mcpServers": {"srv": {"type": "http", "url": mock.url(),
        "headers": {"X-Key": "secret://HK"}, "auth": {"type": "oauth"}}}});
    let m = connect(&host, config).await;
    settle(&m, "srv", State::NeedsLogin).await;
    sign_in(&m, &as_mock).await;
    let before = mock.requests("initialize").await.len();
    let challenge = format!(
        r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource""#,
        mock.server().uri()
    );
    mock.http_status("tools/call", 401, &[("www-authenticate", &challenge)])
        .await;
    assert_eq!(host.call(ECHO, json!({})).await.unwrap_err(), SIGN_IN);
    assert_eq!(server(&m, "srv").await["state"], "needs_login");
    assert_eq!(
        mock.requests("initialize").await.len(),
        before,
        "no credential recovery"
    );
}

#[tokio::test]
async fn the_code_exchange_waits_for_a_refresh_in_flight() {
    let (_mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    let start = m.login_start("srv").await.unwrap();
    let good = as_mock.authorize_redirect(&start.authorize_url);
    let call = call_mid_refresh(&host, &as_mock).await;
    m.login_complete(&good).await.unwrap();
    let _ = call.await;
    let log = as_mock.token_log().await;
    let grant = |g: &str| -> Vec<std::time::Instant> {
        log.iter()
            .filter(|(_, b)| b.contains(g))
            .map(|(t, _)| *t)
            .collect()
    };
    let refresh_at = grant("grant_type=refresh_token")[0];
    let code_at = grant("grant_type=authorization_code")[1];
    assert!(
        code_at >= refresh_at + HOLD,
        "the code went out while the refresh was in flight"
    );
    assert!(
        !refresh_token(&host).starts_with("rt-2-"),
        "the refreshed old session replaced the sign-in"
    );
}

#[tokio::test]
async fn sign_out_during_a_refresh_in_flight_clears_the_session() {
    let (_mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    let call = call_mid_refresh(&host, &as_mock).await;
    let out = tokio::time::timeout(WAIT, m.sign_out("srv")).await;
    assert_eq!(out.expect("sign-out finished"), Ok(State::NeedsLogin));
    let _ = call.await;
    assert_eq!(stored(&host)["credentials"], Value::Null);
}

#[tokio::test]
async fn removing_an_entry_during_a_refresh_in_flight_clears_the_session() {
    let (_mock, as_mock, host, m) = challenged().await;
    sign_in(&m, &as_mock).await;
    let call = call_mid_refresh(&host, &as_mock).await;
    let empty = RawConfig::from_value(&json!({"mcpServers": {}}));
    let removed = tokio::time::timeout(WAIT, m.apply_config(empty)).await;
    removed.expect("the removal finished");
    let _ = call.await;
    assert_eq!(stored(&host)["credentials"], Value::Null);
}

#[tokio::test]
async fn a_configured_client_secret_survives_a_reconnect() {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    host.set_secret("secret://APP", "shh");
    let auth = json!({"type": "oauth", "client_id": "pre-1", "client_secret": "secret://APP"});
    let m = connect(&host, oauth(&mock, auth)).await;
    settle(&m, "srv", State::NeedsLogin).await;
    sign_in(&m, &as_mock).await;
    m.reconnect("srv").await.unwrap();
    settle(&m, "srv", State::Ready).await;
    age_stored_token(&host);
    assert_eq!(host.call(ECHO, json!({})).await, Ok(hi()));
    let sent = as_mock.token_auth().await;
    let of = |grant: &str| {
        let found = sent.iter().find(|(body, _)| body.contains(grant));
        found.expect(grant).1.clone()
    };
    let code = of("grant_type=authorization_code");
    assert!(
        code.as_deref().is_some_and(|a| a.starts_with("Basic ")),
        "{code:?}"
    );
    assert_eq!(
        of("grant_type=refresh_token"),
        code,
        "the refresh lost the client secret"
    );
    assert_eq!(server(&m, "srv").await["state"], "ready");
}

#[tokio::test]
async fn a_sign_in_finished_after_its_removal_leaves_no_session() {
    let (_mock, as_mock, host, m) = challenged().await;
    as_mock.hold_metadata(HOLD).await;
    let started = m.clone();
    let start = tokio::spawn(async move { started.login_start("srv").await });
    eventually("discovery reached the mock", || async {
        as_mock.metadata_gets().await > 0
    })
    .await;
    m.apply_config(RawConfig::from_value(&json!({"mcpServers": {}})))
        .await;
    let start = start.await.unwrap().unwrap();
    let good = as_mock.authorize_redirect(&start.authorize_url);
    assert_eq!(
        m.login_complete(&good).await.unwrap_err(),
        "no server named `srv`"
    );
    assert_eq!(as_mock.token_requests().await.len(), 1, "the exchange ran");
    assert_eq!(stored(&host)["credentials"], Value::Null);
}

#[tokio::test]
async fn a_tampered_approval_list_drops_the_pending_login() {
    let (_mock, as_mock, host, m) = challenged().await;
    let start = m.login_start("srv").await.unwrap();
    let good = as_mock.authorize_redirect(&start.authorize_url);
    host.set_secret(
        "secret://MCP_CLIENT_APPROVALS",
        r#"{"approved":{},"version":1}"#,
    );
    m.on_secret_change(&SecretChange {
        name: Some("MCP_CLIENT_APPROVALS".into()),
        reference: Some("secret://MCP_CLIENT_APPROVALS".into()),
        action: Some("set".into()),
    })
    .await;
    let row = settle(&m, "srv", State::Unapproved).await;
    assert_eq!(row["login"], Value::Null, "{row}");
    assert_eq!(m.login_complete(&good).await.unwrap_err(), NONE_WAITING);
    assert!(as_mock.token_requests().await.is_empty());
}
