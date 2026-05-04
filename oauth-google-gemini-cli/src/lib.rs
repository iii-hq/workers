//! OAuth 2.0 Authorization Code + PKCE flow for Google Cloud Code Assist
//! (the auth backing the Gemini CLI installed-app flow).
//!
//! The flow mirrors the standard installed-app pattern Google publishes for
//! desktop clients:
//!   1. Generate a PKCE verifier/challenge pair (SHA-256, base64-url, no padding).
//!   2. Spawn a localhost HTTP listener on `127.0.0.1:53702/callback`.
//!   3. Build the authorize URL and hand it to the caller via `on_open_url`.
//!   4. Wait for the callback, exchange `code` for tokens at the Google OAuth
//!      token endpoint. The exchange includes `client_secret` because Google
//!      treats this client as an "installed app" — the secret is documented
//!      and bundled with the client_id.
//!   5. Return a [`Credential::OAuth`] from the shared `auth-credentials` crate.
//!
//! TODO: short-circuit [`login`] when Application Default Credentials are
//! present (file at `$HOME/.config/gcloud/application_default_credentials.json`
//! or env `GOOGLE_APPLICATION_CREDENTIALS`). For 0.1 we always run the
//! interactive flow.

use std::env;
use std::time::Duration;

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::timeout;

pub use auth_credentials::Credential;

const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform openid profile email";

/// Cloud Code Assist installed-app client_id. Set this env var to the value
/// documented in Google's installed-app OAuth flow before calling `login()`.
const CLIENT_ID_ENV: &str = "HARNESS_OAUTH_GEMINI_CLI_CLIENT_ID";
/// Cloud Code Assist installed-app client_secret. Despite the name, Google's
/// installed-app docs require this to be supplied during code exchange.
const CLIENT_SECRET_ENV: &str = "HARNESS_OAUTH_GEMINI_CLI_CLIENT_SECRET";

fn client_id() -> Result<String, OAuthError> {
    std::env::var(CLIENT_ID_ENV).map_err(|_| OAuthError::MissingClientCredentials(CLIENT_ID_ENV))
}

fn client_secret() -> Result<String, OAuthError> {
    std::env::var(CLIENT_SECRET_ENV)
        .map_err(|_| OAuthError::MissingClientCredentials(CLIENT_SECRET_ENV))
}
const DEFAULT_PORT: u16 = 53702;
const DEFAULT_HOST: &str = "127.0.0.1";
const CALLBACK_PATH: &str = "/callback";
const CALLBACK_TIMEOUT_SECS: u64 = 300;

/// Caller-supplied callbacks for the interactive part of the login flow.
pub struct OAuthLoginCallbacks {
    /// Called once with the fully-formed authorize URL. The caller is expected
    /// to open it in a browser (or instruct the user to do so).
    pub on_open_url: Box<dyn Fn(String) + Send + Sync>,
    /// Called with progress messages suitable for surfacing in a UI.
    pub on_progress: Box<dyn Fn(String) + Send + Sync>,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("http: {0}")]
    Http(String),
    #[error("invalid state")]
    InvalidState,
    #[error("missing code")]
    MissingCode,
    #[error("not an oauth credential")]
    WrongCredential,
    #[error("no refresh token")]
    NoRefreshToken,
    #[error("token endpoint returned non-success: {0}")]
    TokenEndpoint(String),
    #[error("missing required env var: {0}")]
    MissingClientCredentials(&'static str),
}

/// Returns the loopback host the callback listener will bind to.
/// Defaults to `127.0.0.1`; override via `HARNESS_OAUTH_GOOGLE_CALLBACK_HOST`.
pub fn callback_host() -> String {
    env::var("HARNESS_OAUTH_GOOGLE_CALLBACK_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string())
}

/// Returns the loopback port the callback listener will bind to.
/// Defaults to 53702; override via `HARNESS_OAUTH_GOOGLE_CALLBACK_PORT`.
pub fn callback_port() -> u16 {
    env::var("HARNESS_OAUTH_GOOGLE_CALLBACK_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn redirect_uri() -> String {
    format!(
        "http://{}:{}{}",
        callback_host(),
        callback_port(),
        CALLBACK_PATH
    )
}

/// Generate a PKCE verifier (random 32 bytes, base64-url, no padding).
fn pkce_verifier() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Compute the PKCE S256 challenge for a given verifier.
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn random_state() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Build the full authorize URL for the OAuth flow.
fn build_authorize_url(cid: &str, challenge: &str, state: &str, redirect_uri: &str) -> String {
    let mut u = url::Url::parse(AUTHORIZE_URL).expect("static url parses");
    u.query_pairs_mut()
        .append_pair("client_id", cid)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", state);
    u.to_string()
}

/// Wait for the OAuth callback HTTP request and parse `code` + `state` from it.
async fn await_callback(listener: TcpListener, expected_state: &str) -> Result<String, OAuthError> {
    let timeout_dur = Duration::from_secs(CALLBACK_TIMEOUT_SECS);
    let (mut socket, _) = timeout(timeout_dur, listener.accept())
        .await
        .map_err(|_| OAuthError::Http("callback timeout".into()))?
        .map_err(|e| OAuthError::Http(format!("accept: {e}")))?;

    let (read_half, mut write_half) = socket.split();
    let mut reader = BufReader::new(read_half);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| OAuthError::Http(format!("read: {e}")))?;

    loop {
        let mut header = String::new();
        let n = reader
            .read_line(&mut header)
            .await
            .map_err(|e| OAuthError::Http(format!("read: {e}")))?;
        if n == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }

    let body =
        b"<html><body><h2>Login complete.</h2><p>You can close this window now.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_half
        .write_all(response.as_bytes())
        .await
        .map_err(|e| OAuthError::Http(format!("write: {e}")))?;
    write_half
        .write_all(body)
        .await
        .map_err(|e| OAuthError::Http(format!("write: {e}")))?;
    let _ = write_half.shutdown().await;

    parse_code_from_request_line(&request_line, expected_state)
}

fn parse_code_from_request_line(line: &str, expected_state: &str) -> Result<String, OAuthError> {
    let mut parts = line.split_whitespace();
    let _method = parts.next();
    let target = parts.next().ok_or(OAuthError::MissingCode)?;
    let parsed = url::Url::parse(&format!("http://localhost{target}"))
        .map_err(|e| OAuthError::Http(format!("bad callback target: {e}")))?;
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            _ => {}
        }
    }
    let state = state.ok_or(OAuthError::InvalidState)?;
    if state != expected_state {
        return Err(OAuthError::InvalidState);
    }
    code.ok_or(OAuthError::MissingCode)
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

fn credential_from_token(resp: TokenResponse) -> Credential {
    let expires_at = resp
        .expires_in
        .map(|secs| chrono::Utc::now().timestamp() + secs);
    let scopes = resp
        .scope
        .as_deref()
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    Credential::OAuth {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at,
        scopes,
        provider_extra: serde_json::Value::Object(resp.extra),
    }
}

async fn exchange_code(
    cid: &str,
    csec: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Credential, OAuthError> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
        ("client_id", cid),
        ("client_secret", csec),
    ];
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await
        .map_err(|e| OAuthError::Http(format!("token request: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(OAuthError::TokenEndpoint(format!("{status}: {body}")));
    }
    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| OAuthError::Http(format!("token decode: {e}")))?;
    Ok(credential_from_token(token))
}

/// Run the full OAuth Authorization Code + PKCE login flow.
pub async fn login(callbacks: OAuthLoginCallbacks) -> Result<Credential, OAuthError> {
    let host = callback_host();
    let port = callback_port();
    let listener = TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| OAuthError::Http(format!("bind {host}:{port}: {e}")))?;

    let cid = client_id()?;
    let csec = client_secret()?;
    let verifier = pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let state = random_state();
    let redirect = redirect_uri();
    let url = build_authorize_url(&cid, &challenge, &state, &redirect);

    (callbacks.on_progress)("waiting for callback".to_string());
    (callbacks.on_open_url)(url);

    let code = await_callback(listener, &state).await?;
    (callbacks.on_progress)("exchanging code for tokens".to_string());
    exchange_code(&cid, &csec, &code, &verifier, &redirect).await
}

/// Refresh an existing OAuth credential. Returns a new credential with the
/// refreshed access token; the refresh token may rotate as well.
pub async fn refresh(credential: Credential) -> Result<Credential, OAuthError> {
    let Credential::OAuth { refresh_token, .. } = credential else {
        return Err(OAuthError::WrongCredential);
    };
    let refresh_token = refresh_token.ok_or(OAuthError::NoRefreshToken)?;
    let cid = client_id()?;
    let csec = client_secret()?;
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", cid.as_str()),
        ("client_secret", csec.as_str()),
    ];
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await
        .map_err(|e| OAuthError::Http(format!("refresh request: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(OAuthError::TokenEndpoint(format!("{status}: {body}")));
    }
    let token: TokenResponse = resp
        .json()
        .await
        .map_err(|e| OAuthError::Http(format!("refresh decode: {e}")))?;
    Ok(credential_from_token(token))
}

/// Reachability placeholder; always returns true today.
pub async fn status() -> bool {
    true
}

/// Register `oauth::google-gemini-cli::*` iii functions on the bus.
///
/// Functions registered:
/// - `oauth::google-gemini-cli::login` — runs the PKCE flow; URL + progress messages
///   logged via `log::info!`. UI integration can wrap this with an
///   ahead-of-time URL fetch in a follow-up. Returns the resulting
///   [`Credential`] as JSON.
/// - `oauth::google-gemini-cli::refresh` — payload is the existing credential JSON;
///   returns the rotated credential.
/// - `oauth::google-gemini-cli::status` — payload is empty; returns `{ ready: bool }`
///   reflecting reachability of the upstream identity provider.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<OAuthFunctionRefs> {
    use iii_sdk::{IIIError, RegisterFunctionMessage};
    use serde_json::{json, Value};

    let login_fn = iii.register_function((
        RegisterFunctionMessage::with_id("oauth::google-gemini-cli::login".to_string())
            .with_description("Run the PKCE flow and return a fresh credential.".into()),
        |_payload: Value| async move {
            let callbacks = OAuthLoginCallbacks {
                on_open_url: Box::new(|url| {
                    log::info!("oauth::google-gemini-cli::login open URL: {url}");
                }),
                on_progress: Box::new(|msg| {
                    log::info!("oauth::google-gemini-cli::login progress: {msg}");
                }),
            };
            let cred = login(callbacks)
                .await
                .map_err(|e| IIIError::Handler(format!("login failed: {e}")))?;
            serde_json::to_value(cred).map_err(|e| IIIError::Handler(e.to_string()))
        },
    ));

    let refresh_fn = iii.register_function((
        RegisterFunctionMessage::with_id("oauth::google-gemini-cli::refresh".to_string())
            .with_description("Refresh an OAuth credential.".into()),
        |payload: Value| async move {
            let cred_value = payload
                .get("credential")
                .cloned()
                .ok_or_else(|| IIIError::Handler("missing required field: credential".into()))?;
            let cred: Credential = serde_json::from_value(cred_value)
                .map_err(|e| IIIError::Handler(format!("invalid credential: {e}")))?;
            let rotated = refresh(cred)
                .await
                .map_err(|e| IIIError::Handler(format!("refresh failed: {e}")))?;
            serde_json::to_value(rotated).map_err(|e| IIIError::Handler(e.to_string()))
        },
    ));

    let status_fn = iii.register_function((
        RegisterFunctionMessage::with_id("oauth::google-gemini-cli::status".to_string())
            .with_description("Probe identity-provider reachability.".into()),
        |_payload: Value| async move {
            let ready = status().await;
            Ok(json!({ "ready": ready }))
        },
    ));

    Ok(OAuthFunctionRefs {
        login: login_fn,
        refresh: refresh_fn,
        status: status_fn,
    })
}

/// Handles returned by [`register_with_iii`].
pub struct OAuthFunctionRefs {
    pub login: iii_sdk::FunctionRef,
    pub refresh: iii_sdk::FunctionRef,
    pub status: iii_sdk::FunctionRef,
}

impl OAuthFunctionRefs {
    pub fn unregister_all(self) {
        for r in [self.login, self.refresh, self.status] {
            r.unregister();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let verifier = "test-verifier-google-abc";
        let challenge = pkce_challenge(verifier);
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn authorize_url_includes_cloud_platform_scope() {
        let url = build_authorize_url(
            "test-client-id",
            "CHAL",
            "STATE",
            "http://127.0.0.1:53702/callback",
        );
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("code_challenge=CHAL"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=STATE"));
        assert!(url.contains("cloud-platform"));
        assert!(url.contains("access_type=offline"));
    }

    #[test]
    fn callback_port_respects_env_var() {
        env::set_var("HARNESS_OAUTH_GOOGLE_CALLBACK_PORT", "65001");
        assert_eq!(callback_port(), 65001);
        env::remove_var("HARNESS_OAUTH_GOOGLE_CALLBACK_PORT");
        assert_eq!(callback_port(), DEFAULT_PORT);
    }

    #[test]
    fn parse_code_validates_state() {
        let line = "GET /callback?code=GCODE&state=GOOD HTTP/1.1\r\n";
        let code = parse_code_from_request_line(line, "GOOD").unwrap();
        assert_eq!(code, "GCODE");
        let bad = parse_code_from_request_line(line, "WRONG");
        assert!(matches!(bad, Err(OAuthError::InvalidState)));
    }
}
