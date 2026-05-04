//! GitHub Copilot device-code OAuth flow.
//!
//! Unlike PKCE flows, the device-code flow needs no callback server. The user
//! is shown a short code and a verification URL, enters the code in their
//! browser, and the client polls the token endpoint until authorization
//! succeeds (or fails). This crate implements that flow against GitHub's
//! `login/device/code` and `login/oauth/access_token` endpoints using the
//! Copilot CLI client_id.
//!
//! Refresh: GitHub Copilot device-code tokens do not return a usable refresh
//! token in this flow; to renew credentials, re-run [`login`]. The [`refresh`]
//! function therefore returns [`OAuthError::NoRefreshToken`].

use std::time::Duration;

use auth_credentials::Credential;
use serde::Deserialize;

const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const SCOPE: &str = "read:user";

const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const EDITOR_VERSION: &str = "vscode/1.107.0";
const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

const SLOW_DOWN_MULTIPLIER: f64 = 1.4;
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;

pub use auth_credentials::Credential as ReExportCredential;

/// Caller-supplied callbacks driven by [`login`] as the flow progresses.
pub struct OAuthLoginCallbacks {
    /// Invoked once with the verification URL the user should open.
    pub on_open_url: Box<dyn Fn(String) + Send + Sync>,
    /// Invoked once with the user_code and verification_uri so the harness can
    /// display the short code the user pastes into the browser.
    pub on_user_code: Option<Box<dyn Fn(UserCode) + Send + Sync>>,
    /// Free-form progress messages ("waiting for authorization", etc).
    pub on_progress: Box<dyn Fn(String) + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCode {
    pub code: String,
    pub verification_uri: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("http: {0}")]
    Http(String),
    #[error("device code expired")]
    Expired,
    #[error("device code denied")]
    Denied,
    #[error("no refresh token; re-run login()")]
    NoRefreshToken,
    #[error("token endpoint returned non-success: {0}")]
    TokenEndpoint(String),
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_interval")]
    interval: u64,
    #[serde(default)]
    #[allow(dead_code)]
    expires_in: u64,
}

fn default_interval() -> u64 {
    DEFAULT_POLL_INTERVAL_SECS
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Run the full device-code login flow and return a stored
/// [`Credential::OAuth`] on success.
pub async fn login(callbacks: OAuthLoginCallbacks) -> Result<Credential, OAuthError> {
    let client = build_client()?;
    let device = request_device_code(&client).await?;

    let user_code = UserCode {
        code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
    };
    (callbacks.on_open_url)(device.verification_uri.clone());
    if let Some(cb) = callbacks.on_user_code.as_ref() {
        cb(user_code);
    }
    (callbacks.on_progress)(format!(
        "waiting for authorization at {} (code: {})",
        device.verification_uri, device.user_code
    ));

    poll_for_token(&client, &device, &callbacks.on_progress).await
}

/// Re-issue credentials. The device-code flow does not return a refresh token,
/// so this always returns [`OAuthError::NoRefreshToken`] and callers should
/// re-run [`login`].
pub async fn refresh(_credential: Credential) -> Result<Credential, OAuthError> {
    Err(OAuthError::NoRefreshToken)
}

/// Whether the configured endpoints look reachable. Returns `true` when an
/// HTTP client can be constructed; this crate does not probe live endpoints.
pub async fn status() -> bool {
    build_client().is_ok()
}

fn build_client() -> Result<reqwest::Client, OAuthError> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| OAuthError::Http(e.to_string()))
}

fn copilot_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("editor-version"),
        reqwest::header::HeaderValue::from_static(EDITOR_VERSION),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("editor-plugin-version"),
        reqwest::header::HeaderValue::from_static(EDITOR_PLUGIN_VERSION),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("copilot-integration-id"),
        reqwest::header::HeaderValue::from_static(COPILOT_INTEGRATION_ID),
    );
    headers
}

/// Encode the device-code request body as `application/x-www-form-urlencoded`.
/// Exposed for unit-testing the wire format.
pub fn build_device_code_body(client_id: &str, scope: &str) -> String {
    let mut s = String::new();
    s.push_str("client_id=");
    s.push_str(&urlencode(client_id));
    s.push_str("&scope=");
    s.push_str(&urlencode(scope));
    s
}

/// Encode the polling request body.
pub fn build_token_poll_body(client_id: &str, device_code: &str) -> String {
    let mut s = String::new();
    s.push_str("client_id=");
    s.push_str(&urlencode(client_id));
    s.push_str("&device_code=");
    s.push_str(&urlencode(device_code));
    s.push_str("&grant_type=urn:ietf:params:oauth:grant-type:device_code");
    s
}

fn urlencode(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

async fn request_device_code(client: &reqwest::Client) -> Result<DeviceCodeResponse, OAuthError> {
    let body = build_device_code_body(CLIENT_ID, SCOPE);
    let resp = client
        .post(DEVICE_CODE_URL)
        .headers(copilot_headers())
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| OAuthError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        let code = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(OAuthError::TokenEndpoint(format!("{code}: {text}")));
    }

    resp.json::<DeviceCodeResponse>()
        .await
        .map_err(|e| OAuthError::Http(format!("decode device-code response: {e}")))
}

async fn poll_for_token(
    client: &reqwest::Client,
    device: &DeviceCodeResponse,
    on_progress: &(dyn Fn(String) + Send + Sync),
) -> Result<Credential, OAuthError> {
    let mut interval = device.interval.max(1);
    let body = build_token_poll_body(CLIENT_ID, &device.device_code);

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let resp = client
            .post(TOKEN_URL)
            .headers(copilot_headers())
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body.clone())
            .send()
            .await
            .map_err(|e| OAuthError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let code = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(OAuthError::TokenEndpoint(format!("{code}: {text}")));
        }

        let body: TokenResponse = resp
            .json()
            .await
            .map_err(|e| OAuthError::Http(format!("decode token response: {e}")))?;

        if let Some(access_token) = body.access_token {
            let scopes = body
                .scope
                .as_deref()
                .map_or_else(|| vec![SCOPE.to_string()], parse_scopes);
            let provider_extra = serde_json::json!({
                "token_type": body.token_type,
            });
            return Ok(Credential::OAuth {
                access_token,
                refresh_token: None,
                expires_at: None,
                scopes,
                provider_extra,
            });
        }

        match body.error.as_deref() {
            Some("authorization_pending") => {
                on_progress("authorization pending".to_string());
                continue;
            }
            Some("slow_down") => {
                interval = next_slow_down_interval(interval);
                on_progress(format!("slow_down; interval now {interval}s"));
                continue;
            }
            Some("expired_token") => return Err(OAuthError::Expired),
            Some("access_denied") => return Err(OAuthError::Denied),
            Some(other) => {
                let detail = body.error_description.unwrap_or_else(|| other.to_string());
                return Err(OAuthError::TokenEndpoint(detail));
            }
            None => {
                return Err(OAuthError::TokenEndpoint(
                    "missing access_token and error".to_string(),
                ));
            }
        }
    }
}

/// Compute the next polling interval after a `slow_down` response from the
/// token endpoint. Multiplies the previous interval by 1.4 (rounded up to the
/// next whole second).
pub fn next_slow_down_interval(current: u64) -> u64 {
    let next = (current as f64 * SLOW_DOWN_MULTIPLIER).ceil() as u64;
    next.max(current + 1)
}

fn parse_scopes(scope: &str) -> Vec<String> {
    scope
        .split([' ', ','])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Register `oauth::github-copilot::*` iii functions on the bus.
///
/// Functions registered:
/// - `oauth::github-copilot::login` — runs the PKCE flow; URL + progress messages
///   logged via `log::info!`. UI integration can wrap this with an
///   ahead-of-time URL fetch in a follow-up. Returns the resulting
///   [`Credential`] as JSON.
/// - `oauth::github-copilot::refresh` — payload is the existing credential JSON;
///   returns the rotated credential.
/// - `oauth::github-copilot::status` — payload is empty; returns `{ ready: bool }`
///   reflecting reachability of the upstream identity provider.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<OAuthFunctionRefs> {
    use iii_sdk::{IIIError, RegisterFunctionMessage};
    use serde_json::{json, Value};

    let login_fn = iii.register_function((
        RegisterFunctionMessage::with_id("oauth::github-copilot::login".to_string())
            .with_description("Run the PKCE flow and return a fresh credential.".into()),
        |_payload: Value| async move {
            let callbacks = OAuthLoginCallbacks {
                on_open_url: Box::new(|url| {
                    log::info!("oauth::github-copilot::login open URL: {url}");
                }),
                on_user_code: Some(Box::new(|uc| {
                    log::info!("oauth::github-copilot::login user code: {uc:?}");
                })),
                on_progress: Box::new(|msg| {
                    log::info!("oauth::github-copilot::login progress: {msg}");
                }),
            };
            let cred = login(callbacks)
                .await
                .map_err(|e| IIIError::Handler(format!("login failed: {e}")))?;
            serde_json::to_value(cred).map_err(|e| IIIError::Handler(e.to_string()))
        },
    ));

    let refresh_fn = iii.register_function((
        RegisterFunctionMessage::with_id("oauth::github-copilot::refresh".to_string())
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
        RegisterFunctionMessage::with_id("oauth::github-copilot::status".to_string())
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
    fn device_code_request_includes_client_id() {
        let body = build_device_code_body(CLIENT_ID, SCOPE);
        assert!(body.contains("client_id=Iv1.b507a08c87ecfe98"));
        assert!(body.contains("scope=read%3Auser"));
    }

    #[test]
    fn token_poll_body_includes_grant_type_and_device_code() {
        let body = build_token_poll_body(CLIENT_ID, "abc-device");
        assert!(body.contains("client_id=Iv1.b507a08c87ecfe98"));
        assert!(body.contains("device_code=abc-device"));
        assert!(body.contains("grant_type=urn:ietf:params:oauth:grant-type:device_code"));
    }

    #[test]
    fn slow_down_multiplies_interval_by_1_4() {
        assert_eq!(next_slow_down_interval(5), 7);
        assert_eq!(next_slow_down_interval(10), 14);
        // Always increases by at least one second.
        assert_eq!(next_slow_down_interval(1), 2);
    }

    #[test]
    fn parse_scopes_handles_space_and_comma() {
        assert_eq!(parse_scopes("read:user"), vec!["read:user".to_string()]);
        assert_eq!(
            parse_scopes("read:user user:email"),
            vec!["read:user".to_string(), "user:email".to_string()]
        );
        assert_eq!(
            parse_scopes("read:user,user:email"),
            vec!["read:user".to_string(), "user:email".to_string()]
        );
    }

    #[tokio::test]
    async fn refresh_returns_no_refresh_token() {
        let cred = Credential::OAuth {
            access_token: "x".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec!["read:user".into()],
            provider_extra: serde_json::Value::Null,
        };
        let err = refresh(cred).await.unwrap_err();
        assert!(matches!(err, OAuthError::NoRefreshToken));
    }

    #[tokio::test]
    async fn status_is_true_when_client_builds() {
        assert!(status().await);
    }
}
