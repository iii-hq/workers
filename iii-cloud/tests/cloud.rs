//! The cloud client and `connect`, against wiremock.

mod support;

use std::sync::Arc;

use iii_cloud::cloud::Req;
use iii_cloud::connect::{connect, device_name};
use iii_cloud::types::*;
use serde_json::json;
use support::{Fake, harness};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str =
    "iii_0123456789abcdef_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const TEAM: &str = "11111111-1111-4111-8111-111111111111";
const ENV: &str = "22222222-2222-4222-8222-222222222222";

fn env_body() -> serde_json::Value {
    json!({"id": ENV, "project_id": "p", "name": "production", "production": true, "branch": "main", "manifest": "worker-compose.yaml", "url": null, "settings": {}})
}

async fn key_connected(server: &MockServer) -> support::Harness {
    Mock::given(method("GET"))
        .and(path("/api/v1/keys/self"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"team_id": TEAM, "environment_id": ENV, "project_id": "p", "permissions": ["read", "deploy"]})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(env_body()))
        .mount(server)
        .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("MY_KEY", KEY),
    );
    let conn = connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: None,
            environment_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(conn.status, ConnStatus::Connected);
    h
}

#[tokio::test]
async fn a_key_connects_with_the_secret_name_alone_and_the_cloud_tells_the_ids() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    let conn = h.ctx.conn().await;
    let key = conn.api_key.unwrap();
    assert_eq!(
        (
            key.team_id.as_str(),
            key.environment_id.as_str(),
            key.secret.as_str()
        ),
        (TEAM, ENV, "MY_KEY")
    );
    assert_eq!(conn.method, Some(Method::ApiKey));
    // The key is in no record: the store holds the secret's name only.
    let state = std::fs::read_to_string(h.ctx.store.dir().join("state.json")).unwrap();
    assert!(!state.contains(KEY));
    assert_eq!(h.fake.kinds(), vec!["connection:true"]);
}

#[tokio::test]
async fn an_older_cloud_without_keys_self_falls_back_to_the_ids_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys/self"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"code": "NOT_FOUND", "message": "no"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(env_body()))
        .mount(&server)
        .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("MY_KEY", KEY),
    );
    // No ids: the worker says what is missing.
    let err = connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: None,
            environment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "INVALID_REQUEST");
    let conn = connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: Some(TEAM.into()),
            environment_id: Some(ENV.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(conn.api_key.unwrap().environment_id, ENV);
}

#[tokio::test]
async fn an_older_cloud_that_refuses_the_route_with_a_401_also_falls_back() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys/self"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"code": "UNAUTHENTICATED", "message": "This API key can't call this route."}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(env_body()))
        .mount(&server)
        .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("MY_KEY", KEY),
    );
    let conn = connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: Some(TEAM.into()),
            environment_id: Some(ENV.into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(conn.status, ConnStatus::Connected);
}

#[tokio::test]
async fn a_rejected_key_and_a_wrong_environment_have_their_own_codes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys/self"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Bad key."})),
        )
        .mount(&server)
        .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("MY_KEY", KEY),
    );
    let err = connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: None,
            environment_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "KEY_REJECTED");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/keys/self"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"code": "NOT_FOUND", "message": "x"})),
        )
        .mount(&server)
        .await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("MY_KEY", KEY),
    );
    let err = connect(
        &h.ctx,
        ConnectRequest::ApiKey {
            secret: "MY_KEY".into(),
            team_id: Some(TEAM.into()),
            environment_id: Some(ENV.into()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "KEY_WRONG_ENVIRONMENT");
}

#[tokio::test]
async fn writes_carry_an_idempotency_key_and_if_match_and_the_auth_header() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/thing"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .and(header("if-match", "fp1"))
        .and(header("idempotency-key", "idem-1"))
        .and(body_json(json!({"a": 1})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let mut req = Req::post("/thing", json!({"a": 1}));
    req.if_match = Some("fp1".into());
    req.idempotency_key = Some("idem-1".into());
    assert_eq!(h.ctx.call(&req).await.unwrap(), json!({"ok": true}));
    // Without a key given, a v4 UUID is sent.
    Mock::given(method("POST"))
        .and(path("/api/v1/other"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    h.ctx.call(&Req::post("/other", json!({}))).await.unwrap();
    let sent = server.received_requests().await.unwrap();
    let other = sent
        .iter()
        .find(|r| r.url.path() == "/api/v1/other")
        .unwrap();
    let key = other
        .headers
        .get("idempotency-key")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(uuid_like(key), "{key}");
}

fn uuid_like(s: &str) -> bool {
    s.len() == 36 && s.matches('-').count() == 4
}

#[tokio::test]
async fn the_error_envelope_becomes_cloud_error_with_status_and_fix() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/boom"))
        .respond_with(ResponseTemplate::new(422).set_body_json(
            json!({"code": "VALIDATION_FAILED", "message": "Nope.", "fix": "Do this."}),
        ))
        .mount(&server)
        .await;
    let err = h.ctx.call(&Req::get("/boom")).await.unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    assert_eq!(err.status, Some(422));
    assert_eq!(err.fix.as_deref(), Some("Do this."));
    assert!(err.message.contains("VALIDATION_FAILED"));
}

#[tokio::test]
async fn a_401_on_a_key_expires_the_connection_and_a_route_refusal_does_not() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    Mock::given(method("PUT"))
        .and(path("/api/v1/session-only"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"code": "UNAUTHENTICATED", "message": "This API key can't call this route."}),
        ))
        .mount(&server)
        .await;
    let mut put = Req::post("/session-only", json!({}));
    put.verb = Some(reqwest::Method::PUT);
    let err = h.ctx.call(&put).await.unwrap_err();
    assert_eq!(err.code, "NEEDS_BROWSER_SIGN_IN");
    assert_eq!(h.ctx.conn().await.status, ConnStatus::Connected);

    // The key is revoked: the route and the environment it was connected to both say 401.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV}")))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Revoked."})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/revoked"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Revoked."})),
        )
        .mount(&server)
        .await;
    let err = h.ctx.call(&Req::get("/revoked")).await.unwrap_err();
    assert_eq!(err.code, "KEY_REJECTED");
    let conn = h.ctx.conn().await;
    assert_eq!(conn.status, ConnStatus::Expired);
    assert_eq!(conn.error.unwrap().code, "KEY_REJECTED");
    assert_eq!(h.fake.kinds(), vec!["connection:true", "connection:true"]);
    // Later calls say not connected.
    assert_eq!(
        h.ctx.call(&Req::get("/revoked")).await.unwrap_err().code,
        "NOT_CONNECTED"
    );
}

#[tokio::test]
async fn a_401_on_a_session_expires_it_as_not_connected() {
    let server = MockServer::start().await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("IIICLOUD_SESSION", "iiia_tok"),
    );
    let mut conn = Connection::signed_out(&server.uri());
    conn.status = ConnStatus::Connected;
    conn.method = Some(Method::Browser);
    h.ctx.set_connection(conn);
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .and(header("authorization", "Bearer iiia_tok"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Expired."})),
        )
        .mount(&server)
        .await;
    let err = h.ctx.call(&Req::get("/me")).await.unwrap_err();
    assert_eq!(err.code, "NOT_CONNECTED");
    assert_eq!(h.ctx.conn().await.status, ConnStatus::Expired);
}

// -------------------------------------------------------------- device flow

async fn wait_for(h: &support::Harness, want: ConnStatus) -> Connection {
    for _ in 0..300 {
        let c = h.ctx.conn().await;
        if c.status == want {
            return c;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!(
        "connection never became {want:?}: {:?}",
        h.ctx.conn().await.error
    );
}

fn device_start() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"device_code": "dc", "user_code": "ABCD-1234", "verification_uri": "https://cloud.test/device", "verification_uri_complete": "https://cloud.test/device?code=ABCD-1234", "expires_in": 600, "interval": 5}))
}

#[tokio::test]
async fn the_browser_flow_waits_backs_off_and_stores_the_token_only_in_the_vault() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v1/auth/device")).and(body_json(json!({"name": device_name(&std::env::var("HOSTNAME").ok().or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok().map(|h| h.trim().to_string())).unwrap_or_else(|| "this machine".into())), "platform": "worker"}))).respond_with(device_start()).mount(&server).await;
    // pending, then a 429, then the token.
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"code": "AUTHORIZATION_PENDING", "message": "wait"})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(json!({"code": "RATE_LIMITED", "message": "slow"})),
        )
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/api/v1/auth/device/token")).and(body_json(json!({"device_code": "dc"}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "iiia_secrettoken", "session_id": "s", "expires_at": "2027-02-01T00:00:00Z", "user": {"id": "u1", "email": "ada@example.test", "name": "Ada"}}))).with_priority(3).mount(&server).await;
    Mock::given(method("GET")).and(path("/api/v1/me")).and(header("authorization", "Bearer iiia_secrettoken")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"user": {"id": "u1", "email": "ada@example.test", "name": "Ada"}, "teams": [{"id": TEAM, "name": "Acme", "slug": "acme", "personal": false, "role": "owner"}]}))).mount(&server).await;
    let fake = Arc::new(Fake::default()).with_secret("IIICLOUD_SESSION", "old-token");
    let h = harness(&server.uri(), fake);
    let waiting = connect(&h.ctx, ConnectRequest::Browser).await.unwrap();
    assert_eq!(waiting.status, ConnStatus::Waiting);
    let device = waiting.device.unwrap();
    assert_eq!(device.user_code, "ABCD-1234");
    // The worker never builds a link with the code in it.
    assert_eq!(device.verification_uri, "https://cloud.test/device");
    let conn = wait_for(&h, ConnStatus::Connected).await;
    assert_eq!(conn.user.unwrap().email, "ada@example.test");
    assert_eq!(conn.teams[0].slug, "acme");
    assert_eq!(conn.expires_at.as_deref(), Some("2027-02-01T00:00:00Z"));
    assert!(conn.device.is_none());
    // Compare-and-set replaced the old value.
    assert_eq!(
        h.fake
            .secrets
            .lock()
            .unwrap()
            .get("IIICLOUD_SESSION")
            .unwrap(),
        "iiia_secrettoken"
    );
    // No record, event or file carries the token.
    let state = std::fs::read_to_string(h.ctx.store.dir().join("state.json")).unwrap();
    let events = serde_json::to_string(&*h.fake.events.lock().unwrap()).unwrap();
    assert!(!state.contains("iiia_secrettoken") && !events.contains("iiia_secrettoken"));
    assert_eq!(h.fake.kinds(), vec!["connection:false", "connection:true"]);
}

#[tokio::test]
async fn an_expired_device_code_ends_the_wait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device"))
        .respond_with(device_start())
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"code": "DEVICE_CODE_EXPIRED", "message": "This sign-in code has expired."}),
        ))
        .mount(&server)
        .await;
    let h = harness(&server.uri(), Arc::new(Fake::default()));
    connect(&h.ctx, ConnectRequest::Browser).await.unwrap();
    let conn = wait_for(&h, ConnStatus::Error).await;
    assert_eq!(conn.error.unwrap().code, "DEVICE_CODE_EXPIRED");
}

#[tokio::test]
async fn a_cloud_without_worker_sign_in_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v1/auth/device")).respond_with(ResponseTemplate::new(422).set_body_json(json!({"code": "VALIDATION_FAILED", "message": "platform: must be ios, ipados, macos or android."}))).mount(&server).await;
    let h = harness(&server.uri(), Arc::new(Fake::default()));
    let err = connect(&h.ctx, ConnectRequest::Browser).await.unwrap_err();
    assert_eq!(err.code, "BROWSER_SIGN_IN_UNAVAILABLE");
    assert_eq!(h.ctx.conn().await.status, ConnStatus::SignedOut);
}

#[tokio::test]
async fn a_changed_cloud_url_signs_out() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    *h.ctx.cfg.write().await = Arc::new(iii_cloud::config::Config {
        cloud_url: "http://other.test".into(),
        data_dir: "x".into(),
        ..Default::default()
    });
    let conn = h.ctx.conn().await;
    assert_eq!(conn.status, ConnStatus::SignedOut);
    assert_eq!(
        h.ctx.call(&Req::get("/x")).await.unwrap_err().code,
        "NOT_CONNECTED"
    );
}

#[tokio::test]
async fn a_cookie_only_route_does_not_sign_a_valid_session_out() {
    let server = MockServer::start().await;
    let h = harness(
        &server.uri(),
        Arc::new(Fake::default()).with_secret("IIICLOUD_SESSION", "iiia_tok"),
    );
    let mut conn = Connection::signed_out(&server.uri());
    conn.status = ConnStatus::Connected;
    conn.method = Some(Method::Browser);
    h.ctx.set_connection(conn);
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"user": {"id": "u", "email": "e"}, "teams": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/alerts")).respond_with(ResponseTemplate::new(401).set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Sign in with a browser session to continue."}))).mount(&server).await;
    let err = h.ctx.call(&Req::get("/alerts")).await.unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.status),
        ("BROWSER_SESSION_ONLY", Some(401))
    );
    assert_eq!(
        h.ctx.conn().await.status,
        ConnStatus::Connected,
        "still signed in"
    );
    // When /me says 401 too, it is expired.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Expired."})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/alerts"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    assert_eq!(
        h.ctx.call(&Req::get("/alerts")).await.unwrap_err().code,
        "NOT_CONNECTED"
    );
    assert_eq!(h.ctx.conn().await.status, ConnStatus::Expired);
}

#[tokio::test]
async fn a_401_on_a_valid_key_that_is_not_a_route_refusal_keeps_the_connection() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/odd"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"code": "UNAUTHENTICATED", "message": "Nope."})),
        )
        .mount(&server)
        .await;
    let err = h.ctx.call(&Req::get("/odd")).await.unwrap_err();
    assert_eq!(err.status, Some(401));
    assert_ne!(err.code, "KEY_REJECTED");
    assert_eq!(h.ctx.conn().await.status, ConnStatus::Connected);
}

#[tokio::test]
async fn only_a_platform_422_means_no_worker_sign_in_yet() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v1/auth/device")).respond_with(ResponseTemplate::new(422).set_body_json(json!({"code": "VALIDATION_FAILED", "message": "name: no control or invisible formatting characters."}))).mount(&server).await;
    let h = harness(&server.uri(), Arc::new(Fake::default()));
    let err = connect(&h.ctx, ConnectRequest::Browser).await.unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    assert!(err.message.contains("VALIDATION_FAILED"));
}

#[test]
fn the_device_name_loses_format_characters_too() {
    let name = device_name("ho\u{200B}st\u{200F}\u{2060}\u{FEFF}x\u{AD}");
    assert_eq!(name, "iii-cloud worker on hostx");
    // Cut at 64 characters, control characters gone.
    let name = device_name(&format!("host\u{7}{}", "x".repeat(100)));
    assert_eq!(name.chars().count(), 64);
    assert!(!name.chars().any(char::is_control));
}

/// `api` only reads (owner decision, 26/11): every write is `PERSON_ONLY` and
/// reaches no route; `api::write` is the page's, and still refuses a secret value.
#[tokio::test]
async fn api_reads_only_and_api_write_is_the_pages() {
    let server = MockServer::start().await;
    let h = key_connected(&server).await;
    let vars = format!("/teams/{TEAM}/environments/{ENV}/variables");
    let call = |method: HttpMethod, body: Option<serde_json::Value>| ApiRequest {
        method,
        path: vars.clone(),
        query: None,
        body,
        if_match: None,
        idempotency_key: None,
    };
    Mock::given(method("GET"))
        .and(path(format!("/api/v1{vars}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"variables": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1{vars}")))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let read = iii_cloud::api::api(&h.ctx, call(HttpMethod::Get, None))
        .await
        .unwrap();
    assert_eq!(read.status, 200);
    for m in [
        HttpMethod::Post,
        HttpMethod::Put,
        HttpMethod::Patch,
        HttpMethod::Delete,
    ] {
        let err = iii_cloud::api::api(&h.ctx, call(m, Some(json!({"value": "x"}))))
            .await
            .unwrap_err();
        assert_eq!(err.code, "PERSON_ONLY", "{m:?}");
        assert!(err.message.contains("page"), "{}", err.message);
    }
    // The page's write goes through: the one POST the mock expects.
    let wrote = iii_cloud::api::write(
        &h.ctx,
        call(HttpMethod::Post, Some(json!({"name": "A", "value": "1"}))),
    )
    .await
    .unwrap();
    assert_eq!(wrote.status, 201);
    let secret = iii_cloud::api::write(
        &h.ctx,
        call(
            HttpMethod::Post,
            Some(json!({"name": "A", "type": "secret", "value": "x"})),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(secret.code, "SECRET_VIA_API");
    assert!(
        iii_cloud::functions::catalog()
            .iter()
            .any(|s| s.function_id == "iii-cloud::api::write")
    );
}
