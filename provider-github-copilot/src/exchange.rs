//! GitHub OAuth token → short-lived Copilot bearer. The exchange endpoint
//! (`GET copilot_internal/v2/token`) returns the bearer, its expiry
//! (~25 minutes), and the API endpoint to call — the reply names the
//! endpoint, the worker never assumes it. The bearer is cached in-memory and
//! re-exchanged proactively inside a margin, so stream calls almost never pay
//! the exchange round-trip.
use crate::auth::GithubCredential;
use crate::request::client_headers;
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub const DEFAULT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
pub const DEFAULT_API_URL: &str = "https://api.githubcopilot.com/chat/completions";

/// Re-exchange when the bearer expires within this margin.
const EXPIRY_MARGIN_SECS: i64 = 120;

#[derive(Debug, Clone)]
pub struct CopilotBearer {
    pub token: String,
    pub expires_at: i64, // seconds since epoch; 0 = unknown (never refresh)
    /// Chat Completions endpoint the exchange reply named, when it did.
    pub api_url: Option<String>,
}

impl CopilotBearer {
    fn near_expiry(&self) -> bool {
        self.expires_at != 0 && self.expires_at <= crate::now_ms() / 1000 + EXPIRY_MARGIN_SECS
    }
}

/// Process-wide bearer cache shared by streaming and discovery.
#[derive(Clone, Default)]
pub struct BearerCache(Arc<Mutex<Option<CopilotBearer>>>);

impl BearerCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_fresh(&self) -> Option<CopilotBearer> {
        let guard = self.0.lock().expect("bearer cache lock");
        guard.clone().filter(|b| !b.near_expiry())
    }

    fn put(&self, bearer: CopilotBearer) {
        *self.0.lock().expect("bearer cache lock") = Some(bearer);
    }

    /// Drop the cached bearer (a 401 mid-stream means it died early).
    pub fn invalidate(&self) {
        *self.0.lock().expect("bearer cache lock") = None;
    }
}

/// Parse the exchange reply: `{ "token", "expires_at", "endpoints": { "api" } }`.
pub fn parse_exchange_reply(v: &Value) -> Option<CopilotBearer> {
    let token = v.get("token").and_then(Value::as_str)?.to_string();
    let expires_at = v.get("expires_at").and_then(Value::as_i64).unwrap_or(0);
    let api_url = v
        .pointer("/endpoints/api")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|base| format!("{}/chat/completions", base.trim_end_matches('/')));
    Some(CopilotBearer {
        token,
        expires_at,
        api_url,
    })
}

#[derive(Debug)]
pub enum ExchangeError {
    /// 401/403 — the GitHub token is revoked or has no Copilot access.
    Unauthorized(String),
    /// Everything else (network, 5xx, malformed reply).
    Transient(String),
}

async fn exchange(
    http: &reqwest::Client,
    token_url: &str,
    oauth: &str,
) -> Result<CopilotBearer, ExchangeError> {
    let mut req = http
        .get(token_url)
        .header("authorization", format!("token {oauth}"));
    for (name, value) in client_headers() {
        req = req.header(name, value);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ExchangeError::Transient(format!("token exchange failed: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status == 401 || status == 403 {
        // Bounded: the reply is GitHub's, but an error body is not a place to
        // spill an arbitrary upstream payload into logs and error frames.
        let detail: String = body.chars().take(200).collect();
        return Err(ExchangeError::Unauthorized(format!(
            "copilot token exchange rejected (http {status}): {detail}"
        )));
    }
    if !(200..300).contains(&status) {
        return Err(ExchangeError::Transient(format!(
            "copilot token exchange http {status}"
        )));
    }
    serde_json::from_str::<Value>(&body)
        .ok()
        .as_ref()
        .and_then(parse_exchange_reply)
        .ok_or_else(|| ExchangeError::Transient("token exchange reply not json".into()))
}

/// A fresh bearer for one upstream call: cache hit inside the margin,
/// otherwise exchange (or pass a ready bearer through untouched).
pub async fn fresh_bearer(
    http: &reqwest::Client,
    cache: &BearerCache,
    credential: &GithubCredential,
    token_url: &str,
) -> Result<CopilotBearer, ExchangeError> {
    match credential {
        GithubCredential::Bearer(token) => Ok(CopilotBearer {
            token: token.clone(),
            expires_at: 0,
            api_url: None,
        }),
        GithubCredential::Oauth(oauth) => {
            if let Some(fresh) = cache.get_fresh() {
                return Ok(fresh);
            }
            let bearer = exchange(http, token_url, oauth).await?;
            cache.put(bearer.clone());
            Ok(bearer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exchange_reply_parses_token_expiry_and_endpoint() {
        let v = json!({
            "token": "tid=abc;exp=1234",
            "expires_at": 1_900_000_000i64,
            "endpoints": { "api": "https://api.enterprise.githubcopilot.com" }
        });
        let b = parse_exchange_reply(&v).unwrap();
        assert_eq!(b.token, "tid=abc;exp=1234");
        assert_eq!(b.expires_at, 1_900_000_000);
        assert_eq!(
            b.api_url.as_deref(),
            Some("https://api.enterprise.githubcopilot.com/chat/completions")
        );
    }

    #[test]
    fn exchange_reply_without_endpoint_or_expiry_still_parses() {
        let b = parse_exchange_reply(&json!({ "token": "t" })).unwrap();
        assert_eq!(b.expires_at, 0);
        assert!(b.api_url.is_none());
        assert!(parse_exchange_reply(&json!({})).is_none());
    }

    #[test]
    fn cache_serves_fresh_and_rejects_near_expiry() {
        let cache = BearerCache::new();
        assert!(cache.get_fresh().is_none());
        cache.put(CopilotBearer {
            token: "fresh".into(),
            expires_at: crate::now_ms() / 1000 + 3600,
            api_url: None,
        });
        assert_eq!(cache.get_fresh().unwrap().token, "fresh");
        cache.put(CopilotBearer {
            token: "stale".into(),
            expires_at: crate::now_ms() / 1000 + 30, // inside the margin
            api_url: None,
        });
        assert!(cache.get_fresh().is_none());
        cache.invalidate();
        assert!(cache.get_fresh().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ready_bearer_skips_the_exchange() {
        let cache = BearerCache::new();
        // an unroutable token_url proves no network call happens
        let b = fresh_bearer(
            &reqwest::Client::new(),
            &cache,
            &GithubCredential::Bearer("direct".into()),
            "http://127.0.0.1:1/nope",
        )
        .await
        .unwrap();
        assert_eq!(b.token, "direct");
        assert_eq!(b.expires_at, 0, "unknown expiry never refreshes");
    }
}
