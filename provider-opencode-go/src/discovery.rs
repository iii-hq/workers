//! Live model discovery: `GET /v1/models` is the source of truth for the
//! catalog's id list — all models returned are valid chat models, no family
//! filtering, no legacy dedup.
//!
//! Model metadata (context window, reasoning, tool support) comes from the
//! hardcoded curated table in [`crate::curated`] — prepared from models.dev
//! (fetched 2026-08-03), never fetched at runtime. Ids the table does not
//! know keep conservative defaults, so the model list always works.
use crate::config::DEFAULT_API_URL;
use crate::curated::enrich;
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// Derive the models endpoint from the generation endpoint.
pub fn models_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');
    trimmed
        .strip_suffix("/chat/completions")
        .map(|base| format!("{base}/models"))
        .unwrap_or_else(|| "https://opencode.ai/zen/go/v1/models".to_string())
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    let ids: Vec<String> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    raw.get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();

    ids.iter().map(|id| enrich(id)).collect()
}

enum FetchOutcome {
    Ok(Vec<Model>),
    AuthFailed,
    Transient(String),
}

async fn fetch_live_models(
    http: &reqwest::Client,
    url: &str,
    credential_value: &str,
) -> FetchOutcome {
    let req = http
        .get(url)
        .header("authorization", format!("Bearer {credential_value}"));
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
/// Fetches the live id list and enriches it from the curated metadata table.
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let Some(credential) = resolved.credential else {
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let credential_value = crate::config::credential_parts(&credential);

    let url = models_url(resolved.api_url.as_deref().unwrap_or(DEFAULT_API_URL));
    match fetch_live_models(http, &url, credential_value).await {
        FetchOutcome::Ok(models) => {
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            Ok(0)
        }
        FetchOutcome::Transient(msg) => Err(upstream_unavailable(msg)),
    }
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_url_derives_from_generation_endpoint() {
        assert_eq!(
            models_url("https://opencode.ai/zen/go/v1/chat/completions"),
            "https://opencode.ai/zen/go/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://opencode.ai/zen/go/v1/models"
        );
    }

    #[test]
    fn parses_all_ids_with_curated_enrichment() {
        let json = serde_json::json!({
            "data": [
                { "id": "deepseek-v4-flash", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "kimi-k2.7-code", "object": "model" },
                { "id": "qwen2.5-coder-7b-instruct", "object": "model" },
            ]
        });
        let models = parse_live_models(&json);
        assert_eq!(models.len(), 3);
        // Curated id → curated metadata (1M context, thinking on).
        assert_eq!(models[0].id, "deepseek-v4-flash");
        assert_eq!(models[0].context_window, 1_000_000);
        assert_eq!(models[0].supports_thinking, Some(true));
        // Curated id → curated metadata.
        assert_eq!(models[1].id, "kimi-k2.7-code");
        // Unknown id → conservative defaults, never vanishes.
        assert_eq!(models[2].id, "qwen2.5-coder-7b-instruct");
        assert_eq!(models[2].context_window, 128_000);
        assert_eq!(models[2].supports_thinking, None);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }

    #[test]
    fn unknown_ids_keep_conservative_defaults() {
        let json = serde_json::json!({
            "data": [
                { "id": "opencode-go-test-model", "object": "model" },
                { "id": "not-a-real-model", "object": "model" },
            ]
        });
        for m in parse_live_models(&json) {
            assert_eq!(m.display_name.as_deref(), Some(m.id.as_str()));
            assert_eq!(m.context_window, 128_000);
            assert_eq!(m.max_output_tokens, 4096);
            assert_eq!(m.supports_thinking, None);
            assert!(m.reasoning_efforts.is_none());
            assert_eq!(m.supports_tools, Some(true));
            assert_eq!(m.supports_structured_output, None);
        }
    }
}
