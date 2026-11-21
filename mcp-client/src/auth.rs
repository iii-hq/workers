//! Client-credentials tokens: the form a token request sends, one POST to
//! the token endpoint, and the single-flight holder of the current token.
//! Interactive sign-in: the redirect address, the pending logins and the
//! fixed texts of a failed sign-in.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use rmcp::transport::auth::AuthError;
use serde_json::Value;

use crate::server::{ErrorCode, ServerError};
use crate::translate::scrub;

pub const CALLBACK_PATH: &str = "/mcp-client/oauth/callback";

/// How long a started sign-in waits for its callback.
pub const LOGIN_TTL: Duration = Duration::from_secs(600);

/// The redirect URI of every sign-in, on the engine's http port.
pub fn redirect_uri(port: Option<u16>) -> String {
    format!("http://127.0.0.1:{}{CALLBACK_PATH}", port.unwrap_or(3111))
}

/// The `state` pair of a callback address; `None` without one.
pub fn state_of(callback_url: &str) -> Option<String> {
    let url = url::Url::parse(callback_url).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.into_owned())
}

/// Whether `url` names `localhost` or a loopback IP.
pub fn is_loopback(url: &str) -> bool {
    match url::Url::parse(url).ok().as_ref().and_then(url::Url::host) {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// A failed sign-in's text. No arm formats the error: its fields hold what
/// rmcp or the provider sent.
pub fn login_error_text(error: &AuthError) -> String {
    const METADATA: &str = "sign-in could not start: the server's OAuth metadata could not be read";
    match error {
        AuthError::AuthorizationServerMismatch { .. }
        | AuthError::AuthorizationServerMissingIssuer { .. } => {
            "sign-in refused: the answer did not come from the expected authorization server"
        }
        AuthError::TokenExchangeFailed(_) => {
            "sign-in failed: the authorization server refused the code"
        }
        AuthError::AuthorizationFailed(_) => {
            "sign-in failed: the authorization response was not accepted"
        }
        AuthError::RegistrationFailed(_) => {
            "sign-in could not start: client registration failed; set auth.client_id for this server"
        }
        AuthError::PkceUnsupported => {
            "sign-in could not start: the authorization server does not support PKCE S256"
        }
        AuthError::MetadataError(_)
        | AuthError::NoAuthorizationSupport
        | AuthError::DiscoveryRequestFailed { .. }
        | AuthError::ProtectedResourceMetadataInvalid { .. }
        | AuthError::AuthorizationServersUnavailable { .. }
        | AuthError::HttpError(_)
        | AuthError::UrlError(_) => METADATA,
        AuthError::CredentialStoreError(_) => {
            "the sign-in session could not be saved: the secrets worker did not answer"
        }
        _ => "sign-in failed",
    }
    .into()
}

struct Pending<S> {
    server: String,
    session: S,
    started: Instant,
    /// Unix milliseconds.
    expires_at: u64,
}

/// The sign-ins waiting for their callback, by `state`: one per server,
/// each taken once, none after `LOGIN_TTL`.
pub struct PendingLogins<S> {
    by_state: Mutex<HashMap<String, Pending<S>>>,
}

impl<S> Default for PendingLogins<S> {
    fn default() -> Self {
        PendingLogins {
            by_state: Mutex::default(),
        }
    }
}

impl<S> PendingLogins<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops the server's older pending login.
    pub fn insert(&self, state: String, server: &str, session: S, now: Instant) {
        let expires_at = SystemTime::now()
            .checked_add(LOGIN_TTL)
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        let mut by_state = self.by_state.lock().unwrap();
        by_state.retain(|_, p| p.server != server);
        let server = server.into();
        by_state.insert(
            state,
            Pending {
                server,
                session,
                started: now,
                expires_at,
            },
        );
    }

    /// The server and session `state` started; once.
    pub fn take(&self, state: &str, now: Instant) -> Option<(String, S)> {
        let p = self.by_state.lock().unwrap().remove(state)?;
        (now.saturating_duration_since(p.started) < LOGIN_TTL).then_some((p.server, p.session))
    }

    /// Whether a login was pending for `server`.
    pub fn cancel(&self, server: &str) -> bool {
        let mut by_state = self.by_state.lock().unwrap();
        let before = by_state.len();
        by_state.retain(|_, p| p.server != server);
        by_state.len() != before
    }

    /// The expiry of the server's pending login, in unix milliseconds.
    pub fn pending(&self, server: &str, now: Instant) -> Option<u64> {
        let mut by_state = self.by_state.lock().unwrap();
        by_state.retain(|_, p| now.saturating_duration_since(p.started) < LOGIN_TTL);
        by_state
            .values()
            .find(|p| p.server == server)
            .map(|p| p.expires_at)
    }
}

/// The form body of a client-credentials token request; `scope` as written.
pub fn token_request_body(client_id: &str, client_secret: &str, scope: Option<&str>) -> String {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "client_credentials")
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret);
    if let Some(scope) = scope {
        form.append_pair("scope", scope);
    }
    form.finish()
}

/// The client of every token request: never follows a redirect, so the
/// client secret goes to the configured origin only, and keeps no idle
/// connection.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(0)
        .build()
        .expect("a reqwest client needs the rustls provider installed at start")
}

/// One token request. A refusal names the endpoint's `error_description`,
/// else its `error`, else the status: never the token, the body, the client
/// secret or a reqwest error, whose text carries the URL.
pub async fn mint(
    http: &reqwest::Client,
    token_url: &str,
    body: String,
) -> Result<String, ServerError> {
    // Scrubbed before the cut, which could leave part of it unmatched.
    let secret: Vec<String> = url::form_urlencoded::parse(body.as_bytes())
        .filter(|(k, _)| k == "client_secret")
        .map(|(_, v)| v.into_owned())
        .collect();
    let unreachable =
        || ServerError::new(ErrorCode::ConnectFailed, "token endpoint did not answer");
    let rejected = |detail: &str| {
        let message = format!("token request rejected: {detail}");
        ServerError::new(ErrorCode::CredentialRejected, message)
    };
    let response = http
        .post(token_url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(ACCEPT, "application/json")
        .timeout(Duration::from_secs(30))
        .body(body)
        .send()
        .await
        .map_err(|_| unreachable())?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| unreachable())?;
    let answer: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    if !status.is_success() {
        let detail = match answer["error_description"]
            .as_str()
            .or(answer["error"].as_str())
        {
            Some(words) => scrub(words, &secret).chars().take(200).collect(),
            None => format!("HTTP {}", status.as_u16()),
        };
        return Err(rejected(&detail));
    }
    match answer["access_token"].as_str() {
        Some(token) if !token.is_empty() => Ok(token.into()),
        _ => Err(rejected("the answer holds no access_token")),
    }
}

/// The current token of one server. Concurrent callers share one POST: the
/// lock is held across it. No `Debug`: it holds the client secret.
pub struct Minter {
    http: reqwest::Client,
    token_url: String,
    body: String,
    token: tokio::sync::Mutex<Option<String>>,
}

impl Minter {
    pub fn new(http: reqwest::Client, token_url: String, body: String) -> Minter {
        Minter {
            http,
            token_url,
            body,
            token: tokio::sync::Mutex::new(None),
        }
    }

    /// The token held, else a new one.
    pub async fn token(&self) -> Result<String, ServerError> {
        let mut held = self.token.lock().await;
        if let Some(token) = &*held {
            return Ok(token.clone());
        }
        let token = mint(&self.http, &self.token_url, self.body.clone()).await?;
        *held = Some(token.clone());
        Ok(token)
    }

    /// Drops the token held: the next `token` mints.
    pub async fn invalidate(&self) {
        *self.token.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn an_echoed_client_secret_is_scrubbed_before_the_cut() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let secret = "secret-across-the-cut-0123456789";
        let words = format!("{}{secret} tail", "x".repeat(190));
        let reply = serde_json::json!({"error": "invalid_client", "error_description": words});
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(reply))
            .mount(&server)
            .await;
        let body = token_request_body("app", secret, None);
        let err = mint(&http_client(), &server.uri(), body).await.unwrap_err();
        for start in 0..=secret.len() - 6 {
            let piece = &secret[start..start + 6];
            assert!(!err.message.contains(piece), "{piece} in {}", err.message);
        }
        assert!(err.message.ends_with("[redacted]"), "{}", err.message);
    }

    const CB: &str = "http://127.0.0.1:3111/mcp-client/oauth/callback";

    #[test]
    fn redirect_uri_uses_the_http_port_or_3111() {
        assert_eq!(
            redirect_uri(Some(8080)),
            "http://127.0.0.1:8080/mcp-client/oauth/callback"
        );
        assert_eq!(redirect_uri(None), CB);
    }

    #[test]
    fn state_of_reads_only_the_state_pair() {
        let with_code = format!("{CB}?code=c&state=abc&iss=https%3A%2F%2Fas");
        assert_eq!(state_of(&with_code).as_deref(), Some("abc"));
        let denied = format!("{CB}?error=access_denied&state=abc");
        assert_eq!(state_of(&denied).as_deref(), Some("abc"));
        assert_eq!(state_of(&format!("{CB}?code=c")), None);
        assert_eq!(state_of("not a url"), None);
    }

    #[test]
    fn a_pending_login_is_single_use_and_found_by_state_only() {
        let (p, t0) = (PendingLogins::new(), Instant::now());
        p.insert("s1".into(), "linear", 7u32, t0);
        assert_eq!(p.take("wrong", t0), None);
        assert!(
            p.pending("linear", t0).is_some(),
            "a wrong state consumed nothing"
        );
        assert_eq!(p.take("s1", t0), Some(("linear".into(), 7)));
        assert_eq!(p.take("s1", t0), None, "a replay");
        assert_eq!(p.pending("linear", t0), None);
    }

    #[test]
    fn a_pending_login_lives_ten_minutes() {
        let (p, t0) = (PendingLogins::new(), Instant::now());
        p.insert("s1".into(), "linear", 1u32, t0);
        assert!(p.pending("linear", t0 + Duration::from_secs(599)).is_some());
        assert_eq!(p.pending("linear", t0 + LOGIN_TTL), None);
        assert_eq!(p.take("s1", t0 + LOGIN_TTL), None);
        assert_eq!(p.take("s1", t0), None, "an expired entry is gone for good");
    }

    #[test]
    fn one_pending_login_per_server_and_cancel_drops_it() {
        let (p, t0) = (PendingLogins::new(), Instant::now());
        p.insert("s1".into(), "linear", 1u32, t0);
        p.insert("s2".into(), "linear", 2, t0);
        p.insert("s3".into(), "other", 3, t0);
        assert_eq!(p.take("s1", t0), None);
        assert!(p.cancel("other"));
        assert!(!p.cancel("other"));
        assert_eq!(p.take("s3", t0), None);
        assert_eq!(p.take("s2", t0), Some(("linear".into(), 2)));
    }

    #[test]
    fn pending_reports_the_expiry_in_unix_milliseconds() {
        let (p, t0) = (PendingLogins::new(), Instant::now());
        p.insert("s1".into(), "linear", 1u32, t0);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let expires_at = p.pending("linear", t0).unwrap();
        assert!(
            (now_ms + 599_000..=now_ms + 601_000).contains(&expires_at),
            "{expires_at} vs {now_ms}"
        );
    }

    #[test]
    fn only_loopback_hosts_count_as_loopback() {
        for url in [
            "http://127.0.0.1:8080/mcp",
            "http://localhost/mcp",
            "http://[::1]:9/mcp",
        ] {
            assert!(is_loopback(url), "{url}");
        }
        for url in [
            "https://mcp.linear.app/mcp",
            "http://10.0.0.9/mcp",
            "https://localhost.evil.example/mcp",
            "nope",
        ] {
            assert!(!is_loopback(url), "{url}");
        }
    }

    #[test]
    fn login_errors_never_quote_what_rmcp_or_the_provider_said() {
        const LEAK: &str = "leak-7f3a9c";
        let rows = [
            (
                AuthError::AuthorizationServerMismatch {
                    expected_issuer: LEAK.into(),
                    received_issuer: LEAK.into(),
                },
                "sign-in refused: the answer did not come from the expected authorization server",
            ),
            (
                AuthError::AuthorizationServerMissingIssuer {
                    expected_issuer: LEAK.into(),
                },
                "sign-in refused: the answer did not come from the expected authorization server",
            ),
            (
                AuthError::TokenExchangeFailed(LEAK.into()),
                "sign-in failed: the authorization server refused the code",
            ),
            (
                AuthError::AuthorizationFailed(LEAK.into()),
                "sign-in failed: the authorization response was not accepted",
            ),
            (
                AuthError::RegistrationFailed(LEAK.into()),
                "sign-in could not start: client registration failed; set auth.client_id for this server",
            ),
            (
                AuthError::PkceUnsupported,
                "sign-in could not start: the authorization server does not support PKCE S256",
            ),
            (
                AuthError::MetadataError(LEAK.into()),
                "sign-in could not start: the server's OAuth metadata could not be read",
            ),
            (
                AuthError::NoAuthorizationSupport,
                "sign-in could not start: the server's OAuth metadata could not be read",
            ),
            (
                AuthError::CredentialStoreError(LEAK.into()),
                "the sign-in session could not be saved: the secrets worker did not answer",
            ),
            (AuthError::InternalError(LEAK.into()), "sign-in failed"),
        ];
        for (error, text) in rows {
            assert_eq!(login_error_text(&error), text);
        }
    }

    #[test]
    fn token_body_is_form_encoded_and_keeps_the_scope_verbatim() {
        assert_eq!(
            token_request_body("app id", "s&cret=", Some("read,write")),
            "grant_type=client_credentials&client_id=app+id&client_secret=s%26cret%3D&scope=read%2Cwrite"
        );
        assert_eq!(
            token_request_body("a", "b", None),
            "grant_type=client_credentials&client_id=a&client_secret=b"
        );
    }
}
