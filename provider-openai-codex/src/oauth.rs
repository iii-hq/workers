//! Native ChatGPT device OAuth HTTP adapter; credential storage belongs to the caller.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::{account_id_from_access_token, expires_at_from_access_token};

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_INTERVAL: u64 = 5;
const DEVICE_LIFETIME_SECS: i64 = 900;

#[derive(Clone, Serialize, Deserialize)]
pub struct Credential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub account_id: String,
    pub expires_at: i64,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("id_token", &self.id_token.as_ref().map(|_| "[REDACTED]"))
            .field("account_id", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone)]
pub struct DeviceCode {
    pub device_auth_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_at: i64,
}

impl fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCode")
            .field("device_auth_id", &"[REDACTED]")
            .field("user_code", &"[REDACTED]")
            .field("interval", &self.interval)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum PollOutcome {
    Pending { retry_after_secs: u64 },
    Authorized(Credential),
}

#[derive(Clone, Debug)]
pub struct OAuthError {
    pub code: String,
    pub message: String,
    /// Whether to stop this operation. This alone does not invalidate stored credentials.
    pub permanent: bool,
    pub retry_after_secs: Option<u64>,
}

impl OAuthError {
    /// For a refresh failure, whether the issuer definitively rejected the stored session.
    /// Use this to mark a session as needing login; `permanent` also covers protocol failures
    /// and device-login failures that say nothing about the validity of stored credentials.
    /// Callers handling a new device login must leave an existing session intact.
    pub fn invalidates_session(&self) -> bool {
        self.permanent
            && matches!(
                self.code.as_str(),
                "invalid_grant"
                    | "revoked"
                    | "token_revoked"
                    | "refresh_token_revoked"
                    | "refresh_token_reused"
                    | "refresh_token_expired"
                    | "expired_token"
            )
    }
}

#[derive(Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
    issuer: String,
}

impl OAuthClient {
    /// Use a client with redirects disabled so token POSTs stay on the issuer.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            issuer: ISSUER.into(),
        }
    }

    #[cfg(test)]
    pub fn with_issuer(http: reqwest::Client, issuer: String) -> Self {
        Self {
            http,
            issuer: issuer.trim_end_matches('/').into(),
        }
    }

    pub async fn start(&self) -> Result<DeviceCode, OAuthError> {
        let response = self
            .post("/api/accounts/deviceauth/usercode")
            .json(&json!({"client_id": CLIENT_ID}))
            .send()
            .await
            .map_err(transport_error)?;
        // Only the initial user-code endpoint uses 404 to signal disabled login.
        // The polling endpoint uses the same status for a pending authorization.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            let mut err = device_login_disabled();
            err.retry_after_secs = retry_after(&response);
            return Err(err);
        }
        let retry_after = retry_after(&response).unwrap_or(0);
        let body: UserCodeResponse = self.decode(response).await?;
        if body.device_auth_id.trim().is_empty() || body.user_code.trim().is_empty() {
            return Err(invalid_response());
        }
        let interval = body
            .interval
            .as_u64()
            .or_else(|| body.interval.as_str().and_then(|s| s.trim().parse().ok()))
            .unwrap_or(DEFAULT_INTERVAL)
            .max(1)
            .max(retry_after);
        Ok(DeviceCode {
            device_auth_id: body.device_auth_id,
            user_code: body.user_code,
            verification_uri: format!("{}/codex/device", self.issuer),
            interval,
            expires_at: now() + DEVICE_LIFETIME_SECS,
        })
    }

    /// Perform one poll. The caller must wait at least the returned retry delay.
    pub async fn poll(&self, device: &DeviceCode) -> Result<PollOutcome, OAuthError> {
        if device.expires_at <= now() {
            return Err(error(
                "expired_token",
                "The device sign-in code expired. Start sign-in again.",
                true,
            ));
        }
        self.poll_once(device).await.map_err(|mut err| {
            if !err.permanent {
                err.retry_after_secs = Some(
                    err.retry_after_secs
                        .unwrap_or(0)
                        .max(device.interval)
                        .max(1),
                );
            }
            err
        })
    }

    async fn poll_once(&self, device: &DeviceCode) -> Result<PollOutcome, OAuthError> {
        let response = self
            .post("/api/accounts/deviceauth/token")
            .json(&json!({"device_auth_id": device.device_auth_id, "user_code": device.user_code}))
            .send()
            .await
            .map_err(transport_error)?;
        if matches!(response.status().as_u16(), 403 | 404) {
            return Ok(PollOutcome::Pending {
                retry_after_secs: device
                    .interval
                    .max(1)
                    .max(retry_after(&response).unwrap_or(0)),
            });
        }
        let auth: AuthorizationResponse = self.decode(response).await?;
        if auth.authorization_code.trim().is_empty()
            || auth.code_verifier.trim().is_empty()
            || auth.code_challenge.trim().is_empty()
        {
            return Err(invalid_response());
        }
        let redirect_uri = format!("{}/deviceauth/callback", self.issuer);
        let response = self
            .post("/oauth/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", auth.authorization_code.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("client_id", CLIENT_ID),
                ("code_verifier", auth.code_verifier.as_str()),
            ])
            .send()
            .await
            .map_err(transport_error)?;
        let tokens: TokenResponse = self.decode(response).await?;
        Ok(PollOutcome::Authorized(tokens.into_credential(None)?))
    }

    pub async fn refresh(&self, credential: &Credential) -> Result<Credential, OAuthError> {
        let refresh_token = credential
            .refresh_token
            .as_deref()
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| {
                error(
                    "missing_refresh_token",
                    "Sign in again to obtain a refresh token.",
                    true,
                )
            })?;
        let response = self.post("/oauth/token")
            .json(&json!({"grant_type": "refresh_token", "client_id": CLIENT_ID, "refresh_token": refresh_token}))
            .send().await.map_err(transport_error)?;
        let tokens: TokenResponse = self.decode(response).await?;
        tokens.into_credential(Some(credential))
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}{path}", self.issuer))
            .timeout(REQUEST_TIMEOUT)
    }

    async fn decode<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, OAuthError> {
        let status = response.status();
        let retry_after_secs = retry_after(&response);
        // Status takes priority over an intermediary's untrusted error body.
        let transient = match status.as_u16() {
            429 => Some(error(
                "rate_limited",
                "OpenAI sign-in is rate limited. Try again later.",
                false,
            )),
            500..=599 => Some(error(
                "service_unavailable",
                "OpenAI sign-in is temporarily unavailable.",
                false,
            )),
            408 => Some(error(
                "request_timeout",
                "The OpenAI sign-in request timed out.",
                false,
            )),
            _ => None,
        };
        if let Some(mut err) = transient {
            err.retry_after_secs = retry_after_secs;
            return Err(err);
        }
        let body = response.bytes().await.map_err(|cause| {
            let mut err = transport_error(cause);
            err.retry_after_secs = retry_after_secs;
            err
        })?;
        if !status.is_success() {
            let mut err = provider_error(&body);
            err.retry_after_secs = retry_after_secs;
            return Err(err);
        }
        serde_json::from_slice(&body).map_err(|_| invalid_response())
    }
}

#[derive(Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Value,
}

#[derive(Deserialize)]
struct AuthorizationResponse {
    authorization_code: String,
    code_verifier: String,
    code_challenge: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

impl TokenResponse {
    fn into_credential(self, previous: Option<&Credential>) -> Result<Credential, OAuthError> {
        if previous.is_none()
            && !self
                .refresh_token
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty())
        {
            return Err(error(
                "missing_refresh_token",
                "OpenAI did not return a refresh token. Sign in again to enable session renewal.",
                true,
            ));
        }
        if [&self.access_token, &self.refresh_token, &self.id_token]
            .iter()
            .any(|token| token.as_ref().is_some_and(|token| token.trim().is_empty()))
        {
            return Err(invalid_response());
        }
        let access_token = self
            .access_token
            .or_else(|| previous.map(|c| c.access_token.clone()))
            .ok_or_else(|| {
                error(
                    "missing_access_token",
                    "OpenAI did not return an OAuth access token.",
                    true,
                )
            })?;
        let refresh_token = self
            .refresh_token
            .or_else(|| previous.and_then(|c| c.refresh_token.clone()));
        let id_token = self
            .id_token
            .or_else(|| previous.and_then(|c| c.id_token.clone()));
        // These unsigned claims are metadata, not verification of the JWT signature.
        // An API key or opaque token cannot supply the required OAuth metadata.
        if access_token.starts_with("sk-") || access_token.split('.').count() != 3 {
            return Err(error(
                "invalid_access_token",
                "Expected a ChatGPT OAuth access token, not an API key.",
                true,
            ));
        }
        let expires_at = expires_at_from_access_token(&access_token)
            .filter(|expiry| *expiry > 0)
            .ok_or_else(|| {
                error(
                    "missing_expiry",
                    "The OAuth access token is missing its expiry.",
                    true,
                )
            })?;
        let account_id = account_id_from_access_token(&access_token)
            .filter(|account| !account.trim().is_empty())
            .or_else(|| {
                id_token
                    .as_deref()
                    .and_then(account_id_from_access_token)
                    .filter(|account| !account.trim().is_empty())
            })
            .ok_or_else(|| {
                error(
                    "missing_account_id",
                    "The OAuth tokens are missing the ChatGPT account ID.",
                    true,
                )
            })?;
        if previous.is_some_and(|old| old.account_id != account_id) {
            return Err(error(
                "account_mismatch",
                "The refreshed tokens belong to a different ChatGPT account. Sign in again.",
                true,
            ));
        }
        Ok(Credential {
            access_token,
            refresh_token,
            id_token,
            account_id,
            expires_at,
        })
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    let value = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds.max(1));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let remaining = date.duration_since(SystemTime::now()).unwrap_or_default();
    Some(
        remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() > 0))
            .max(1),
    )
}

fn error(code: &'static str, message: &'static str, permanent: bool) -> OAuthError {
    OAuthError {
        code: code.into(),
        message: message.into(),
        permanent,
        retry_after_secs: None,
    }
}

fn invalid_response() -> OAuthError {
    error(
        "invalid_response",
        "OpenAI returned an invalid OAuth response.",
        true,
    )
}

fn device_login_disabled() -> OAuthError {
    error(
        "device_login_disabled",
        "Enable device code login in ChatGPT security settings or ask your workspace administrator.",
        true,
    )
}

fn transport_error(cause: reqwest::Error) -> OAuthError {
    if cause.is_timeout() {
        error(
            "request_timeout",
            "The OpenAI sign-in request timed out.",
            false,
        )
    } else {
        error(
            "network_error",
            "Could not reach the OpenAI sign-in service.",
            false,
        )
    }
}

fn provider_error(body: &[u8]) -> OAuthError {
    let value = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
    let code = value
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/code").and_then(Value::as_str))
        .or_else(|| value.get("code").and_then(Value::as_str));
    // Never propagate descriptions or arbitrary provider codes, which can echo secrets.
    let safe_code = match code {
        Some("device_login_disabled" | "device_auth_disabled" | "device_code_login_disabled") => {
            return device_login_disabled();
        }
        Some("invalid_grant") => "invalid_grant",
        Some("revoked") => "revoked",
        Some("token_revoked") => "token_revoked",
        Some("refresh_token_revoked") => "refresh_token_revoked",
        Some("refresh_token_reused") => "refresh_token_reused",
        Some("refresh_token_expired") => "refresh_token_expired",
        Some("expired_token") => "expired_token",
        Some("access_denied") => "access_denied",
        Some("invalid_client") => "invalid_client",
        _ => return error("http_error", "OpenAI rejected the OAuth request.", true),
    };
    error(
        safe_code,
        "OpenAI rejected the sign-in credentials. Sign in again.",
        true,
    )
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
