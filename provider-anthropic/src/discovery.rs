//! Live model discovery: GET /v1/models, merge curated capability metadata,
//! reconcile the catalog slice through the router's single write path.
use crate::config::{credential_parts, AuthMode, DEFAULT_API_URL};
use crate::curated::{merge_with_live, LiveStub};
use crate::errors::upstream_unavailable;
use crate::request::{auth_header, ANTHROPIC_VERSION};
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

/// Derive the models endpoint from the configured messages endpoint
/// (`…/v1/messages` → `…/v1/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/messages") {
        Some(base) => format!("{base}/models"),
        None => "https://api.anthropic.com/v1/models".to_string(),
    }
}

pub fn parse_live_models(json: &Value) -> Vec<LiveStub> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    let id = raw
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())?;
                    Some(LiveStub {
                        id: id.to_string(),
                        display_name: raw
                            .get("display_name")
                            .and_then(Value::as_str)
                            .map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

enum FetchOutcome {
    Ok(Vec<LiveStub>),
    AuthFailed,
    Transient(String),
}

async fn fetch_live_models(
    http: &reqwest::Client,
    url: &str,
    credential_value: &str,
    auth_mode: AuthMode,
) -> FetchOutcome {
    let (auth_name, auth_value) = auth_header(auth_mode, credential_value);
    let req = http
        .get(url)
        .header(auth_name, auth_value)
        .header("anthropic-version", ANTHROPIC_VERSION);
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return FetchOutcome::Transient(format!("models fetch failed: {e}")),
    };
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return FetchOutcome::AuthFailed;
    }
    if !(200..300).contains(&status) {
        return FetchOutcome::Transient(format!("models fetch http {status}"));
    }
    match resp.json::<Value>().await {
        Ok(v) => FetchOutcome::Ok(parse_live_models(&v)),
        Err(e) => FetchOutcome::Transient(format!("models response not json: {e}")),
    }
}

/// The refresh flow; returns the reconciled slice size.
pub async fn refresh_models(iii: &III, http: &reqwest::Client) -> Result<usize, IIIError> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let Some(credential) = resolved.credential else {
        // Key removed: prune the slice so the picker reflects removal
        // instead of showing stale, unusable rows.
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let (credential_value, auth_mode) = credential_parts(&credential);

    let url = models_url(resolved.api_url.as_deref().unwrap_or(DEFAULT_API_URL));
    match fetch_live_models(http, &url, credential_value, auth_mode).await {
        FetchOutcome::Ok(stubs) => {
            let models = merge_with_live(&stubs);
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            // Revoked/invalid key: the models are genuinely unusable.
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            Ok(0)
        }
        // Blip: keep the previous slice (spec § reconcile-to-empty guidance).
        FetchOutcome::Transient(msg) => Err(upstream_unavailable(msg)),
    }
}

pub fn make_refresh_models(
    iii: III,
    http: reqwest::Client,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |_raw: Value| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http).await?;
            Ok(json!({ "ok": true, "count": count }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_url_derives_from_messages_endpoint() {
        assert_eq!(
            models_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/messages"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn parses_ids_and_display_names_skipping_malformed_rows() {
        let json = serde_json::json!({
            "data": [
                { "id": "claude-sonnet-4-6-20260115", "display_name": "Claude Sonnet 4.6" },
                { "id": "" },
                { "display_name": "no id" },
                { "id": "claude-haiku-4-5" },
            ]
        });
        let stubs = parse_live_models(&json);
        assert_eq!(stubs.len(), 2);
        assert_eq!(stubs[0].id, "claude-sonnet-4-6-20260115");
        assert_eq!(stubs[0].display_name.as_deref(), Some("Claude Sonnet 4.6"));
        assert_eq!(stubs[1].display_name, None);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
