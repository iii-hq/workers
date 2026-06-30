//! Live model discovery: `GET /v1/models` is the source of truth for the
//! catalog's id list — filtered to current-generation chat/reasoning
//! families, deduplicated against dated snapshots, enriched with the local
//! metadata table (xAI's API carries no capability data), and reconciled
//! through the router's single write path.
use crate::config::DEFAULT_API_URL;
use crate::curated::{base_id, enrich, is_legacy_generation};
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
        None => "https://api.x.ai/v1/models".to_string(),
    }
}

/// Substrings marking a non-chat modality. Applied to xAI's catalog AND
/// custom xAI-compatible endpoints, since a local server may also expose
/// image/embedding models on /v1/models. `grok-imagine*` and `grok-2-image*`
/// are xAI's image/video families.
const NON_CHAT: [&str; 5] = ["imagine", "image", "video", "embed", "audio"];

/// True when the (lowercased) id is not an obvious non-chat modality.
fn is_chat_modality(lower: &str) -> bool {
    !NON_CHAT.iter().any(|term| lower.contains(term))
}

pub fn is_chat_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    // xAI's text/chat family is `grok-*`; the non-chat gate drops the
    // image/video members (grok-imagine, grok-2-image).
    lower.starts_with("grok-") && is_chat_modality(&lower)
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    let raw_ids: Vec<String> = json
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

    let filtered: Vec<String> = raw_ids
        .iter()
        .filter(|id| is_chat_model(id) && !is_legacy_generation(id))
        .cloned()
        .collect();

    // Custom xAI-compatible servers (LMStudio, Ollama, vLLM) serve
    // arbitrarily-named models that the grok family gate and the
    // legacy denylist would discard entirely. When xAI-style filtering
    // admits nothing but the server did return models, it is plainly not
    // xAI: list everything it serves, only dropping non-chat modalities.
    // Real xAI always carries a current family, so this never triggers
    // against api.x.ai.
    let ids = if filtered.is_empty() && !raw_ids.is_empty() {
        raw_ids
            .into_iter()
            .filter(|id| is_chat_modality(&id.to_ascii_lowercase()))
            .collect()
    } else {
        filtered
    };

    // Dated snapshots are pinning artifacts: when the undated alias is also
    // live (grok-4 next to grok-4-0709), keep only the alias so the
    // picker carries one row per model.
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
            models_url("https://api.x.ai/v1/chat/completions"),
            "https://api.x.ai/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.x.ai/v1/models"
        );
    }

    #[test]
    fn chat_family_filter_admits_grok_only() {
        assert!(is_chat_model("grok-4.3"));
        assert!(is_chat_model("grok-4"));
        assert!(is_chat_model("grok-3-mini"));
        assert!(is_chat_model("grok-code-fast-1"));
        assert!(is_chat_model("grok-build-0.1"));
        assert!(!is_chat_model("text-embedding-3-large"));
        assert!(!is_chat_model("grok-imagine-image"));
        assert!(!is_chat_model("grok-imagine-video-1.5"));
        assert!(!is_chat_model("grok-2-image-1212"));
        assert!(!is_chat_model("gpt-5.2"));
    }

    #[test]
    fn parses_ids_skipping_malformed_non_chat_and_legacy_rows() {
        let json = serde_json::json!({
            "data": [
                { "id": "grok-4.3", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "grok-imagine-image", "object": "model" },
                { "id": "grok-2-1212", "object": "model" },
                { "id": "grok-2-vision-1212", "object": "model" },
            ]
        });
        let models = parse_live_models(&json);
        // grok-2 generations are legacy; imagine is non-chat; only grok-4.3 stays.
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "grok-4.3");
        assert_eq!(models[0].display_name.as_deref(), Some("Grok 4.3"));
    }

    #[test]
    fn dated_snapshot_drops_when_undated_alias_is_live() {
        let json = serde_json::json!({
            "data": [
                { "id": "grok-4", "object": "model" },
                { "id": "grok-4-0709", "object": "model" },
                { "id": "grok-3-mini-0625", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["grok-4", "grok-3-mini-0625"]);
    }

    #[test]
    fn custom_compatible_endpoint_lists_all_when_xai_filter_admits_nothing() {
        // An LMStudio/Ollama-style catalog: no id passes the grok family gate,
        // so xAI filtering would yield an empty list. The fallback lists
        // everything the server serves, dropping only the embedding model.
        let json = serde_json::json!({
            "data": [
                { "id": "qwen2.5-coder-7b-instruct", "object": "model" },
                { "id": "llama-3.2-3b-instruct", "object": "model" },
                { "id": "nomic-embed-text-v1.5", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["qwen2.5-coder-7b-instruct", "llama-3.2-3b-instruct"]);
        // Unknown families get conservative defaults, never vanish.
        assert_eq!(parse_live_models(&json)[0].context_window, 131_072);
    }

    #[test]
    fn xai_filter_still_applies_when_some_current_model_is_present() {
        // Mixed catalog with a current grok model present → normal xAI
        // filtering (legacy + non-chat dropped), NOT the fallback.
        let json = serde_json::json!({
            "data": [
                { "id": "grok-4.3", "object": "model" },
                { "id": "grok-2-1212", "object": "model" },
                { "id": "text-embedding-3-large", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["grok-4.3"]);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
