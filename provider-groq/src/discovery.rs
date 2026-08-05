//! Catalog reconcile. Groq's `GET /models` is the source of truth for far
//! more than the id list: display name, context window, output ceiling,
//! modalities, supported features and live per-token pricing all come from
//! it, and are pushed through the router's single write path. Only the
//! floor for a row that reports none of this lives locally (curated.rs).
//! The configured credential gates the slice — no key → empty catalog, so the
//! picker never shows unusable rows.
use crate::config::{credential_parts, DEFAULT_API_URL};
use crate::curated::base;
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::{Model, Pricing};
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

/// One live listing row → a catalog Model.
///
/// Groq reports far more than a provider usually does — display name, window,
/// output ceiling, modalities, supported features, and live per-token pricing
/// — so the row is read rather than looked up. Anything the listing omits
/// falls back to [`base`], which claims no capability it has not been told
/// about.
fn from_listing(raw: &Value) -> Option<Model> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;

    // A model that is not serving cannot answer a turn, so offering it would
    // only produce a failure the picker could have avoided. An absent flag
    // means a listing that does not report it: serve the model.
    if !raw.get("active").and_then(Value::as_bool).unwrap_or(true) {
        return None;
    }
    // Speech and moderation models share this listing with chat models and
    // have no chat completion surface. Modality is what tells them apart:
    // Whisper reports a context window like everything else (448), so a
    // missing-window rule would let it through.
    if !is_chat_model(raw) {
        return None;
    }

    // An absent list and an empty one say different things: a listing that
    // omits the field has told us nothing, while one that sends `[]` has said
    // the model supports none of these. Only the first leaves a capability
    // unknown.
    let features = raw
        .get("supported_features")
        .and_then(Value::as_array)
        .map(|f| {
            f.iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        });
    let declared = |name: &str| features.as_ref().map(|list| list.iter().any(|f| f == name));

    let mut model = base(id);
    if let Some(name) = raw.get("name").and_then(Value::as_str) {
        model.display_name = Some(name.to_string());
    }
    if let Some(window) = raw
        .get("context_window")
        .and_then(Value::as_u64)
        .filter(|w| *w > 0)
    {
        model.context_window = window;
    }
    if let Some(output) = raw
        .get("max_completion_tokens")
        .and_then(Value::as_u64)
        .filter(|o| *o > 0)
    {
        model.max_output_tokens = output;
    }
    model.supports_tools = declared("tools");
    model.supports_thinking = declared("reasoning");
    model.supports_structured_output = declared("structured_outputs");
    if let Some(modalities) = input_modalities(raw) {
        model.supports_vision = Some(modalities.iter().any(|m| m == "image"));
    }
    // Groq's ladder of reasoning efforts stops at `high`; nothing here has a
    // tier above it.
    if model.supports_thinking == Some(true) {
        model.supports_xhigh = Some(false);
    }
    model.pricing = pricing_from(raw);
    Some(model)
}

/// Whether a listing row is a chat model: it takes text in and produces text
/// out. Audio in (Whisper) or speech out (Orpheus) is a different surface
/// this worker does not serve.
fn is_chat_model(raw: &Value) -> bool {
    let text_in = input_modalities(raw).is_none_or(|m| m.iter().any(|m| m == "text"));
    let text_out =
        modalities(raw, "output_modalities").is_none_or(|m| m.iter().any(|m| m == "text"));
    text_in && text_out
}

fn input_modalities(raw: &Value) -> Option<Vec<String>> {
    modalities(raw, "input_modalities")
}

fn modalities(raw: &Value, field: &str) -> Option<Vec<String>> {
    let list: Vec<String> = raw
        .get(field)?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_ascii_lowercase)
        .collect();
    (!list.is_empty()).then_some(list)
}

/// Groq quotes prices per single token as strings; the catalog carries USD
/// per MTok, so each is scaled. A price that will not parse is dropped rather
/// than guessed at — a wrong number on a cost display is worse than none.
fn pricing_from(raw: &Value) -> Option<Pricing> {
    let pricing = raw.get("pricing")?;
    let per_mtok = |field: &str| -> Option<f64> {
        pricing
            .get(field)?
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            // Scaling by a million leaves binary-float dust — 0.00000079
            // becomes 0.7899999999999999, which would reach a cost display
            // verbatim. Six decimals is finer than any published rate.
            .map(|per_token| (per_token * 1_000_000.0 * 1_000_000.0).round() / 1_000_000.0)
    };
    let input = per_mtok("prompt");
    let output = per_mtok("completion");
    let cache_read = per_mtok("input_cache_read");
    (input.is_some() || output.is_some()).then_some(Pricing {
        input,
        output,
        cache_read,
        cache_write: None,
    })
}

/// `{ "data": [ … ] }` → enriched catalog rows.
pub fn parse_live_models(json: &Value) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(from_listing).collect())
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

    /// A row exactly as `GET /models` returns it, captured from the live API.
    /// Testing against invented shapes is how a listing parser passes while
    /// the real one does not.
    fn live_row(id: &str) -> Value {
        match id {
            "llama-3.3-70b-versatile" => serde_json::json!({
                "id": "llama-3.3-70b-versatile", "object": "model", "owned_by": "Meta",
                "active": true, "context_window": 131072, "max_completion_tokens": 32768,
                "hugging_face_id": "meta-llama/Llama-3.3-70B-Instruct",
                "name": "Llama 3.3 70B Versatile",
                "input_modalities": ["text"], "output_modalities": ["text"],
                "pricing": { "prompt": "0.00000059", "completion": "0.00000079",
                             "input_cache_read": "0" },
                "supported_features": ["tools", "json_mode"],
            }),
            "openai/gpt-oss-20b" => serde_json::json!({
                "id": "openai/gpt-oss-20b", "object": "model", "owned_by": "OpenAI",
                "active": true, "context_window": 131072, "max_completion_tokens": 65536,
                "hugging_face_id": "openai/gpt-oss-20b", "name": "GPT OSS 20B",
                "input_modalities": ["text"], "output_modalities": ["text"],
                "pricing": { "prompt": "0.000000075", "completion": "0.0000003",
                             "input_cache_read": "0.0000000375" },
                "supported_features": ["tools", "json_mode", "structured_outputs", "reasoning"],
            }),
            "qwen/qwen3.6-27b" => serde_json::json!({
                "id": "qwen/qwen3.6-27b", "object": "model", "active": true,
                "context_window": 131072, "max_completion_tokens": 16384,
                "name": "Qwen/Qwen3.6-27B",
                "input_modalities": ["text", "image"], "output_modalities": ["text"],
                "supported_features": ["tools", "json_mode", "reasoning"],
            }),
            "whisper-large-v3" => serde_json::json!({
                "id": "whisper-large-v3", "object": "model", "active": true,
                "context_window": 448, "max_completion_tokens": 448,
                "input_modalities": ["audio"], "output_modalities": ["transcription"],
                "supported_features": [],
            }),
            "canopylabs/orpheus-v1-english" => serde_json::json!({
                "id": "canopylabs/orpheus-v1-english", "object": "model", "active": true,
                "context_window": 4000, "max_completion_tokens": 50000,
                "input_modalities": ["text"], "output_modalities": ["speech"],
                "supported_features": [],
            }),
            other => serde_json::json!({ "id": other, "object": "model" }),
        }
    }

    #[test]
    fn a_live_row_is_read_rather_than_looked_up() {
        let m = from_listing(&live_row("llama-3.3-70b-versatile")).unwrap();
        assert_eq!(m.display_name.as_deref(), Some("Llama 3.3 70B Versatile"));
        assert_eq!(m.context_window, 131_072);
        assert_eq!(m.max_output_tokens, 32_768);
        assert_eq!(m.supports_tools, Some(true));
        // Groq quotes per single token; the catalog carries per MTok.
        let p = m.pricing.unwrap();
        assert_eq!(p.input, Some(0.59));
        assert_eq!(p.output, Some(0.79));
    }

    #[test]
    fn capabilities_come_from_the_listing_and_differ_between_families() {
        // The whole reason none of this is a provider-wide constant.
        let llama = from_listing(&live_row("llama-3.3-70b-versatile")).unwrap();
        let gpt_oss = from_listing(&live_row("openai/gpt-oss-20b")).unwrap();
        let qwen = from_listing(&live_row("qwen/qwen3.6-27b")).unwrap();

        assert_eq!(llama.supports_thinking, Some(false));
        assert_eq!(gpt_oss.supports_thinking, Some(true));
        assert_eq!(gpt_oss.supports_structured_output, Some(true));
        assert_eq!(llama.supports_structured_output, Some(false));
        // Vision is per model too: Qwen takes images, Llama does not.
        assert_eq!(qwen.supports_vision, Some(true));
        assert_eq!(llama.supports_vision, Some(false));
    }

    #[test]
    fn speech_models_are_dropped_by_modality_not_by_window() {
        // Whisper reports a context window (448) like everything else, so a
        // missing-window rule would let it into the picker.
        assert!(from_listing(&live_row("whisper-large-v3")).is_none());
        assert!(from_listing(&live_row("canopylabs/orpheus-v1-english")).is_none());
        assert!(from_listing(&live_row("llama-3.3-70b-versatile")).is_some());
    }

    #[test]
    fn an_inactive_model_is_not_offered() {
        let mut row = live_row("llama-3.3-70b-versatile");
        row["active"] = serde_json::json!(false);
        assert!(from_listing(&row).is_none());
    }

    #[test]
    fn a_sparse_row_survives_on_the_floor_claiming_nothing() {
        // A gateway behind an api_url override that reports only ids: the
        // model still routes, and no capability is invented for it.
        let m = from_listing(&live_row("some-proxied-model")).unwrap();
        assert_eq!(m.id, "some-proxied-model");
        assert_eq!(m.context_window, crate::curated::UNKNOWN_CONTEXT_WINDOW);
        assert_eq!(m.supports_tools, None);
        assert_eq!(m.supports_vision, None);
        assert!(m.pricing.is_none());
    }

    #[test]
    fn a_price_that_will_not_parse_is_dropped_rather_than_guessed() {
        let mut row = live_row("llama-3.3-70b-versatile");
        row["pricing"] = serde_json::json!({ "prompt": "free", "completion": "free" });
        assert!(from_listing(&row).unwrap().pricing.is_none());
    }

    #[test]
    fn malformed_rows_are_skipped_and_bad_payloads_yield_empty() {
        let json = serde_json::json!({
            "data": [
                live_row("llama-3.3-70b-versatile"),
                { "id": "" },
                { "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["llama-3.3-70b-versatile"]);
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
