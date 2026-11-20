//! Catalog reconcile. Gemini's `GET /models` is the source of truth for the
//! id list; each id is enriched (curated.rs) and pushed through the router's
//! single write path. The configured credential gates the slice — no key →
//! empty catalog, so the picker never shows unusable rows.
use crate::config::{DEFAULT_API_URL, SYNC_OUTPUT_CAP, auth_header};
use crate::curated::enrich;
use crate::errors::{classify, upstream_unavailable};
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use llm_router::types::events::ErrorKind;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// Derive the models endpoint from the generation endpoint, so a proxy
/// behind an `api_url` override serves both routes. `None` when `api_url`
/// does not end in `/interactions`: guessing a host would send the
/// operator's credential somewhere they did not configure.
pub fn models_url(api_url: &str) -> Option<String> {
    api_url
        .trim_end_matches('/')
        .strip_suffix("/interactions")
        .map(|base| format!("{base}/models"))
}

/// Id fragments that mark a non-chat model.
const NON_CHAT: [&str; 11] = [
    "-tts",
    "-image",
    "-audio",
    "embedding",
    "-live",
    "robotics",
    "computer-use",
    "decision",
    "omni",
    "nano-banana",
    "transcribe",
];

/// A chat model this provider can serve. `gemini-2.5-*` is listed upstream
/// but answers 404 "no longer available to new users" for new keys.
/// `supportedGenerationMethods` names no Interactions value, so it is not read.
fn is_chat_model(id: &str) -> bool {
    id.starts_with("gemini-")
        && !id.starts_with("gemini-2.5-")
        && !NON_CHAT.iter().any(|f| id.contains(f))
}

/// Live `/models` reply → enriched catalog rows.
pub fn parse_live_models(json: &Value) -> Vec<Model> {
    let Some(rows) = json.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let id = row["name"].as_str()?;
            let id = id.strip_prefix("models/").unwrap_or(id);
            if !is_chat_model(id) {
                return None;
            }
            let mut model = enrich(id);
            if let Some(n) = row["inputTokenLimit"].as_u64() {
                model.context_window = n;
            }
            // Without a router soft cap the catalog ceiling becomes the
            // request's max_output_tokens, which must stay under the sync cap.
            if let Some(n) = row["outputTokenLimit"].as_u64() {
                model.max_output_tokens = n.min(SYNC_OUTPUT_CAP);
            }
            model.display_name = row["displayName"].as_str().map(str::to_string);
            if row["thinking"] == false {
                model.supports_thinking = Some(false);
                model.supports_xhigh = None;
                model.reasoning_efforts = None;
            }
            Some(model)
        })
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
    auth: &(&'static str, String),
) -> FetchOutcome {
    // shortcut: one page of 1000 rows; follow nextPageToken if the list ever outgrows it.
    let req = http
        .get(url)
        .query(&[("pageSize", "1000")])
        .header(auth.0, &auth.1);
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return FetchOutcome::Transient(format!("models fetch failed: {e}")),
    };
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        // A bad key is HTTP 400 here too; `classify` reads API_KEY_INVALID.
        let body = resp.text().await.unwrap_or_default();
        if classify(Some(status), &body) == ErrorKind::AuthExpired {
            return FetchOutcome::AuthFailed;
        }
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
    let Some(url) = models_url(api_url) else {
        return Err(upstream_unavailable(format!(
            "no models endpoint for api_url {api_url}: it must end in /interactions"
        )));
    };
    let outcome = match auth_header(&credential) {
        Ok(auth) => fetch_live_models(http, &url, &auth).await,
        // An OAuth credential cannot call this API yet: nothing is usable.
        Err(_) => FetchOutcome::AuthFailed,
    };
    match outcome {
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
            models_url(DEFAULT_API_URL).as_deref(),
            Some("https://generativelanguage.googleapis.com/v1beta/models")
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1beta/interactions/").as_deref(),
            Some("http://127.0.0.1:9999/v1beta/models")
        );
        // an unrecognized shape never falls back to the public host
        assert_eq!(models_url("https://proxy.example/custom"), None);
    }

    fn live_list() -> Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/models.json"
        )))
        .unwrap()
    }

    #[test]
    fn live_list_keeps_exactly_the_chat_ids() {
        let models = parse_live_models(&live_list());
        let mut ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "gemini-3-flash-preview",
                "gemini-3.1-flash-lite",
                "gemini-3.1-flash-lite-preview",
                "gemini-3.1-pro-preview",
                "gemini-3.1-pro-preview-customtools",
                "gemini-3.5-flash",
                "gemini-3.5-flash-lite",
                "gemini-3.6-flash",
                "gemini-3.7-flash",
                "gemini-3.8-flash",
                "gemini-4-argon",
                "gemini-4-carbon",
                "gemini-4-hydrogen",
                "gemini-4-neon",
                "gemini-flash-latest",
                "gemini-flash-lite-latest",
                "gemini-pro-latest",
            ]
        );
    }

    #[test]
    fn limits_come_from_the_live_row() {
        let models = parse_live_models(&live_list());
        let get = |id: &str| models.iter().find(|m| m.id == id).unwrap();
        assert_eq!(get("gemini-4-carbon").max_output_tokens, 32_768);
        assert_eq!(get("gemini-3.8-flash").max_output_tokens, 65_536);
        assert_eq!(get("gemini-3.8-flash").context_window, 1_048_576);
        assert_eq!(
            get("gemini-3.8-flash").display_name.as_deref(),
            Some("Gemini 3.8 Flash")
        );
        assert_eq!(get("gemini-3.8-flash").supports_thinking, Some(true));
    }

    #[test]
    fn output_limit_is_clamped_to_the_sync_cap() {
        let rows = serde_json::json!({ "models": [
            { "name": "models/gemini-9-big", "outputTokenLimit": 1_048_576, "thinking": false }
        ]});
        let models = parse_live_models(&rows);
        assert_eq!(models[0].max_output_tokens, 65_536);
        assert_eq!(models[0].supports_thinking, Some(false));
        assert_eq!(models[0].reasoning_efforts, None);
    }
}
