//! Live model discovery: `GET /api/v1/models` is the source of truth for the
//! whole catalog slice — ids AND metadata, since OpenRouter's listing is
//! self-describing (see [`crate::catalog`]). Admission keeps the slice
//! harness-usable instead of mirroring all of OpenRouter: a model must accept
//! function tools (the agent loop is unusable without them) and emit text
//! (drops image/audio/video generators). Everything admitted is reconciled
//! through the router's single write path.
use crate::catalog::model_from_row;
use crate::config::DEFAULT_API_URL;
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// Derive the models endpoint from the configured completions endpoint
/// (`…/api/v1/chat/completions` → `…/api/v1/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => "https://openrouter.ai/api/v1/models".to_string(),
    }
}

fn str_array(v: Option<&Value>) -> impl Iterator<Item = &str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// Harness-usable models only: function tools + text output. OpenRouter lists
/// hundreds of models; the ones that cannot drive an agent loop (no `tools`
/// support) or produce no text (image/audio generators, embedders) would be
/// dead rows in the picker.
pub fn admit(row: &Value) -> bool {
    let has_tools = str_array(row.get("supported_parameters")).any(|p| p == "tools");
    let text_out = str_array(row.pointer("/architecture/output_modalities")).any(|m| m == "text");
    has_tools && text_out
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
    use serde_json::json;

    fn row(id: &str, params: &[&str], out_modalities: &[&str]) -> Value {
        json!({
            "id": id,
            "supported_parameters": params,
            "architecture": { "output_modalities": out_modalities }
        })
    }

    #[test]
    fn models_url_derives_from_completions_endpoint() {
        assert_eq!(
            models_url("https://openrouter.ai/api/v1/chat/completions"),
            "https://openrouter.ai/api/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://openrouter.ai/api/v1/models"
        );
    }

    #[test]
    fn admission_requires_tools_and_text_output() {
        assert!(admit(&row(
            "anthropic/claude-x",
            &["tools", "reasoning"],
            &["text"]
        )));
        // no tools support → dead in the agent loop
        assert!(!admit(&row(
            "vendor/story-teller",
            &["temperature"],
            &["text"]
        )));
        // image generator → no text out
        assert!(!admit(&row("vendor/painter", &["tools"], &["image"])));
        // missing metadata → not admitted
        assert!(!admit(&json!({ "id": "vendor/bare" })));
    }

    #[test]
    fn parses_rows_applying_admission_and_prefix() {
        let listing = json!({ "data": [
            row("anthropic/claude-x", &["tools"], &["text"]),
            row("openai/gpt-x", &["tools", "structured_outputs"], &["text"]),
            row("vendor/painter", &["tools"], &["image"]),
            { "id": "", "supported_parameters": ["tools"],
              "architecture": { "output_modalities": ["text"] } },
        ]});
        let ids: Vec<String> = parse_live_models(&listing)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            ids,
            ["openrouter/anthropic/claude-x", "openrouter/openai/gpt-x"]
        );
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&json!({})).is_empty());
        assert!(parse_live_models(&json!({ "data": "nope" })).is_empty());
    }
}
