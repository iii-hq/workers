//! GitHub device-flow login surface: `login::start` returns the code to type
//! at github.com/login/device, `login::poll` exchanges the device code for
//! the OAuth token and persists it in iii-state. Operator-only (denied to
//! in-run agents like every direct provider call); the same public OAuth
//! client id the editor plugin ecosystem authenticates with.
use crate::auth;
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The public device-flow OAuth client id used across the Copilot editor
/// plugin ecosystem (copilot.vim and derivatives).
pub const OAUTH_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub const DEFAULT_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
pub const DEFAULT_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Seconds GitHub asks callers to add to their interval after a `slow_down`.
const SLOW_DOWN_BACKOFF_SECS: u64 = 5;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct LoginStartRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LoginStartResponse {
    /// Code the operator types at `verification_uri`.
    pub user_code: String,
    /// Where to type it (github.com/login/device).
    pub verification_uri: String,
    /// Opaque device code to pass to `login::poll`.
    pub device_code: String,
    /// Seconds to wait between polls (GitHub's requested cadence).
    pub interval: u64,
    /// Seconds until the device code expires.
    pub expires_in: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LoginPollRequest {
    /// The `device_code` from `login::start`.
    pub device_code: String,
}

/// Outcome of one poll. `SlowDown` is kept distinct from `Pending` because
/// GitHub asks callers to lengthen their interval when it appears; the
/// requested wait rides along in `retry_after_seconds`.
#[derive(Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    /// Signed in; the credential is stored and discovery has been kicked.
    Ok,
    /// The operator has not finished at the verification URL yet.
    Pending,
    /// Polling too fast — wait `retry_after_seconds` longer before retrying.
    SlowDown,
    /// The device code expired; start again.
    Expired,
    /// The operator rejected the request.
    Denied,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct LoginPollResponse {
    pub status: LoginStatus,
    /// Seconds to add to the poll interval; set when `status` is `slow_down`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
}

fn login_error(message: impl Into<String>) -> Error {
    Error::Remote {
        code: "provider/login_failed".into(),
        message: message.into(),
        stacktrace: None,
    }
}

pub async fn start(
    http: &reqwest::Client,
    device_code_url: &str,
) -> Result<LoginStartResponse, Error> {
    let resp = http
        .post(device_code_url)
        .header("accept", "application/json")
        .form(&[("client_id", OAUTH_CLIENT_ID), ("scope", "read:user")])
        .send()
        .await
        .map_err(|e| login_error(format!("device code request failed: {e}")))?;
    let v: Value = resp
        .json()
        .await
        .map_err(|e| login_error(format!("device code reply not json: {e}")))?;
    let field = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    match (
        field("user_code"),
        field("verification_uri"),
        field("device_code"),
    ) {
        (Some(user_code), Some(verification_uri), Some(device_code)) => Ok(LoginStartResponse {
            user_code,
            verification_uri,
            device_code,
            interval: v.get("interval").and_then(Value::as_u64).unwrap_or(5),
            expires_in: v.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
        }),
        _ => Err(login_error(format!("unexpected device code reply: {v}"))),
    }
}

pub async fn poll(
    iii: &IIIClient,
    http: &reqwest::Client,
    access_token_url: &str,
    device_code: &str,
) -> Result<LoginPollResponse, Error> {
    let resp = http
        .post(access_token_url)
        .header("accept", "application/json")
        .form(&[
            ("client_id", OAUTH_CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|e| login_error(format!("access token request failed: {e}")))?;
    let v: Value = resp
        .json()
        .await
        .map_err(|e| login_error(format!("access token reply not json: {e}")))?;

    if let Some(token) = v.get("access_token").and_then(Value::as_str) {
        auth::store_oauth(iii, token).await?;
        return Ok(LoginPollResponse {
            status: LoginStatus::Ok,
            retry_after_seconds: None,
        });
    }
    let status = match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => LoginStatus::Pending,
        Some("slow_down") => LoginStatus::SlowDown,
        Some("expired_token") => LoginStatus::Expired,
        Some("access_denied") => LoginStatus::Denied,
        Some(other) => return Err(login_error(format!("device flow failed: {other}"))),
        // Never interpolate the reply itself: this branch is reached with a
        // body we did not recognise, which may still carry a credential.
        None => {
            let keys: Vec<&str> = v
                .as_object()
                .map(|o| o.keys().map(String::as_str).collect())
                .unwrap_or_default();
            return Err(login_error(format!(
                "unexpected access token reply (keys: {keys:?})"
            )));
        }
    };
    let retry_after_seconds = (status == LoginStatus::SlowDown).then_some(SLOW_DOWN_BACKOFF_SECS);
    Ok(LoginPollResponse {
        status,
        retry_after_seconds,
    })
}

pub fn make_login_start(
    http: reqwest::Client,
) -> impl Fn(LoginStartRequest) -> BoxFuture<'static, Result<LoginStartResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req| {
        let http = http.clone();
        Box::pin(async move { start(&http, DEFAULT_DEVICE_CODE_URL).await })
    }
}

pub fn make_login_poll(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(LoginPollRequest) -> BoxFuture<'static, Result<LoginPollResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: LoginPollRequest| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move { poll(&iii, &http, DEFAULT_ACCESS_TOKEN_URL, &req.device_code).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn stub(response: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 65536];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}/login")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_parses_the_device_code_reply() {
        let url = stub(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"device_code\":\"dc\",\"user_code\":\"ABCD-1234\",\"verification_uri\":\"https://github.com/login/device\",\"interval\":5,\"expires_in\":899}",
        )
        .await;
        let r = start(&reqwest::Client::new(), &url).await.unwrap();
        assert_eq!(r.user_code, "ABCD-1234");
        assert_eq!(r.verification_uri, "https://github.com/login/device");
        assert_eq!(r.device_code, "dc");
        assert_eq!(r.interval, 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_rejects_malformed_replies() {
        let url = stub(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":\"unauthorized_client\"}",
        )
        .await;
        assert!(start(&reqwest::Client::new(), &url).await.is_err());
    }
}
