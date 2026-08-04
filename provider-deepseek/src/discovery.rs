//! Catalog reconcile. DeepSeek's `GET /models` is the source of truth for the
//! id list; it carries no metadata, so each id is enriched from the local
//! table (curated.rs) and pushed through the router's single write path.
//! The configured credential gates the slice — no key → empty catalog, so the
//! picker never shows unusable rows.
use crate::config::{credential_parts, DEFAULT_API_URL};
use crate::curated::enrich;
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// Derive the models endpoint from the generation endpoint. DeepSeek's
/// OpenAI surface is rooted at the bare host (`/chat/completions`,
/// `/models`), so the same sibling rule serves custom `/v1/…` gateways.
pub fn models_url(api_url: &str) -> String {
    api_url
        .trim_end_matches('/')
        .strip_suffix("/chat/completions")
        .map(|base| format!("{base}/models"))
        .unwrap_or_else(|| "https://api.deepseek.com/models".to_string())
}

/// `{ "data": [ { "id": … } ] }` → enriched catalog rows. Ids are taken
/// verbatim: DeepSeek serves chat models only, so there is no modality to
/// filter out.
pub fn parse_live_models(json: &Value) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    raw.get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                })
                .map(enrich)
                .collect()
        })
        .unwrap_or_default()
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
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let Some(credential) = resolved.credential else {
        // Key removed: prune the slice so the picker reflects removal
        // instead of showing stale, unusable rows.
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };

    let api_url = resolved
        .api_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_API_URL);
    match fetch_live_models(http, &models_url(api_url), credential_parts(&credential)).await {
        FetchOutcome::Ok(models) => {
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
    fn models_url_is_the_sibling_of_the_generation_endpoint() {
        assert_eq!(
            models_url(DEFAULT_API_URL),
            "https://api.deepseek.com/models"
        );
        assert_eq!(
            models_url("https://api.deepseek.com/chat/completions/"),
            "https://api.deepseek.com/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.deepseek.com/models"
        );
    }

    #[test]
    fn live_ids_are_enriched_and_malformed_rows_skipped() {
        let json = serde_json::json!({
            "object": "list",
            "data": [
                { "id": "deepseek-v4-flash", "object": "model", "owned_by": "deepseek" },
                { "id": "deepseek-v4-pro", "object": "model", "owned_by": "deepseek" },
                { "id": "", "object": "model" },
                { "object": "model" },
            ]
        });
        let models = parse_live_models(&json);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["deepseek-v4-flash", "deepseek-v4-pro"]);
        assert_eq!(models[1].context_window, 1_000_000);
        assert_eq!(models[1].display_name.as_deref(), Some("DeepSeek V4 Pro"));
    }

    #[test]
    fn unknown_ids_survive_discovery_with_defaults() {
        // A model DeepSeek ships before this table is updated must still be
        // routable — the row degrades, it never disappears.
        let json = serde_json::json!({ "data": [{ "id": "deepseek-v5-pro" }] });
        let models = parse_live_models(&json);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "deepseek-v5-pro");
        assert_eq!(models[0].context_window, 65_536);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
