//! Catalog reconcile. Groq's `GET /models` is the source of truth for the
//! id list, and unusually it carries the context window and an active flag
//! per model too. Those are taken live; everything the listing cannot say
//! (display name, output ceiling, capabilities, pricing) is enriched from
//! the local table (curated.rs) and pushed through the router's single
//! write path.
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

/// Derive the models endpoint from the generation endpoint — the sibling
/// of the configured chat route, so an override pointed at a gateway finds
/// its listing on the same host.
pub fn models_url(api_url: &str) -> String {
    api_url
        .trim_end_matches('/')
        .strip_suffix("/chat/completions")
        .map(|base| format!("{base}/models"))
        .unwrap_or_else(|| "https://api.groq.com/openai/v1/models".to_string())
}

/// `{ "data": [ { "id", "context_window", "active" } ] }` → enriched catalog
/// rows.
///
/// Groq's listing says more than most: it carries the context window per
/// model and marks whether the model is currently serving. Both are taken
/// over the local snapshot, because the live answer is the true one — a
/// window Groq raises reaches the router without a release, and a model that
/// is not `active` cannot serve a turn, so offering it would only produce a
/// failure the picker could have avoided.
///
/// Speech and moderation models share this listing with chat models. They
/// have no chat completion surface, so serving them would put unroutable rows
/// in the picker; they are dropped by the absence of a context window, which
/// is what distinguishes them here.
pub fn parse_live_models(json: &Value) -> Vec<Model> {
    let Some(rows) = json.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    // Whether this listing reports context windows at all. Groq does, and a
    // row without one is a speech or moderation model rather than a chat
    // model. A gateway that reports none for anything is a different story:
    // requiring the field there would empty the catalog, so the rule only
    // applies when the listing has shown it knows how to speak it.
    let reports_windows = rows
        .iter()
        .any(|raw| raw.get("context_window").and_then(Value::as_u64).is_some());

    rows.iter()
        .filter(|raw| {
            // Absent `active` means an older or proxied listing that does not
            // report it: serve the model rather than hide it.
            raw.get("active").and_then(Value::as_bool).unwrap_or(true)
        })
        .filter_map(|raw| {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())?;
            let window = raw
                .get("context_window")
                .and_then(Value::as_u64)
                .filter(|w| *w > 0);
            if reports_windows && window.is_none() {
                return None;
            }
            let mut model = enrich(id);
            if let Some(window) = window {
                model.context_window = window;
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
            "https://api.groq.com/openai/v1/models"
        );
        assert_eq!(
            models_url("https://api.groq.com/openai/v1/chat/completions/"),
            "https://api.groq.com/openai/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.groq.com/openai/v1/models"
        );
    }

    #[test]
    fn live_ids_are_enriched_and_malformed_rows_skipped() {
        let json = serde_json::json!({
            "object": "list",
            "data": [
                { "id": "llama-3.1-8b-instant", "context_window": 131072, "active": true },
                { "id": "llama-3.3-70b-versatile", "context_window": 131072, "active": true },
                { "id": "", "context_window": 131072 },
                { "context_window": 131072 },
            ]
        });
        let models = parse_live_models(&json);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["llama-3.1-8b-instant", "llama-3.3-70b-versatile"]);
        assert_eq!(
            models[1].display_name.as_deref(),
            Some("Llama 3.3 70B Versatile")
        );
    }

    #[test]
    fn the_live_context_window_wins_over_the_local_snapshot() {
        // Groq raising a window should reach the router without a release.
        let json = serde_json::json!({
            "data": [{ "id": "llama-3.3-70b-versatile", "context_window": 262_144 }]
        });
        let models = parse_live_models(&json);
        assert_eq!(models[0].context_window, 262_144);
    }

    #[test]
    fn inactive_models_are_not_offered() {
        // A model that cannot serve a turn would only produce a failure the
        // picker could have avoided.
        let json = serde_json::json!({
            "data": [
                { "id": "llama-3.1-8b-instant", "context_window": 131072, "active": true },
                { "id": "retired-model", "context_window": 131072, "active": false },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["llama-3.1-8b-instant"]);
    }

    #[test]
    fn speech_models_sharing_the_listing_are_dropped() {
        // Whisper has no chat completion surface, and no context window in the
        // listing, which is how it is told apart from a chat model.
        let json = serde_json::json!({
            "data": [
                { "id": "llama-3.1-8b-instant", "context_window": 131072 },
                { "id": "whisper-large-v3", "active": true },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["llama-3.1-8b-instant"]);
    }

    #[test]
    fn a_listing_that_reports_no_windows_at_all_keeps_every_row() {
        // A gateway that omits the field for everything must not be emptied:
        // the rule only applies once the listing has shown it speaks it.
        let json = serde_json::json!({
            "data": [{ "id": "some-proxied-model" }, { "id": "another-one" }]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["some-proxied-model", "another-one"]);
    }

    #[test]
    fn unknown_ids_survive_discovery_with_defaults() {
        // A model Groq ships before this table is updated must still be
        // routable — the row degrades, it never disappears.
        let json = serde_json::json!({ "data": [{ "id": "brand-new-model" }] });
        let models = parse_live_models(&json);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "brand-new-model");
        assert_eq!(models[0].display_name, None);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
