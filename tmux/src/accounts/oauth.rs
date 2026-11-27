//! Claude Code's token endpoint, for a renewal (chat-projects spec
//! §4.2.1, D141): the request Claude Code 2.1.332 makes when its access
//! token expires within five minutes (`Qfe()` and its caller in its auth
//! chunk), and how it keeps the answer (`BC()`).
//!
//! `POST https://platform.claude.com/v1/oauth/token`, JSON:
//! `{grant_type: "refresh_token", refresh_token, client_id, scope}`, where
//! `client_id` is the sign-in's own `clientId` or Claude Code's, and
//! `scope` is Claude Code's default scopes plus the optional ones the
//! sign-in had, for a Claude.ai sign-in without a client of its own, or
//! else the sign-in's own scopes. The answer is `{access_token,
//! refresh_token?, expires_in (s), refresh_token_expires_in? (s), scope}`;
//! a missing `refresh_token` keeps the old one. Anthropic rotates refresh
//! tokens, so an answer must be kept once it came: a renewal runs to its
//! end in a task of its own.
//!
//! The refresh token goes to that endpoint and nowhere else; nothing here
//! logs a token.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::credentials::Secret;

/// The token endpoint's path (Claude Code's `TOKEN_URL`).
pub const TOKEN_PATH: &str = "/v1/oauth/token";
/// Claude Code's OAuth client (its `CLIENT_ID`).
pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// The scope that makes a sign-in a Claude.ai one (`rV()`, `kSt()`).
pub const INFERENCE_SCOPE: &str = "user:inference";
/// Claude Code's default scopes for a Claude.ai sign-in (`uRn()`, with its
/// plugins scope registered).
pub const DEFAULT_SCOPES: [&str; 6] = [
    "user:profile",
    INFERENCE_SCOPE,
    "user:sessions:claude_code",
    "user:mcp_servers",
    "user:file_upload",
    "user:plugins",
];
/// The optional scopes Claude Code asks for again when the sign-in has
/// them (`w5s()`).
pub const KEPT_SCOPES: [&str; 3] = [
    "user:projects:read",
    "user:projects:write",
    "user:privacy_consents:write",
];
/// How long the request may take (Claude Code allows 30 s; the worker holds
/// Claude Code's locks meanwhile, touched, so it asks for less).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// How soon before its expiry a token is renewed: Claude Code's `Lv()`
/// (`now + 300000 >= expiresAt`).
pub const RENEW_BEFORE_MS: u64 = 5 * 60_000;

/// Whether a token expiring at `expires_at_ms` is renewed at `now_ms`, as
/// Claude Code decides it (`Lv()`). A token without an expiry never is:
/// Claude Code uses it as it is, and renews it only after the API refused
/// it (a 401), which the worker never sees.
pub fn due(expires_at_ms: Option<u64>, now_ms: u64) -> bool {
    expires_at_ms.is_some_and(|at| now_ms.saturating_add(RENEW_BEFORE_MS) >= at)
}

/// Whether Claude Code renews the sign-in `oauth` (a store's
/// `claudeAiOauth`) at all: a Claude.ai one, with the inference scope or a
/// subscription (`not_refreshable` otherwise).
pub fn refreshable(oauth: &Map<String, Value>) -> bool {
    let inference = stored_scopes(oauth)
        .is_some_and(|scopes| scopes.iter().any(|scope| scope == INFERENCE_SCOPE));
    let subscription = oauth
        .get("subscriptionType")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.is_empty());
    inference || subscription
}

/// The scopes a sign-in's `claudeAiOauth` holds.
fn stored_scopes(oauth: &Map<String, Value>) -> Option<Vec<String>> {
    let scopes = oauth.get("scopes")?.as_array()?;
    Some(
        scopes
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

/// What a renewal of `oauth` (a store's `claudeAiOauth`) asks for, as
/// Claude Code does: its default scopes plus the optional ones the
/// sign-in had, for a Claude.ai sign-in (one with the inference scope or a
/// subscription) without a client of its own; otherwise the sign-in's own
/// scopes, or the defaults when it has none. The second value is what to
/// ask once more after an `invalid_scope` refusal of the first: the
/// sign-in's own scopes, when Claude Code would retry with them.
pub fn scopes_to_ask(oauth: &Map<String, Value>) -> (Vec<String>, Option<Vec<String>>) {
    let stored = stored_scopes(oauth);
    let has_inference = stored
        .as_ref()
        .is_some_and(|scopes| scopes.iter().any(|s| s == INFERENCE_SCOPE));
    let subscription = oauth
        .get("subscriptionType")
        .is_some_and(|value| !value.is_null());
    let own_client = oauth
        .get("clientId")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    let claude_ai = (has_inference || subscription) && !own_client;
    let defaults = || DEFAULT_SCOPES.iter().map(|s| s.to_string()).collect();
    if claude_ai {
        let mut asked: Vec<String> = defaults();
        for scope in stored.iter().flatten() {
            if KEPT_SCOPES.contains(&scope.as_str()) && !asked.contains(scope) {
                asked.push(scope.clone());
            }
        }
        let fallback = stored.filter(|_| has_inference);
        (asked, fallback)
    } else {
        match stored.filter(|scopes| !scopes.is_empty()) {
            Some(scopes) => (scopes, None),
            None => (defaults(), None),
        }
    }
}

/// The client a renewal names: the sign-in's own, else Claude Code's.
pub fn client_id(oauth: &Map<String, Value>) -> String {
    oauth
        .get("clientId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .unwrap_or(CLIENT_ID)
        .to_string()
}

/// A renewal's answer.
#[derive(Debug, Clone)]
pub struct Renewed {
    pub access_token: Secret,
    pub refresh_token: Secret,
    pub expires_at_ms: u64,
    pub refresh_token_expires_at_ms: Option<u64>,
    /// The scopes granted; empty when the answer named none.
    pub scopes: Vec<String>,
}

/// Why a renewal failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewError {
    /// Anthropic said the refresh token is dead (a 400 or 401 with
    /// `invalid_grant`, Claude Code's `tY()`): the sign-in has to be made
    /// again.
    Refused,
    /// Anthropic refused the scopes asked for (a 400 with `invalid_scope`).
    InvalidScope,
    /// Anthropic put the account on hold (`account_on_hold`).
    OnHold,
    /// No answer: the network, a timeout.
    Network,
    /// Any other status: a 5xx, a 429, another 400, or a 401 or 403 that
    /// is no OAuth answer (a proxy's).
    Unexpected(u16),
    /// An answer without a token or an expiry.
    Unreadable,
    /// The endpoint is no host a refresh token may go to.
    NotAllowed,
}

impl RenewError {
    /// Whether the refresh token is dead (sign in again), not just this
    /// attempt.
    pub fn refused(&self) -> bool {
        matches!(self, Self::Refused)
    }

    pub fn sentence(&self) -> String {
        match self {
            Self::Refused => "Anthropic refused to renew the sign-in; sign in again.".into(),
            Self::InvalidScope => "Anthropic refused the permissions the sign-in asks for.".into(),
            Self::OnHold => {
                "Anthropic put the account on hold; see https://claude.ai/restricted.".into()
            }
            Self::Network => "Anthropic's sign-in server could not be reached.".into(),
            Self::Unexpected(status) => {
                format!("Anthropic's sign-in server answered {status}.")
            }
            Self::Unreadable => {
                "Anthropic's sign-in server gave an answer that can't be read.".into()
            }
            Self::NotAllowed => "The sign-in server is not one a sign-in may be sent to.".into(),
        }
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<f64>,
    refresh_token_expires_in: Option<f64>,
    scope: Option<String>,
}

/// An OAuth error answer: `{error: "<code>" | {type: "<code>"},
/// error_description?}` (Claude Code's `xi()`).
#[derive(Deserialize)]
struct ErrorAnswer {
    error: Option<Value>,
    error_description: Option<String>,
}

impl ErrorAnswer {
    fn code(&self) -> Option<&str> {
        match self.error.as_ref()? {
            Value::String(code) => Some(code),
            Value::Object(error) => error.get("type").and_then(Value::as_str),
            _ => None,
        }
    }
}

/// What a refusal (`status`, body `text`) says, as Claude Code reads it.
fn refusal(status: u16, text: &str) -> RenewError {
    let Ok(answer) = serde_json::from_str::<ErrorAnswer>(text) else {
        return RenewError::Unexpected(status);
    };
    if answer.error_description.as_deref() == Some("account_on_hold") {
        return RenewError::OnHold;
    }
    match (status, answer.code()) {
        (400 | 401, Some("invalid_grant")) => RenewError::Refused,
        (400, Some("invalid_scope")) => RenewError::InvalidScope,
        // Any other code (`invalid_request`, `invalid_client`), or a 401
        // or 403 with no OAuth answer, says nothing of the sign-in.
        _ => RenewError::Unexpected(status),
    }
}

/// The HTTP client for the token endpoint.
#[derive(Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
    origin: String,
}

impl OAuthClient {
    /// `None` when the client can't be built as it must be (no proxy, no
    /// redirect, a timeout): then nothing is renewed, rather than renewed
    /// through a client that could carry the refresh token elsewhere.
    pub fn new(origin: &str) -> Option<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent(concat!("iii-tmux/", env!("CARGO_PKG_VERSION")))
            .build()
            .inspect_err(
                |error| tracing::warn!(%error, "token client not built; nothing is renewed"),
            )
            .ok()?;
        Some(Self {
            http,
            origin: origin.to_string(),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// One renewal of `refresh_token`, asking for `scopes` on behalf of
    /// `client_id`, at `now_ms`.
    pub async fn renew(
        &self,
        refresh_token: &Secret,
        client_id: &str,
        scopes: &[String],
        now_ms: u64,
    ) -> Result<Renewed, RenewError> {
        let url = super::paths::allowed_oauth_origin(&self.origin)
            .and_then(|origin| origin.join(TOKEN_PATH).ok())
            .ok_or(RenewError::NotAllowed)?;
        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token.expose(),
            "client_id": client_id,
            "scope": scopes.join(" "),
        });
        let response = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|_| RenewError::Network)?;
        let status = response.status().as_u16();
        let text = response.text().await.map_err(|_| RenewError::Network)?;
        match status {
            200 => parse_answer(&text, refresh_token, now_ms),
            400 | 401 | 403 => Err(refusal(status, &text)),
            other => Err(RenewError::Unexpected(other)),
        }
    }
}

fn parse_answer(text: &str, posted: &Secret, now_ms: u64) -> Result<Renewed, RenewError> {
    let answer: TokenAnswer = serde_json::from_str(text).map_err(|_| RenewError::Unreadable)?;
    let access_token = answer
        .access_token
        .filter(|token| !token.trim().is_empty())
        .ok_or(RenewError::Unreadable)?;
    let expires_in = answer
        .expires_in
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
        .ok_or(RenewError::Unreadable)?;
    let in_ms = |seconds: f64| now_ms.saturating_add((seconds * 1000.0) as u64);
    Ok(Renewed {
        access_token: Secret::new(access_token),
        refresh_token: answer
            .refresh_token
            .filter(|token| !token.trim().is_empty())
            .map(Secret::new)
            .unwrap_or_else(|| posted.clone()),
        expires_at_ms: in_ms(expires_in),
        refresh_token_expires_at_ms: answer
            .refresh_token_expires_in
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(in_ms),
        scopes: answer
            .scope
            .map(|scope| {
                scope
                    .split(' ')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// `oauth` (a store's `claudeAiOauth`) after `renewed`, as Claude Code
/// keeps a renewal (`BC()` in refresh mode): the new access token, refresh
/// token and expiry; the scopes granted (the old ones when the answer named
/// none); the refresh token's own expiry only when the old sign-in had
/// one, never later than it; everything else as it was (subscription,
/// rate-limit tier, client, account and organisation).
pub fn renewed_oauth(oauth: &Map<String, Value>, renewed: &Renewed) -> Map<String, Value> {
    let mut out = oauth.clone();
    out.insert(
        "accessToken".into(),
        Value::String(renewed.access_token.expose().to_string()),
    );
    out.insert(
        "refreshToken".into(),
        Value::String(renewed.refresh_token.expose().to_string()),
    );
    out.insert("expiresAt".into(), Value::from(renewed.expires_at_ms));
    let old_expiry = oauth.get("refreshTokenExpiresAt").and_then(Value::as_u64);
    match old_expiry {
        Some(old) => {
            let kept = renewed
                .refresh_token_expires_at_ms
                .map_or(old, |new| new.min(old));
            out.insert("refreshTokenExpiresAt".into(), Value::from(kept));
        }
        None => {
            out.remove("refreshTokenExpiresAt");
        }
    }
    if !renewed.scopes.is_empty() {
        out.insert(
            "scopes".into(),
            Value::Array(renewed.scopes.iter().cloned().map(Value::String).collect()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn oauth(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn renewal_is_due_five_minutes_before_expiry() {
        let now = 1_000_000_000;
        assert!(due(Some(now + RENEW_BEFORE_MS), now));
        assert!(due(Some(now - 1), now));
        assert!(!due(Some(now + RENEW_BEFORE_MS + 1), now));
        assert!(!due(None, now));
    }

    #[test]
    fn scopes_follow_claude_code() {
        // A Claude.ai sign-in: the defaults, plus the optional ones it had.
        let (asked, fallback) = scopes_to_ask(&oauth(serde_json::json!({
            "scopes": ["user:inference", "user:profile", "user:projects:read", "org:other"],
            "subscriptionType": "max"
        })));
        assert_eq!(
            asked,
            [
                "user:profile",
                "user:inference",
                "user:sessions:claude_code",
                "user:mcp_servers",
                "user:file_upload",
                "user:plugins",
                "user:projects:read"
            ]
        );
        assert_eq!(
            fallback.unwrap(),
            [
                "user:inference",
                "user:profile",
                "user:projects:read",
                "org:other"
            ]
        );
        // A subscription without the inference scope: defaults, no fallback.
        let (asked, fallback) = scopes_to_ask(&oauth(
            serde_json::json!({"scopes": [], "subscriptionType": "pro"}),
        ));
        assert_eq!(asked.len(), DEFAULT_SCOPES.len());
        assert!(fallback.is_none());
        // A client of its own: its own scopes, and its own client id.
        let own = oauth(serde_json::json!({
            "scopes": ["user:inference"], "clientId": "custom-client"
        }));
        assert_eq!(
            scopes_to_ask(&own),
            (vec!["user:inference".to_string()], None)
        );
        assert_eq!(client_id(&own), "custom-client");
        // Nothing stored: the defaults, Claude Code's client.
        let bare = oauth(serde_json::json!({}));
        assert_eq!(scopes_to_ask(&bare).0.len(), DEFAULT_SCOPES.len());
        assert_eq!(client_id(&bare), CLIENT_ID);
    }

    #[test]
    fn an_answer_is_kept_as_claude_code_keeps_it() {
        let posted = Secret::new("rt-old".into());
        let now = 1_000_000;
        let renewed = parse_answer(
            r#"{"access_token":"at-new","expires_in":3600,"scope":"user:inference user:profile"}"#,
            &posted,
            now,
        )
        .unwrap();
        assert_eq!(
            renewed.refresh_token.expose(),
            "rt-old",
            "kept when not rotated"
        );
        assert_eq!(renewed.expires_at_ms, now + 3_600_000);
        let old = oauth(serde_json::json!({
            "accessToken": "at-old", "refreshToken": "rt-old", "expiresAt": 5,
            "scopes": ["user:inference"], "subscriptionType": "max",
            "rateLimitTier": "default_claude_max_20x", "accountUuid": "u-1",
            "refreshTokenExpiresAt": 9_000_000
        }));
        let rotated = parse_answer(
            r#"{"access_token":"at-new","refresh_token":"rt-new","expires_in":60,
                "refresh_token_expires_in":86400,"scope":""}"#,
            &posted,
            now,
        )
        .unwrap();
        let kept = renewed_oauth(&old, &rotated);
        assert_eq!(kept["accessToken"], "at-new");
        assert_eq!(kept["refreshToken"], "rt-new");
        assert_eq!(kept["expiresAt"], now + 60_000);
        // Never later than the old refresh-token expiry.
        assert_eq!(kept["refreshTokenExpiresAt"], 9_000_000);
        // No scope named: the old ones stay.
        assert_eq!(kept["scopes"], serde_json::json!(["user:inference"]));
        for key in ["subscriptionType", "rateLimitTier", "accountUuid"] {
            assert_eq!(kept[key], old[key], "{key} kept");
        }
        // Without an old refresh-token expiry, none is kept.
        let mut no_expiry = old.clone();
        no_expiry.remove("refreshTokenExpiresAt");
        assert!(!renewed_oauth(&no_expiry, &rotated).contains_key("refreshTokenExpiresAt"));
        assert_eq!(
            parse_answer(r#"{"expires_in":60}"#, &posted, now).unwrap_err(),
            RenewError::Unreadable
        );
        assert_eq!(
            parse_answer(r#"{"access_token":"x"}"#, &posted, now).unwrap_err(),
            RenewError::Unreadable
        );
    }

    #[test]
    fn a_refresh_token_only_goes_to_claude_code_s_endpoint_or_loopback() {
        let client = OAuthClient::new("https://evil.example").unwrap();
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(client.renew(&Secret::new("rt".into()), CLIENT_ID, &[], 0));
        assert_eq!(outcome.unwrap_err(), RenewError::NotAllowed);
    }

    #[test]
    fn refusals_are_read_as_claude_code_reads_them() {
        // Dead: a 400 or 401 with `invalid_grant`, either shape of `error`.
        assert_eq!(
            refusal(400, r#"{"error":"invalid_grant"}"#),
            RenewError::Refused
        );
        assert_eq!(
            refusal(401, r#"{"error":{"type":"invalid_grant","message":"x"}}"#),
            RenewError::Refused
        );
        // A 403 is never a dead token, nor a 401 or 403 a proxy answered.
        assert_eq!(
            refusal(403, r#"{"error":"invalid_grant"}"#),
            RenewError::Unexpected(403)
        );
        assert_eq!(
            refusal(401, "<html>Unauthorized</html>"),
            RenewError::Unexpected(401)
        );
        assert_eq!(refusal(403, ""), RenewError::Unexpected(403));
        assert_eq!(
            refusal(400, r#"{"error":"invalid_scope"}"#),
            RenewError::InvalidScope
        );
        assert_eq!(
            refusal(401, r#"{"error":"invalid_scope"}"#),
            RenewError::Unexpected(401)
        );
        assert_eq!(
            refusal(400, r#"{"error":"invalid_client"}"#),
            RenewError::Unexpected(400)
        );
        assert_eq!(
            refusal(
                401,
                r#"{"error":"invalid_grant","error_description":"account_on_hold"}"#
            ),
            RenewError::OnHold
        );
        assert!(RenewError::Refused.refused());
        assert!(!RenewError::Unexpected(401).refused());
    }

    #[test]
    fn only_a_claude_ai_sign_in_is_renewed() {
        assert!(refreshable(&oauth(json!({"scopes": ["user:inference"]}))));
        assert!(refreshable(&oauth(json!({"subscriptionType": "pro"}))));
        assert!(!refreshable(&oauth(
            json!({"scopes": ["org:create_api_key"]})
        )));
        assert!(!refreshable(&oauth(json!({"subscriptionType": null}))));
        assert!(!refreshable(&oauth(json!({}))));
    }
}
