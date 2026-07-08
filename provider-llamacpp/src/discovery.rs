//! Live model discovery: `GET /v1/models` lists the loaded model(s); `GET
//! /props` fills in the runtime context size (`n_ctx`, the operator's
//! `--ctx-size`, which is more accurate than `/v1/models`' `meta.n_ctx_train`
//! — the model's *trained* max) and vision/audio modality hints. Unlike
//! every cloud provider here, discovery is attempted **regardless of
//! whether a credential is configured** — most local llama.cpp servers run
//! with no `--api-key` at all — and the catalog is only pruned on an actual
//! 401/403 from the server (an operator-configured key we don't have).
use crate::config::{credential_parts, DEFAULT_API_URL, DEFAULT_MAX_TOKENS};
use crate::errors::error_chain;
use crate::{router_client, state, PROVIDER_ID};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// llama.cpp's own `--ctx-size` default, used when `/props` is unreachable.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 4096;

/// Derive the models endpoint from the configured completions endpoint
/// (`…/v1/chat/completions` → `…/v1/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => format!("{}/models", api_url.trim_end_matches('/')),
    }
}

/// Derive the `/props` endpoint — a server-root path, not under `/v1`
/// (`…/v1/chat/completions` → `…/props`).
pub fn props_url(api_url: &str) -> String {
    match api_url.strip_suffix("/v1/chat/completions") {
        Some(root) => format!("{root}/props"),
        None => format!("{}/props", api_url.trim_end_matches('/')),
    }
}

/// The subset of `/props` this provider reads for catalog enrichment.
#[derive(Debug, Clone, Copy, Default)]
struct Props {
    context_window: Option<u64>,
    supports_vision: Option<bool>,
}

fn parse_props(json: &Value) -> Props {
    let context_window = json
        .pointer("/default_generation_settings/n_ctx")
        .and_then(Value::as_u64)
        .or_else(|| json.get("n_ctx").and_then(Value::as_u64));
    let supports_vision = json.pointer("/modalities/vision").and_then(Value::as_bool);
    Props {
        context_window,
        supports_vision,
    }
}

/// Conservative defaults, never vanish: llama.cpp's `/v1/models` carries no
/// capability metadata, so every id it serves is listed — there's no
/// "gpt-"-style family gate to apply against an arbitrary GGUF alias.
fn enrich(id: &str, props: &Props) -> Model {
    let context_window = props.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW);
    Model {
        id: id.to_string(),
        provider: PROVIDER_ID.to_string(),
        display_name: None,
        context_window,
        max_output_tokens: context_window.min(DEFAULT_MAX_TOKENS),
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        // Best-effort: requires the server run with --jinja and a
        // tool-capable chat template, which `/props` doesn't report;
        // llama.cpp silently ignores tools a template can't use, so this
        // never breaks non-tool turns.
        supports_tools: Some(true),
        supports_vision: props.supports_vision,
        supports_cache: None,
        // Real grammar/json_schema-constrained decoding, unlike json_object-only
        // providers.
        supports_structured_output: Some(true),
        thinking_budgets: None,
        pricing: None, // self-hosted
    }
}

fn parse_live_models(json: &Value, props: &Props) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| raw.get("id").and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .map(|id| enrich(id, props))
                .collect()
        })
        .unwrap_or_default()
}

async fn fetch_props(http: &reqwest::Client, url: &str, credential: Option<&str>) -> Props {
    let mut req = http.get(url);
    if let Some(c) = credential {
        req = req.header("authorization", format!("Bearer {c}"));
    }
    let Ok(resp) = req.send().await else {
        return Props::default();
    };
    if !resp.status().is_success() {
        return Props::default();
    }
    resp.json::<Value>()
        .await
        .map(|v| parse_props(&v))
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
    credential: Option<&str>,
    props: &Props,
) -> FetchOutcome {
    let mut req = http.get(url);
    if let Some(c) = credential {
        req = req.header("authorization", format!("Bearer {c}"));
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return FetchOutcome::Transient(format!("models fetch failed: {}", error_chain(&e)))
        }
    };
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return FetchOutcome::AuthFailed;
    }
    if !(200..300).contains(&status) {
        return FetchOutcome::Transient(format!("models fetch http {status}"));
    }
    match resp.json::<Value>().await {
        Ok(v) => FetchOutcome::Ok(parse_live_models(&v, props)),
        Err(e) => FetchOutcome::Transient(format!("models response not json: {e}")),
    }
}

/// The refresh flow; returns the reconciled slice size.
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let api_url = resolved
        .api_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_API_URL);
    let credential_value = resolved
        .credential
        .as_ref()
        .map(|c| credential_parts(c).trim().to_string())
        .filter(|s| !s.is_empty());

    let props = fetch_props(http, &props_url(api_url), credential_value.as_deref()).await;
    match fetch_live_models(
        http,
        &models_url(api_url),
        credential_value.as_deref(),
        &props,
    )
    .await
    {
        FetchOutcome::Ok(models) => {
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            // `--api-key` is configured on the server and ours is
            // missing/wrong: the models are genuinely unusable.
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            Ok(0)
        }
        // Blip: keep the previous slice (spec § reconcile-to-empty guidance).
        FetchOutcome::Transient(msg) => Err(Error::Remote {
            code: "provider/upstream_unavailable".into(),
            message: msg,
            stacktrace: None,
        }),
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
            models_url("http://127.0.0.1:8080/v1/chat/completions"),
            "http://127.0.0.1:8080/v1/models"
        );
        assert_eq!(
            models_url("https://box.example/custom"),
            "https://box.example/custom/models"
        );
    }

    #[test]
    fn props_url_derives_the_server_root_not_the_v1_prefix() {
        assert_eq!(
            props_url("http://127.0.0.1:8080/v1/chat/completions"),
            "http://127.0.0.1:8080/props"
        );
    }

    #[test]
    fn parses_every_id_with_no_family_gate() {
        // llama.cpp serves arbitrary GGUF aliases — no "gpt-"-style filter
        // applies; every id is listed (conservative defaults, never vanish).
        let json = serde_json::json!({
            "data": [
                { "id": "qwen2.5-coder-7b-instruct", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "llama-3.2-3b-instruct", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json, &Props::default())
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["qwen2.5-coder-7b-instruct", "llama-3.2-3b-instruct"]);
    }

    #[test]
    fn enrichment_uses_props_context_and_falls_back_without_it() {
        let json = serde_json::json!({ "data": [{ "id": "m" }] });
        let props = Props {
            context_window: Some(32_768),
            supports_vision: Some(true),
        };
        let models = parse_live_models(&json, &props);
        assert_eq!(models[0].context_window, 32_768);
        assert_eq!(models[0].max_output_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(models[0].supports_vision, Some(true));
        assert_eq!(models[0].supports_structured_output, Some(true));
        assert_eq!(models[0].pricing, None);

        let models = parse_live_models(&json, &Props::default());
        assert_eq!(models[0].context_window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(models[0].supports_vision, None);
    }

    #[test]
    fn parse_props_reads_n_ctx_and_vision_modality() {
        let json = serde_json::json!({
            "default_generation_settings": { "n_ctx": 8192 },
            "modalities": { "vision": true, "audio": false }
        });
        let props = parse_props(&json);
        assert_eq!(props.context_window, Some(8192));
        assert_eq!(props.supports_vision, Some(true));
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({}), &Props::default()).is_empty());
        assert!(
            parse_live_models(&serde_json::json!({ "data": "nope" }), &Props::default()).is_empty()
        );
    }
}
