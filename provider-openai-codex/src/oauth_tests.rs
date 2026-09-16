use super::*;
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

struct Reply {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: vec![],
        }
    }

    fn retry_after(mut self, value: &str) -> Self {
        self.headers.push(("Retry-After".into(), value.into()));
        self
    }
}

#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

struct Stub {
    issuer: String,
    requests: mpsc::UnboundedReceiver<Request>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Stub {
    async fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                tx.send(request).unwrap();
                let extra = reply
                    .headers
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}\r\n"))
                    .collect::<String>();
                let response = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    reply.status, reply.body.len(), extra, reply.body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            issuer,
            requests,
            task,
        }
    }

    fn client(&self) -> OAuthClient {
        OAuthClient::with_issuer(
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            format!("{}/", self.issuer),
        )
    }

    async fn request(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(2), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

async fn read_request(stream: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 2048];
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let mut lines = head.lines();
    let first = lines.next().unwrap().split_whitespace().collect::<Vec<_>>();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let length = headers
        .get("content-length")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    while bytes.len() < header_end + length {
        let mut chunk = [0; 2048];
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
    }
    Request {
        method: first[0].into(),
        path: first[1].into(),
        headers,
        body: String::from_utf8(bytes[header_end..].to_vec()).unwrap(),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn jwt(claims: Value) -> String {
    format!(
        "e30.{}.signature",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

fn credential() -> Credential {
    Credential {
        access_token: jwt(
            json!({"exp": 2_000_000_000, "https://api.openai.com/auth": {"chatgpt_account_id": "account-secret"}}),
        ),
        refresh_token: Some("refresh-secret".into()),
        id_token: Some(jwt(json!({"chatgpt_account_id": "account-secret"}))),
        account_id: "account-secret".into(),
        expires_at: 2_000_000_000,
    }
}

fn device() -> DeviceCode {
    DeviceCode {
        device_auth_id: "device-secret".into(),
        user_code: "ABCD-SECRET".into(),
        verification_uri: "https://auth.openai.com/codex/device".into(),
        interval: 5,
        expires_at: now() + 900,
    }
}

fn authorization() -> Reply {
    Reply::json(
        200,
        json!({"authorization_code": "code +&=/secret", "code_verifier": "verifier +&=/secret", "code_challenge": "challenge-secret"}),
    )
}

#[tokio::test]
async fn start_posts_native_client_id_and_normalizes_aliases_and_intervals() {
    for (field, interval, expected) in [
        ("user_code", json!("7"), 7),
        ("usercode", json!(3), 3),
        ("user_code", json!("0"), 1),
        ("user_code", json!(0), 1),
        ("user_code", Value::Null, 5),
        ("user_code", json!("bad"), 5),
        ("user_code", json!(-1), 5),
    ] {
        let mut response = json!({"device_auth_id": "device-secret", "interval": interval});
        response[field] = json!("ABCD-SECRET");
        let mut stub = Stub::new(vec![Reply::json(200, response)]).await;
        let before = now();
        let result = stub.client().start().await.unwrap();
        assert_eq!(result.device_auth_id, "device-secret");
        assert_eq!(result.user_code, "ABCD-SECRET");
        assert_eq!(
            result.verification_uri,
            format!("{}/codex/device", stub.issuer)
        );
        assert_eq!(result.interval, expected);
        assert!((before + 900..=now() + 900).contains(&result.expires_at));
        let request = stub.request().await;
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/accounts/deviceauth/usercode");
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({"client_id": "app_EMoamEEZ73f0CkXaXp7hrann"})
        );
    }
}

#[tokio::test]
async fn start_404_explains_how_to_enable_device_login() {
    let stub = Stub::new(vec![Reply {
        status: 404,
        body: "server-secret".into(),
        headers: vec![],
    }])
    .await;
    let err = stub.client().start().await.unwrap_err();
    assert_eq!(err.code, "device_login_disabled");
    assert_eq!(err.message, "Enable device code login in ChatGPT security settings or ask your workspace administrator.");
    assert!(err.permanent);
    assert!(!format!("{err:?}").contains("server-secret"));
}

#[tokio::test]
async fn start_unauthorized_requires_an_explicit_device_login_disabled_code() {
    for status in [401, 403] {
        for body in [
            json!({"error": "device_login_disabled", "error_description": "server-secret"}),
            json!({"error": {"code": "device_auth_disabled", "message": "server-secret"}}),
            json!({"code": "device_code_login_disabled", "message": "server-secret"}),
        ] {
            let stub = Stub::new(vec![Reply::json(status, body)]).await;
            let err = stub.client().start().await.unwrap_err();
            assert_eq!(err.code, "device_login_disabled");
            assert_eq!(err.message, "Enable device code login in ChatGPT security settings or ask your workspace administrator.");
            assert!(err.permanent);
            assert!(!format!("{err:?}").contains("server-secret"));
        }
        for body in [
            json!({"error": "access_denied"}),
            json!({"error": "unauthorized"}),
            json!({"error_description": "device_login_disabled"}),
        ] {
            let stub = Stub::new(vec![Reply::json(status, body)]).await;
            let err = stub.client().start().await.unwrap_err();
            assert_ne!(err.code, "device_login_disabled");
            assert!(err.permanent);
        }
    }
}

#[tokio::test]
async fn pending_uses_the_larger_of_poll_interval_and_retry_after() {
    for (status, header, interval, expected) in [
        (403, "12", 5, 12),
        (404, "2", 5, 5),
        (403, "0", 0, 1),
        (404, "bad", 7, 7),
    ] {
        let mut stub = Stub::new(vec![
            Reply::json(status, json!({"error": "pending"})).retry_after(header)
        ])
        .await;
        let mut device = device();
        device.interval = interval;
        let outcome = stub.client().poll(&device).await.unwrap();
        assert!(
            matches!(outcome, PollOutcome::Pending { retry_after_secs } if retry_after_secs == expected)
        );
        let request = stub.request().await;
        assert_eq!(request.path, "/api/accounts/deviceauth/token");
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({"device_auth_id": "device-secret", "user_code": "ABCD-SECRET"})
        );
    }
}

#[tokio::test]
async fn authorization_exchanges_code_as_form_and_extracts_jwt_metadata() {
    let cred = credential();
    let mut stub = Stub::new(vec![authorization(), Reply::json(200, json!({"access_token": cred.access_token, "refresh_token": "new-refresh", "id_token": cred.id_token}))]).await;
    let PollOutcome::Authorized(result) = stub.client().poll(&device()).await.unwrap() else {
        panic!("expected authorization");
    };
    assert_eq!(result.access_token, cred.access_token);
    assert_eq!(result.account_id, "account-secret");
    assert_eq!(result.expires_at, 2_000_000_000);
    assert_eq!(result.refresh_token.as_deref(), Some("new-refresh"));
    stub.request().await;
    let exchange = stub.request().await;
    assert_eq!(exchange.method, "POST");
    assert_eq!(exchange.path, "/oauth/token");
    assert_eq!(
        exchange.headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    // Decode form fields independently of the adapter, including PKCE special characters.
    let parsed = reqwest::Url::parse(&format!("http://localhost/?{}", exchange.body)).unwrap();
    let form = parsed
        .query_pairs()
        .into_owned()
        .collect::<BTreeMap<_, _>>();
    assert_eq!(form.len(), 5);
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "code +&=/secret");
    assert_eq!(form["code_verifier"], "verifier +&=/secret");
    assert_eq!(form["client_id"], "app_EMoamEEZ73f0CkXaXp7hrann");
    assert_eq!(
        form["redirect_uri"],
        format!("{}/deviceauth/callback", stub.issuer)
    );
}

#[tokio::test]
async fn initial_exchange_requires_a_refresh_token_for_session_renewal() {
    for refresh in [None, Some(Value::Null), Some(json!("")), Some(json!("  "))] {
        let cred = credential();
        let mut body = json!({"access_token": cred.access_token, "id_token": cred.id_token});
        if let Some(refresh) = refresh {
            body["refresh_token"] = refresh;
        }
        let stub = Stub::new(vec![authorization(), Reply::json(200, body)]).await;
        let err = stub.client().poll(&device()).await.unwrap_err();
        assert_eq!(err.code, "missing_refresh_token");
        assert!(err.permanent);
        assert!(!format!("{err:?}").contains(&cred.access_token));
    }
}

#[tokio::test]
async fn refresh_sends_json_and_preserves_fields_not_rotated() {
    let old = credential();
    let new_access = jwt(json!({"account_id": "account-secret", "exp": 2_100_000_000}));
    for response in [
        json!({"access_token": new_access}),
        json!({"refresh_token": "rotated-refresh"}),
        json!({"id_token": "rotated-id"}),
        json!({}),
    ] {
        let mut stub = Stub::new(vec![Reply::json(200, response.clone())]).await;
        let result = stub.client().refresh(&old).await.unwrap();
        assert_eq!(
            result.access_token,
            response["access_token"]
                .as_str()
                .unwrap_or(&old.access_token)
        );
        assert_eq!(
            result.refresh_token.as_deref(),
            response["refresh_token"]
                .as_str()
                .or(old.refresh_token.as_deref())
        );
        assert_eq!(
            result.id_token.as_deref(),
            response["id_token"].as_str().or(old.id_token.as_deref())
        );
        assert_eq!(
            result.expires_at,
            if response.get("access_token").is_some() {
                2_100_000_000
            } else {
                2_000_000_000
            }
        );
        let request = stub.request().await;
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/oauth/token");
        assert_eq!(request.headers["content-type"], "application/json");
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).unwrap(),
            json!({"client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token", "refresh_token": "refresh-secret"})
        );
    }
}

#[test]
fn debug_redacts_credentials_and_device_secrets() {
    let cred = credential();
    let text = format!("{cred:?} {:?}", device());
    for secret in [
        &cred.access_token,
        cred.refresh_token.as_ref().unwrap(),
        cred.id_token.as_ref().unwrap(),
        &cred.account_id,
        "device-secret",
        "ABCD-SECRET",
    ] {
        assert!(!text.contains(secret), "Debug exposed a secret");
    }
    let value = serde_json::to_value(&cred).unwrap();
    let round_trip: Credential = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip.access_token, cred.access_token);
}

#[tokio::test]
async fn refresh_rejects_an_account_switch_even_when_access_token_is_unchanged() {
    for through_id in [false, true] {
        let mut old = credential();
        if through_id {
            old.access_token = jwt(json!({"exp": 2_000_000_000}));
        }
        let response = if through_id {
            json!({"id_token": jwt(json!({"chatgpt_account_id": "different-account"}))})
        } else {
            json!({"access_token": jwt(json!({"account_id": "different-account", "exp": 2_100_000_000}))})
        };
        let stub = Stub::new(vec![Reply::json(200, response)]).await;
        let err = stub.client().refresh(&old).await.unwrap_err();
        assert!(err.permanent);
        assert_eq!(err.code, "account_mismatch");
        assert!(!format!("{err:?}").contains("different-account"));
        assert_eq!(old.account_id, "account-secret");
    }
}

#[tokio::test]
async fn metadata_uses_id_token_account_but_requires_access_token_expiry() {
    let access = jwt(json!({"exp": 2_100_000_000}));
    let stub = Stub::new(vec![authorization(), Reply::json(200, json!({"access_token": access, "refresh_token": "refresh-secret", "id_token": jwt(json!({"chatgpt_account_id": "id-account"}))}))]).await;
    let PollOutcome::Authorized(result) = stub.client().poll(&device()).await.unwrap() else {
        panic!("expected authorization");
    };
    assert_eq!(result.account_id, "id-account");
    assert_eq!(result.expires_at, 2_100_000_000);

    for (mut tokens, expected) in [
        (
            json!({"access_token": "sk-api-key-secret", "id_token": jwt(json!({"exp": 2_100_000_000, "account_id": "id-account"}))}),
            "invalid_access_token",
        ),
        (
            json!({"access_token": jwt(json!({"account_id": "account-secret"})), "id_token": jwt(json!({"exp": 2_100_000_000}))}),
            "missing_expiry",
        ),
        (
            json!({"access_token": jwt(json!({"account_id": "account-secret", "exp": 0}))}),
            "missing_expiry",
        ),
        (
            json!({"access_token": jwt(json!({"exp": 2_100_000_000}))}),
            "missing_account_id",
        ),
        (
            json!({"access_token": jwt(json!({"exp": 2_100_000_000, "account_id": "   "}))}),
            "missing_account_id",
        ),
        (
            json!({"refresh_token": "refresh-secret"}),
            "missing_access_token",
        ),
    ] {
        tokens["refresh_token"] = json!("refresh-secret");
        let stub = Stub::new(vec![authorization(), Reply::json(200, tokens)]).await;
        let err = stub.client().poll(&device()).await.unwrap_err();
        assert!(err.permanent);
        assert_eq!(err.code, expected);
    }
}

#[tokio::test]
async fn expired_device_and_missing_refresh_token_fail_before_network() {
    let mut stub = Stub::new(vec![Reply::json(500, json!({}))]).await;
    let mut expired = device();
    expired.expires_at = now() - 1;
    let err = stub.client().poll(&expired).await.unwrap_err();
    assert_eq!(err.code, "expired_token");
    assert!(err.permanent);
    for refresh_token in [None, Some("".into()), Some("  ".into())] {
        let mut old = credential();
        old.refresh_token = refresh_token;
        let err = stub.client().refresh(&old).await.unwrap_err();
        assert_eq!(err.code, "missing_refresh_token");
        assert!(err.permanent);
    }
    assert!(stub.requests.try_recv().is_err());
}

#[tokio::test]
async fn empty_protocol_secrets_are_rejected_before_the_next_request() {
    for body in [
        json!({"device_auth_id": "", "user_code": "code"}),
        json!({"device_auth_id": "id", "user_code": " "}),
    ] {
        let stub = Stub::new(vec![Reply::json(200, body)]).await;
        let err = stub.client().start().await.unwrap_err();
        assert!(err.permanent);
        assert_eq!(err.code, "invalid_response");
    }
    for field in ["authorization_code", "code_verifier", "code_challenge"] {
        let mut body = json!({"authorization_code": "auth", "code_verifier": "verifier", "code_challenge": "challenge"});
        body[field] = json!("");
        let mut stub = Stub::new(vec![Reply::json(200, body), Reply::json(500, json!({}))]).await;
        let err = stub.client().poll(&device()).await.unwrap_err();
        assert_eq!(err.code, "invalid_response");
        stub.request().await;
        assert!(stub.requests.try_recv().is_err());
    }
    for field in ["access_token", "refresh_token", "id_token"] {
        let stub = Stub::new(vec![Reply::json(200, json!({field: ""}))]).await;
        let err = stub.client().refresh(&credential()).await.unwrap_err();
        assert!(err.permanent);
        assert_eq!(err.code, "invalid_response");
    }
}

#[tokio::test]
async fn revoked_and_invalid_grant_errors_are_permanent_and_sanitized() {
    for (status, body, expected) in [
        (
            400,
            json!({"error": "invalid_grant", "error_description": "refresh-secret"}),
            "invalid_grant",
        ),
        (
            400,
            json!({"error": {"code": "invalid_grant", "message": "refresh-secret"}}),
            "invalid_grant",
        ),
        (
            401,
            json!({"error": {"code": "refresh_token_revoked", "message": "refresh-secret"}}),
            "refresh_token_revoked",
        ),
        (
            400,
            json!({"code": "refresh_token_reused", "message": "refresh-secret"}),
            "refresh_token_reused",
        ),
        (
            403,
            json!({"error": "revoked", "message": "refresh-secret"}),
            "revoked",
        ),
        (400, json!({"error": "token_revoked"}), "token_revoked"),
        (
            400,
            json!({"error": "refresh_token_expired"}),
            "refresh_token_expired",
        ),
        (400, json!({"error": "expired_token"}), "expired_token"),
        (
            400,
            json!({"error": "refresh-secret", "error_description": "refresh-secret"}),
            "http_error",
        ),
    ] {
        let stub = Stub::new(vec![Reply::json(status, body)]).await;
        let err = stub.client().refresh(&credential()).await.unwrap_err();
        assert!(err.permanent);
        assert_eq!(err.code, expected);
        assert_eq!(err.invalidates_session(), expected != "http_error");
        assert!(!format!("{err:?}").contains("refresh-secret"));
    }
}

#[tokio::test]
async fn refresh_protocol_failures_do_not_invalidate_the_existing_session() {
    let old = credential();
    for reply in [
        Reply {
            status: 403,
            body: "<html>Temporary proxy failure: refresh-secret</html>".into(),
            headers: vec![],
        },
        Reply::json(
            401,
            json!({"error": "unknown_error", "message": "refresh-secret"}),
        ),
        Reply::json(403, json!({"error": "access_denied"})),
        Reply::json(400, json!({"error": "invalid_client"})),
        Reply::json(403, json!({"error": "device_login_disabled"})),
        Reply {
            status: 200,
            body: "malformed-json refresh-secret".into(),
            headers: vec![],
        },
        Reply::json(200, json!({"access_token": 42})),
        Reply::json(200, json!({"access_token": ""})),
        Reply::json(
            200,
            json!({"access_token": jwt(json!({"account_id": "account-secret"}))}),
        ),
        Reply::json(
            200,
            json!({"access_token": jwt(json!({"account_id": "another-account", "exp": 2_100_000_000}))}),
        ),
    ] {
        let mut stub = Stub::new(vec![
            reply,
            Reply::json(200, json!({"refresh_token": "rotated-refresh"})),
        ])
        .await;
        let client = stub.client();
        let err = client.refresh(&old).await.unwrap_err();
        assert!(
            !err.invalidates_session(),
            "an exchange failure is not proof of session invalidation: {err:?}"
        );
        assert!(!format!("{err:?}").contains("refresh-secret"));
        // The same stored credential remains usable once the intermediary recovers.
        let recovered = client.refresh(&old).await.unwrap();
        assert_eq!(recovered.access_token, old.access_token);
        assert_eq!(recovered.refresh_token.as_deref(), Some("rotated-refresh"));
        for _ in 0..2 {
            let request = stub.request().await;
            assert_eq!(
                serde_json::from_str::<Value>(&request.body).unwrap()["refresh_token"],
                "refresh-secret"
            );
        }
    }
}

#[test]
fn retryable_errors_never_invalidate_a_session_even_with_a_known_code() {
    let err = OAuthError {
        code: "invalid_grant".into(),
        message: "Temporary failure".into(),
        permanent: false,
        retry_after_secs: Some(5),
    };
    assert!(!err.invalidates_session());
}

#[tokio::test]
async fn rate_limits_and_server_errors_are_transient_even_with_misleading_bodies() {
    for (status, expected) in [
        (429, "rate_limited"),
        (500, "service_unavailable"),
        (503, "service_unavailable"),
        (408, "request_timeout"),
    ] {
        let reply = || {
            Reply::json(
                status,
                json!({"error": "invalid_grant", "error_description": "refresh-secret"}),
            )
            .retry_after("17")
        };
        let stub = Stub::new(vec![reply(), reply(), reply()]).await;
        let client = stub.client();
        let errors = [
            client.start().await.unwrap_err(),
            client.poll(&device()).await.unwrap_err(),
            client.refresh(&credential()).await.unwrap_err(),
        ];
        for err in errors {
            assert!(!err.permanent);
            assert!(!err.invalidates_session());
            assert_eq!(err.code, expected);
            assert_eq!(err.retry_after_secs, Some(17));
            assert!(!format!("{err:?}").contains("refresh-secret"));
        }
    }
}

#[tokio::test]
async fn transient_poll_failure_preserves_minimum_interval_during_exchange_too() {
    for replies in [
        vec![Reply::json(503, json!({})).retry_after("1")],
        vec![
            authorization(),
            Reply::json(429, json!({})).retry_after("1"),
        ],
    ] {
        let stub = Stub::new(replies).await;
        let err = stub.client().poll(&device()).await.unwrap_err();
        assert!(!err.permanent);
        assert_eq!(err.retry_after_secs, Some(5));
    }
}

#[tokio::test]
async fn retry_after_accepts_http_dates_and_normalizes_zero() {
    let date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(60));
    let stub = Stub::new(vec![Reply::json(429, json!({})).retry_after(&date)]).await;
    let err = stub.client().start().await.unwrap_err();
    assert!((59..=60).contains(&err.retry_after_secs.unwrap()));
    for value in ["0", "Thu, 01 Jan 1970 00:00:00 GMT"] {
        let stub = Stub::new(vec![Reply::json(503, json!({})).retry_after(value)]).await;
        assert_eq!(
            stub.client().start().await.unwrap_err().retry_after_secs,
            Some(1)
        );
    }
    let stub = Stub::new(vec![Reply::json(
        200,
        json!({"device_auth_id": "device", "user_code": "code", "interval": 3}),
    )
    .retry_after("10")])
    .await;
    assert_eq!(stub.client().start().await.unwrap().interval, 10);
}

#[tokio::test]
async fn malformed_success_and_transport_errors_never_expose_response_or_url() {
    let stub = Stub::new(vec![Reply {
        status: 200,
        body: "not-json refresh-secret".into(),
        headers: vec![],
    }])
    .await;
    let err = stub.client().refresh(&credential()).await.unwrap_err();
    assert_eq!(err.code, "invalid_response");
    assert!(err.permanent);
    assert!(!err.invalidates_session());
    assert!(!format!("{err:?}").contains("refresh-secret"));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}/url-secret", listener.local_addr().unwrap());
    drop(listener);
    let client = OAuthClient::with_issuer(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        issuer,
    );
    let err = client.poll(&device()).await.unwrap_err();
    assert!(!err.permanent);
    assert_eq!(err.code, "network_error");
    assert!(!err.invalidates_session());
    assert_eq!(err.retry_after_secs, Some(5));
    assert!(!format!("{err:?}").contains("url-secret"));
}

#[tokio::test]
async fn request_timeout_covers_a_stalled_response_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let client = OAuthClient::with_issuer(
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap(),
        issuer,
    );
    let request = tokio::spawn(async move { client.start().await });
    let (mut stream, _) = listener.accept().await.unwrap();
    read_request(&mut stream).await;
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 999\r\n\r\n{",
        )
        .await
        .unwrap();
    // Freeze only after connecting; otherwise automatic clock advancement can race TCP I/O.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(21)).await;
    let err = tokio::time::timeout(Duration::from_secs(1), request)
        .await
        .expect("the per-request timeout must override the supplied client's longer timeout")
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, "request_timeout");
    assert!(!err.permanent);
}

#[tokio::test]
async fn redirect_responses_are_errors_and_do_not_forward_refresh_tokens() {
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let reply = Reply {
        status: 307,
        body: String::new(),
        headers: vec![(
            "Location".into(),
            format!("http://{}/stolen", destination.local_addr().unwrap()),
        )],
    };
    let stub = Stub::new(vec![reply]).await;
    let err = stub.client().refresh(&credential()).await.unwrap_err();
    assert!(err.permanent);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), destination.accept())
            .await
            .is_err()
    );
}
