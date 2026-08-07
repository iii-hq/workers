//! Live model discovery: `GET /models` on the Copilot API endpoint is the
//! source of truth for the catalog slice — ids AND metadata, since the
//! listing is capability-structured (see [`crate::catalog`]). Admission
//! keeps the slice harness-usable: chat models with tool support that the
//! subscription actually enables. Everything admitted is reconciled through
//! the router's single write path.
use crate::auth;
use crate::catalog::model_from_row;
use crate::errors::upstream_unavailable;
use crate::exchange::{fresh_bearer, BearerCache, ExchangeError, DEFAULT_TOKEN_URL};
use crate::request::client_headers;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// Derive the models endpoint from the chat completions endpoint
/// (`…/chat/completions` → `…/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => "https://api.githubcopilot.com/models".to_string(),
    }
}

/// Harness-usable models only: chat type, tool support, and enabled for this
/// subscription. `model_picker_enabled: false` rows are models the plan
/// knows about but has not enabled — calling one fails with
/// "model not supported", so they would be dead picker rows.
pub fn admit(row: &Value) -> bool {
    let chat = row
        .pointer("/capabilities/type")
        .and_then(Value::as_str)
        .map(|t| t == "chat")
        .unwrap_or(false);
    let tools = row
        .pointer("/capabilities/supports/tool_calls")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let enabled = row
        .get("model_picker_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    chat && tools && enabled
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|row| admit(row))
                .filter_map(model_from_row)
                .collect()
        })
        .unwrap_or_default()
}

enum FetchOutcome {
    Ok(Vec<Model>),
    AuthFailed,
    Transient(String),
}

async fn fetch_live_models(http: &reqwest::Client, url: &str, bearer: &str) -> FetchOutcome {
    let mut req = http
        .get(url)
        .header("authorization", format!("Bearer {bearer}"));
    for (name, value) in client_headers() {
        req = req.header(name, value);
    }
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
pub async fn refresh_models(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &BearerCache,
) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let Some(credential) = auth::resolve_credential(iii).await else {
        // Never signed in anywhere this worker can see: prune the slice so
        // the picker reflects reality instead of stale rows.
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let bearer = match fresh_bearer(http, cache, &credential, DEFAULT_TOKEN_URL).await {
        Ok(b) => b,
        Err(ExchangeError::Unauthorized(_)) => {
            // Revoked login / no Copilot access: the models are unusable.
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            return Ok(0);
        }
        Err(ExchangeError::Transient(msg)) => return Err(upstream_unavailable(msg)),
    };

    // Operator api_url override wins over the exchange endpoint (same
    // precedence the stream path uses).
    let api_url = resolved
        .api_url
        .clone()
        .or_else(|| bearer.api_url.clone())
        .unwrap_or_else(|| crate::exchange::DEFAULT_API_URL.to_string());
    match fetch_live_models(http, &models_url(&api_url), &bearer.token).await {
        FetchOutcome::Ok(models) => {
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            Ok(0)
        }
        // Blip: keep the previous slice (spec § reconcile-to-empty guidance).
        FetchOutcome::Transient(msg) => Err(upstream_unavailable(msg)),
    }
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
    cache: BearerCache,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let (iii, http, cache) = (iii.clone(), http.clone(), cache.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http, &cache).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: &str, model_type: &str, tools: bool, enabled: bool) -> Value {
        json!({
            "id": id,
            "capabilities": {
                "type": model_type,
                "supports": { "tool_calls": tools }
            },
            "model_picker_enabled": enabled
        })
    }

    #[test]
    fn models_url_derives_from_completions_endpoint() {
        assert_eq!(
            models_url("https://api.githubcopilot.com/chat/completions"),
            "https://api.githubcopilot.com/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.githubcopilot.com/models"
        );
    }

    #[test]
    fn admission_requires_chat_tools_and_plan_enablement() {
        assert!(admit(&row("gpt-5.2", "chat", true, true)));
        // embeddings and other non-chat types are dead in the agent loop
        assert!(!admit(&row(
            "text-embedding-3-small",
            "embeddings",
            false,
            true
        )));
        // no tool support → dead in the agent loop
        assert!(!admit(&row("chatty", "chat", false, true)));
        // known to the plan but not enabled → "model not supported" on call
        assert!(!admit(&row("locked", "chat", true, false)));
        // missing capabilities tree → not admitted
        assert!(!admit(&json!({ "id": "bare" })));
    }

    #[test]
    fn parses_rows_applying_admission_and_prefix() {
        let listing = json!({ "data": [
            row("gpt-5.2", "chat", true, true),
            row("claude-sonnet-4.6", "chat", true, true),
            row("text-embedding-3-small", "embeddings", false, true),
            { "id": "", "capabilities": { "type": "chat",
              "supports": { "tool_calls": true } } },
        ]});
        let ids: Vec<String> = parse_live_models(&listing)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["copilot/gpt-5.2", "copilot/claude-sonnet-4.6"]);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&json!({})).is_empty());
        assert!(parse_live_models(&json!({ "data": "nope" })).is_empty());
    }
}
