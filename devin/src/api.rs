//! Devin REST client. One pooled `reqwest::Client` is built at boot and shared;
//! every call reads the live credentials, base URL, and timeout out of the
//! current config snapshot so a hot-reloaded key or org takes effect on the
//! next request. `request` is the generic transport that backs both the typed
//! wrappers and the `devin::api` passthrough.
//!
//! Devin has two API shapes. Personal tokens use the flat v1 API; service keys
//! use the v3 API scoped under `organizations/{org_id}`. The session wrappers
//! pick the shape from whether `org_id` is set; pr-review is v3-only. The
//! passthrough takes a full relative path, so a caller can address any scope
//! (including code scan and other v3 surfaces) directly.

use std::time::Duration;

use anyhow::{anyhow, Result};
use reqwest::{Client, Method};
use serde_json::{json, Value};

use crate::config::Config;

/// Build the shared client. No global timeout is set here — each request
/// applies the config's `request_timeout_secs` so a hot reload is honoured
/// without rebuilding the connection pool.
pub fn build_client() -> Client {
    Client::builder()
        .user_agent(concat!("iii-devin-worker/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// Perform one Devin API call. `path` is relative to the configured base URL
/// (leading slash optional). `query` and `body` are optional JSON objects.
/// A non-2xx response is returned as an error carrying the parsed body.
pub async fn request(
    http: &Client,
    cfg: &Config,
    method: &str,
    path: &str,
    query: Option<&Value>,
    body: Option<&Value>,
) -> Result<Value> {
    if cfg.api_key.is_empty() {
        return Err(anyhow!(
            "devin api_key is not configured; set DEVIN_API_KEY in the devin configuration"
        ));
    }
    let m = Method::from_bytes(method.trim().to_uppercase().as_bytes())
        .map_err(|_| anyhow!("invalid HTTP method: {method}"))?;
    let base = cfg.base_url.trim_end_matches('/');
    let rel = path.trim_start_matches('/');
    let url = format!("{base}/{rel}");

    let mut rb = http
        .request(m, &url)
        .bearer_auth(&cfg.api_key)
        .timeout(Duration::from_secs(cfg.request_timeout_secs.max(1)));
    if let Some(obj) = query.and_then(Value::as_object) {
        let pairs: Vec<(String, String)> = obj
            .iter()
            .map(|(k, v)| {
                let s = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                (k.clone(), s)
            })
            .collect();
        rb = rb.query(&pairs);
    }
    if let Some(b) = body {
        rb = rb.json(b);
    }

    let resp = rb
        .send()
        .await
        .map_err(|e| anyhow!("devin api request to {url} failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let parsed: Value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
    };
    if !status.is_success() {
        return Err(anyhow!(
            "devin api {} /{} -> {}: {}",
            method.to_uppercase(),
            rel,
            status.as_u16(),
            parsed
        ));
    }
    Ok(parsed)
}

/// Build an organization-scoped path, requiring `org_id`.
fn org_scoped(cfg: &Config, suffix: &str) -> Result<String> {
    if cfg.org_id.is_empty() {
        return Err(anyhow!(
            "org_id is not configured; it is a required path segment for organization-scoped v3 endpoints"
        ));
    }
    Ok(format!("organizations/{}/{}", cfg.org_id, suffix))
}

// Sessions. Devin exposes two API shapes. Personal tokens use the flat v1 API
// (`sessions`, `session/{id}`, `session/{id}/message`); service keys use the
// v3 API scoped under `organizations/{org_id}`. The path shape is driven by
// whether `org_id` is set, so `base_url` should be set to match (v1 vs v3).

fn sessions_collection(cfg: &Config) -> String {
    if cfg.org_id.is_empty() {
        "sessions".to_string()
    } else {
        format!("organizations/{}/sessions", cfg.org_id)
    }
}

fn session_item(cfg: &Config, id: &str) -> String {
    if cfg.org_id.is_empty() {
        format!("session/{id}")
    } else {
        format!("organizations/{}/sessions/{}", cfg.org_id, id)
    }
}

fn session_message_path(cfg: &Config, id: &str) -> String {
    if cfg.org_id.is_empty() {
        format!("session/{id}/message")
    } else {
        format!("organizations/{}/sessions/{}/messages", cfg.org_id, id)
    }
}

pub async fn create_session(http: &Client, cfg: &Config, body: &Value) -> Result<Value> {
    request(
        http,
        cfg,
        "POST",
        &sessions_collection(cfg),
        None,
        Some(body),
    )
    .await
}

pub async fn get_session(http: &Client, cfg: &Config, session_id: &str) -> Result<Value> {
    request(http, cfg, "GET", &session_item(cfg, session_id), None, None).await
}

pub async fn send_message(
    http: &Client,
    cfg: &Config,
    session_id: &str,
    body: &Value,
) -> Result<Value> {
    request(
        http,
        cfg,
        "POST",
        &session_message_path(cfg, session_id),
        None,
        Some(body),
    )
    .await
}

// PR reviews — POST/GET /v3/organizations/{org_id}/pr-reviews.

pub async fn pr_review_trigger(http: &Client, cfg: &Config, body: &Value) -> Result<Value> {
    let path = org_scoped(cfg, "pr-reviews")?;
    request(http, cfg, "POST", &path, None, Some(body)).await
}

pub async fn pr_review_status(http: &Client, cfg: &Config, query: &Value) -> Result<Value> {
    let path = org_scoped(cfg, "pr-reviews")?;
    request(http, cfg, "GET", &path, Some(query), None).await
}
