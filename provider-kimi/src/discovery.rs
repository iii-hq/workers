//! Live model discovery: `GET /v1/models` is the source of truth for the
//! catalog's id list — filtered to Kimi/Moonshot chat families, enriched with
//! the local metadata table (Moonshot's API carries no capability data), and
//! reconciled through the router's single write path.
use crate::config::DEFAULT_API_URL;
use crate::curated::{base_id, enrich};
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;
use std::collections::HashSet;

/// Derive the models endpoint from the configured completions endpoint
/// (`…/v1/chat/completions` → `…/v1/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => "https://api.moonshot.ai/v1/models".to_string(),
    }
}

/// Kimi/Moonshot chat families we route to. Admits `kimi-*` and `moonshot-*`
/// (chat + vision variants); excludes embedding/other non-chat ids defensively.
pub fn is_chat_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    let chat_family = lower.starts_with("kimi") || lower.starts_with("moonshot");
    if !chat_family {
        return false;
    }
    const NON_CHAT: [&str; 3] = ["embedding", "embed", "rerank"];
    !NON_CHAT.iter().any(|term| lower.contains(term))
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    let ids: Vec<String> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    let id = raw
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())?;
                    is_chat_model(id).then(|| id.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    // base_id is the identity for Moonshot ids (no trailing ISO date alias to
    // collapse), so this dedup keeps every distinct id — kept symmetric with
    // the OpenAI provider's shape in case a future alias scheme appears.
    let live: HashSet<&str> = ids.iter().map(String::as_str).collect();
    ids.iter()
        .filter(|id| {
            let base = base_id(id);
            base == id.as_str() || !live.contains(base)
        })
        .map(|id| enrich(id))
        .collect()
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
    let credential_value = crate::config::credential_parts(&credential);

    let url = models_url(resolved.api_url.as_deref().unwrap_or(DEFAULT_API_URL));
    match fetch_live_models(http, &url, credential_value).await {
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
    fn models_url_derives_from_completions_endpoint() {
        assert_eq!(
            models_url("https://api.moonshot.ai/v1/chat/completions"),
            "https://api.moonshot.ai/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.moonshot.ai/v1/models"
        );
    }

    #[test]
    fn chat_family_filter_admits_kimi_and_moonshot_only() {
        assert!(is_chat_model("kimi-k2-0905-preview"));
        assert!(is_chat_model("kimi-latest"));
        assert!(is_chat_model("kimi-thinking-preview"));
        assert!(is_chat_model("moonshot-v1-128k"));
        assert!(is_chat_model("moonshot-v1-8k-vision-preview"));
        assert!(!is_chat_model("gpt-5.2"));
        assert!(!is_chat_model("text-embedding-3-large"));
        assert!(!is_chat_model("moonshot-embedding-v1"));
    }

    #[test]
    fn parses_ids_skipping_malformed_and_non_chat_rows() {
        let json = serde_json::json!({
            "data": [
                { "id": "kimi-k2-0905-preview", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "text-embedding-3-large", "object": "model" },
                { "id": "moonshot-v1-128k", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["kimi-k2-0905-preview", "moonshot-v1-128k"]);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
