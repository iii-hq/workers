//! Live model discovery: GET /v1/models, filter to chat/reasoning families,
//! merge curated capability metadata, reconcile the catalog slice through
//! the router's single write path.
use crate::config::DEFAULT_API_URL;
use crate::curated::{merge_with_live, LiveStub};
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

/// Derive the models endpoint from the configured completions endpoint
/// (`…/v1/chat/completions` → `…/v1/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => "https://api.openai.com/v1/models".to_string(),
    }
}

/// Chat/reasoning families we route to (port of discover.ts). Excludes
/// embeddings, audio, image, moderation, realtime, and legacy
/// completion-only ids.
pub fn is_chat_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    let chat_family = lower.starts_with("gpt-")
        || lower.starts_with("chatgpt")
        || (lower.len() >= 2 && lower.starts_with('o') && lower.as_bytes()[1].is_ascii_digit());
    if !chat_family {
        return false;
    }
    const NON_CHAT: [&str; 14] = [
        "embedding",
        "whisper",
        "tts",
        "audio",
        "dall-e",
        "image",
        "moderation",
        "realtime",
        "transcribe",
        "search",
        "babbage",
        "davinci",
        "ada",
        "curie",
    ];
    !NON_CHAT.iter().any(|term| lower.contains(term))
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
                    is_chat_model(id).then(|| LiveStub { id: id.to_string() })
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
pub async fn refresh_models(iii: &III, http: &reqwest::Client) -> Result<usize, IIIError> {
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
    fn models_url_derives_from_completions_endpoint() {
        assert_eq!(
            models_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn chat_family_filter_admits_gpt_o_series_and_chatgpt_only() {
        assert!(is_chat_model("gpt-5.2"));
        assert!(is_chat_model("gpt-5-mini"));
        assert!(is_chat_model("o3-mini"));
        assert!(is_chat_model("o4-mini"));
        assert!(is_chat_model("chatgpt-4o-latest"));
        assert!(!is_chat_model("text-embedding-3-large"));
        assert!(!is_chat_model("gpt-4o-audio-preview"));
        assert!(!is_chat_model("whisper-1"));
        assert!(!is_chat_model("dall-e-3"));
        assert!(!is_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_chat_model("gpt-image-1"));
        assert!(!is_chat_model("omni-moderation-latest"));
        assert!(!is_chat_model("davinci-002"));
    }

    #[test]
    fn parses_ids_skipping_malformed_and_non_chat_rows() {
        let json = serde_json::json!({
            "data": [
                { "id": "gpt-5.1-2025-11-13", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "text-embedding-3-large", "object": "model" },
                { "id": "o3-mini", "object": "model" },
            ]
        });
        let stubs = parse_live_models(&json);
        assert_eq!(stubs.len(), 2);
        assert_eq!(stubs[0].id, "gpt-5.1-2025-11-13");
        assert_eq!(stubs[1].id, "o3-mini");
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
