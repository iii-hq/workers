//! Slack Web API client. Every method is `POST https://slack.com/api/<method>`
//! with a Bearer token and a JSON body, returning `{ ok, ... }`. We surface the
//! whole payload on success and turn `ok:false` into a handler error.

use std::time::Duration;

use iii_sdk::errors::Error;
use serde_json::Value;

use crate::deps::Deps;

const API_BASE: &str = "https://slack.com/api";

/// Call a Slack method with the configured **bot** token.
pub async fn call(deps: &Deps, method: &str, params: Value) -> Result<Value, Error> {
    let cfg = deps.cfg().await;
    if cfg.bot_token.trim().is_empty() {
        return Err(Error::Handler("slack: bot_token is not configured".into()));
    }
    call_with_token(deps, method, params, &cfg.bot_token, cfg.timeout_ms).await
}

/// Call a Slack method with the configured **user** token (`xoxp-`). Used by
/// methods that bot tokens cannot perform (e.g. `search.messages`).
pub async fn call_user(deps: &Deps, method: &str, params: Value) -> Result<Value, Error> {
    let cfg = deps.cfg().await;
    let Some(token) = cfg.user_token.as_deref().filter(|t| !t.trim().is_empty()) else {
        return Err(Error::Handler(format!(
            "slack {method}: requires a user_token (xoxp-) in configuration"
        )));
    };
    call_with_token(deps, method, params, token, cfg.timeout_ms).await
}

pub async fn call_with_token(
    deps: &Deps,
    method: &str,
    params: Value,
    token: &str,
    timeout_ms: u64,
) -> Result<Value, Error> {
    let url = format!("{API_BASE}/{method}");
    let resp = deps
        .http
        .post(&url)
        .bearer_auth(token)
        .timeout(Duration::from_millis(timeout_ms.max(1000)))
        .json(&params)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("slack http {method}: {}", e.without_url())))?;
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| Error::Handler(format!("slack json {method}: {e}")))?;
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        return Err(Error::Handler(format!("slack {method} failed: {err}")));
    }
    Ok(payload)
}

/// Run `auth.test` and return the parsed identity.
pub async fn auth_test(deps: &Deps) -> Result<crate::deps::Identity, Error> {
    let v = call(deps, "auth.test", serde_json::json!({})).await?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(crate::deps::Identity {
        team: s("team"),
        team_id: s("team_id"),
        user_id: s("user_id"),
        bot_id: s("bot_id"),
        url: s("url"),
        enterprise_id: s("enterprise_id"),
    })
}
