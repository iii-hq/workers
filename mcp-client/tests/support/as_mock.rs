//! An OAuth authorization server on an `McpMock`'s own wiremock server, so
//! the issuer is that server's origin, and the gate in front of its `/mcp`:
//! the wire shapes rmcp 3.11.0 reads in discovery, registration and the
//! token endpoint.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mcp_client::server::{Manager, State};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, Request, ResponseTemplate};

use super::mcp_mock::{McpMock, echo_mock};
use super::{FakeHost, WAIT, connect, server};

/// Part of every code and token the mock issues.
pub const MARK: &str = "7f3a9c-do-not-print";

const VAULT_REF: &str = "secret://MCP_CLIENT_SRV_OAUTH";

#[derive(Default)]
struct Tokens {
    issued: usize,
    current: Option<String>,
    expired: bool,
    reject_refresh: bool,
    hold_refresh: Option<Duration>,
    registrations: usize,
    /// Every token request's form body and when it arrived.
    requests: Vec<(Instant, String)>,
}

/// Matches an `/mcp` request that lacks `Bearer <the current access token>`.
struct Gate(Arc<Mutex<Tokens>>);

impl Match for Gate {
    fn matches(&self, request: &Request) -> bool {
        let t = self.0.lock().unwrap();
        let valid = t.current.as_ref().filter(|_| !t.expired);
        let sent = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        valid.is_none_or(|token| sent != Some(format!("Bearer {token}").as_str()))
    }
}

pub struct AsMock {
    base: String,
    tokens: Arc<Mutex<Tokens>>,
}

fn form(body: &[u8]) -> HashMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

impl AsMock {
    pub async fn start(mcp: &McpMock) -> AsMock {
        let base = mcp.server().uri();
        let tokens = Arc::new(Mutex::new(Tokens::default()));
        let challenge =
            format!(r#"Bearer resource_metadata="{base}/.well-known/oauth-protected-resource""#);
        Mock::given(path("/mcp"))
            .and(Gate(tokens.clone()))
            .respond_with(ResponseTemplate::new(401).insert_header("WWW-Authenticate", challenge))
            .with_priority(1)
            .mount(mcp.server())
            .await;
        let documents = [
            (
                "/.well-known/oauth-protected-resource",
                json!({"resource": mcp.url(), "authorization_servers": [base]}),
            ),
            (
                "/.well-known/oauth-authorization-server",
                json!({
                    "issuer": base,
                    "authorization_endpoint": format!("{base}/authorize"),
                    "token_endpoint": format!("{base}/token"),
                    "registration_endpoint": format!("{base}/register"),
                    "response_types_supported": ["code"],
                    "code_challenge_methods_supported": ["S256"],
                    "authorization_response_iss_parameter_supported": true
                }),
            ),
        ];
        for (at, document) in documents {
            Mock::given(method("GET"))
                .and(path(at))
                .respond_with(ResponseTemplate::new(200).set_body_json(document))
                .with_priority(1)
                .mount(mcp.server())
                .await;
        }
        let t = tokens.clone();
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(move |request: &Request| {
                t.lock().unwrap().registrations += 1;
                let sent: Value = serde_json::from_slice(&request.body).unwrap_or_default();
                let reply =
                    json!({"client_id": "mock-client", "redirect_uris": sent["redirect_uris"]});
                ResponseTemplate::new(201).set_body_json(reply)
            })
            .with_priority(1)
            .mount(mcp.server())
            .await;
        let t = tokens.clone();
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(move |request: &Request| {
                let body = String::from_utf8_lossy(&request.body).into_owned();
                let refresh = form(&request.body).get("grant_type").map(String::as_str)
                    == Some("refresh_token");
                let mut t = t.lock().unwrap();
                t.requests.push((Instant::now(), body));
                if refresh && t.reject_refresh {
                    return ResponseTemplate::new(400)
                        .set_body_json(json!({"error": "invalid_grant"}));
                }
                t.issued += 1;
                let n = t.issued;
                let access = format!("at-{n}-{MARK}");
                (t.current, t.expired) = (Some(access.clone()), false);
                let reply = json!({"access_token": access, "token_type": "Bearer",
                    "expires_in": 3600, "refresh_token": format!("rt-{n}-{MARK}")});
                let answer = ResponseTemplate::new(200).set_body_json(reply);
                match t.hold_refresh.filter(|_| refresh) {
                    Some(by) => answer.set_delay(by),
                    None => answer,
                }
            })
            .with_priority(1)
            .mount(mcp.server())
            .await;
        AsMock { base, tokens }
    }

    /// The callback URL a browser would end on after `authorize_url`.
    pub fn authorize_redirect(&self, authorize_url: &str) -> String {
        let asked = query(authorize_url);
        let mut url = url::Url::parse(&asked["redirect_uri"]).unwrap();
        url.query_pairs_mut()
            .append_pair("code", &format!("code-{MARK}"))
            .append_pair("state", &asked["state"])
            .append_pair("iss", &self.base);
        url.into()
    }

    /// The gate refuses the current access token until the next one is issued.
    pub async fn expire_access_token(&self) {
        self.tokens.lock().unwrap().expired = true;
    }

    /// Every later refresh is answered `400 invalid_grant`.
    pub async fn reject_refresh(&self) {
        self.tokens.lock().unwrap().reject_refresh = true;
    }

    /// Every later refresh is answered `by` after it arrived.
    pub async fn hold_refresh(&self, by: Duration) {
        self.tokens.lock().unwrap().hold_refresh = Some(by);
    }

    /// The form bodies of the token requests, in arrival order.
    pub async fn token_requests(&self) -> Vec<String> {
        let t = self.tokens.lock().unwrap();
        t.requests.iter().map(|(_, b)| b.clone()).collect()
    }

    /// The token requests with their arrival times.
    pub async fn token_log(&self) -> Vec<(Instant, String)> {
        self.tokens.lock().unwrap().requests.clone()
    }

    pub async fn registrations(&self) -> usize {
        self.tokens.lock().unwrap().registrations
    }
}

/// The open entry `srv` on `mock`.
pub fn open(mock: &McpMock) -> Value {
    json!({"mcpServers": {"srv": {"type": "http", "url": mock.url()}}})
}

/// The entry `srv` on `mock` with `auth`.
pub fn oauth(mock: &McpMock, auth: Value) -> Value {
    json!({"mcpServers": {"srv": {"type": "http", "url": mock.url(), "auth": auth}}})
}

pub fn query(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

/// `url` with the pair `key` replaced by `value`, or removed for `None`.
pub fn with(url: &str, key: &str, value: Option<&str>) -> String {
    let mut url = url::Url::parse(url).unwrap();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .into_owned()
        .filter(|(k, _)| k != key)
        .collect();
    let mut out = url.query_pairs_mut();
    out.clear().extend_pairs(pairs);
    if let Some(value) = value {
        out.append_pair(key, value);
    }
    drop(out);
    url.into()
}

/// The vault secret `MCP_CLIENT_SRV_OAUTH`, parsed.
pub fn stored(host: &FakeHost) -> Value {
    serde_json::from_str(
        &host
            .vault("MCP_CLIENT_SRV_OAUTH")
            .expect("no stored session"),
    )
    .unwrap()
}

pub fn refreshes(bodies: &[String]) -> usize {
    bodies
        .iter()
        .filter(|b| b.contains("grant_type=refresh_token"))
        .count()
}

/// Makes the stored access token read as expired: the next call refreshes
/// before it sends.
pub fn age_stored_token(host: &FakeHost) {
    let mut envelope = stored(host);
    envelope["credentials"]["token_received_at"] = json!(1);
    host.set_secret(VAULT_REF, &envelope.to_string());
}

/// `srv` as an open entry on a mock that asks for sign-in, at `needs_login`.
pub async fn challenged() -> (McpMock, AsMock, Arc<FakeHost>, Arc<Manager>) {
    let mock = echo_mock().await;
    let as_mock = AsMock::start(&mock).await;
    let host = FakeHost::new();
    let m = connect(&host, open(&mock)).await;
    let reached = m.wait_for("srv", State::NeedsLogin, WAIT).await;
    assert!(reached, "{}", server(&m, "srv").await);
    (mock, as_mock, host, m)
}

/// Signs `srv` in; the callback URL it used.
pub async fn sign_in(m: &Manager, as_mock: &AsMock) -> String {
    let start = m.login_start("srv").await.unwrap();
    let callback = as_mock.authorize_redirect(&start.authorize_url);
    let done = m.login_complete(&callback).await.unwrap();
    assert_eq!(done, ("srv".to_string(), State::Ready));
    callback
}
