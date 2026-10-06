//! Live model discovery: `GET /v1/models` lists the loaded model(s); `GET
//! /props` fills in the runtime context size (`n_ctx`, the operator's
//! `--ctx-size`, which is more accurate than `/v1/models`' `meta.n_ctx_train`
//! — the model's *trained* max) and vision/audio modality hints. Unlike
//! every cloud provider here, discovery is attempted **regardless of
//! whether a credential is configured** — most local llama.cpp servers run
//! with no `--api-key` at all — and the catalog is only pruned on an actual
//! 401/403 from the server (an operator-configured key we don't have).
use crate::config::{
    credential_parts, remember_probed_api_url, DEFAULT_API_URL_CANDIDATES, DEFAULT_MAX_TOKENS,
};
use crate::{router_client, state, PROVIDER_ID};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::sse_transport::error_chain;
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
fn enrich(id: &str, model_context_length: Option<u64>, props: &Props) -> Model {
    let context_window = model_context_length
        .filter(|&n| n > 0)
        .or(props.context_window)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW);
    Model {
        id: id.to_string(),
        provider: PROVIDER_ID.to_string(),
        display_name: None,
        context_window,
        // llama.cpp shares n_ctx between prompt and generation; claiming
        // the whole window as output leaves a zero input budget (usable =
        // window - max_output = 0) and every turn dies with
        // context/overflow. Cap output at half the window so small-n_ctx
        // servers (llama-server defaults to 4096) stay usable.
        max_output_tokens: (context_window / 2).min(DEFAULT_MAX_TOKENS),
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        reasoning_efforts: None,
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
        speech: None,
    }
}

fn parse_live_models(json: &Value, props: &Props) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    let id = raw.get("id").and_then(Value::as_str)?;
                    // llama.cpp-compatible frontends (e.g. Unsloth Studio)
                    // have no /props but declare the configured window
                    // per model here; vanilla llama-server omits it.
                    let context_length = raw.get("context_length").and_then(Value::as_u64);
                    Some((id, context_length))
                })
                .filter(|(id, _)| !id.is_empty())
                .map(|(id, context_length)| enrich(id, context_length, props))
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

/// Parameter count in billions read from the id (`Qwen3.8-27B-GGUF:Q8_0` →
/// 27): a number followed by `B`, standing alone between separators. Hand
/// named aliases carry no size and rank after every sized model.
pub fn params_billions(id: &str) -> Option<f64> {
    let b: Vec<char> = id.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let starts_token = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if starts_token && b[i].is_ascii_digit() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == '.') {
                j += 1;
            }
            let ends_token = j + 1 == b.len() || !b[j + 1].is_ascii_alphanumeric();
            if j < b.len() && b[j].eq_ignore_ascii_case(&'b') && ends_token {
                if let Ok(n) = b[i..j].iter().collect::<String>().parse::<f64>() {
                    return Some(n);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// The provider's default-model preference list for this listing: loaded
/// models first (router-mode servers report `status.value`; a classic
/// single-model server reports nothing and is loaded), then larger
/// parameter counts, ties in the server's own order.
pub fn rank_default_models(json: &Value) -> Vec<String> {
    let Some(rows) = json.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut ranked: Vec<(bool, f64, &str)> = rows
        .iter()
        .filter_map(|row| {
            let id = row
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())?;
            let loaded = row
                .pointer("/status/value")
                .and_then(Value::as_str)
                .is_none_or(|v| v == "loaded");
            Some((loaded, params_billions(id).unwrap_or(-1.0), id))
        })
        .collect();
    // Stable: equal keys keep the listing order.
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)));
    ranked
        .into_iter()
        .map(|(_, _, id)| id.to_string())
        .collect()
}

enum FetchOutcome {
    /// The catalog slice and the default-model preference list for it.
    Ok(Vec<Model>, Vec<String>),
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
        Ok(v) => FetchOutcome::Ok(parse_live_models(&v, props), rank_default_models(&v)),
        Err(e) => FetchOutcome::Transient(format!("models response not json: {e}")),
    }
}

/// The servers discovery tries, in order: the configured `api_url` alone, or
/// with none set, llama-server's own default port and then the Llama
/// desktop app's.
pub fn probe_order(configured: Option<&str>) -> Vec<String> {
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(url) => vec![url.to_string()],
        None => DEFAULT_API_URL_CANDIDATES
            .iter()
            .map(|u| u.to_string())
            .collect(),
    }
}

/// The refresh flow; returns the reconciled slice size.
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let configured = resolved.api_url.as_deref();
    let candidates = probe_order(configured);
    let probing = candidates.len() > 1;
    let credential_value = resolved
        .credential
        .as_ref()
        .map(|c| credential_parts(c).trim().to_string())
        .filter(|s| !s.is_empty());

    let mut last_failure = String::new();
    for api_url in &candidates {
        let props = fetch_props(http, &props_url(api_url), credential_value.as_deref()).await;
        match fetch_live_models(
            http,
            &models_url(api_url),
            credential_value.as_deref(),
            &props,
        )
        .await
        {
            FetchOutcome::Ok(models, preferred) => {
                if probing {
                    remember_probed_api_url(api_url);
                }
                // Re-declare the default list before the slice lands so the
                // router never holds a slice without its preference.
                if let Err(e) = crate::register::redeclare_with_defaults(iii, preferred).await {
                    eprintln!("[provider-llamacpp] default-model redeclare failed ({e})");
                }
                let count = models.len();
                router_client::reconcile(iii, models, token.as_deref()).await?;
                return Ok(count);
            }
            FetchOutcome::AuthFailed => {
                // `--api-key` is configured on the server and ours is
                // missing/wrong: the models are genuinely unusable.
                if probing {
                    remember_probed_api_url(api_url);
                }
                router_client::reconcile(iii, vec![], token.as_deref()).await?;
                return Ok(0);
            }
            // Blip: try the next candidate; keep the previous slice if none
            // answers (spec § reconcile-to-empty guidance).
            FetchOutcome::Transient(msg) => last_failure = msg,
        }
    }
    Err(Error::Remote {
        code: "provider/upstream_unavailable".into(),
        message: if probing {
            format!("{last_failure} (tried {})", candidates.join(" and "))
        } else {
            last_failure
        },
        stacktrace: None,
    })
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

    #[test]
    fn parameter_count_comes_from_the_id() {
        assert_eq!(
            params_billions("ggml-org/Qwen3.8-27B-GGUF:Q8_0"),
            Some(27.0)
        );
        assert_eq!(params_billions("Llama-3.2-1.5B-Instruct-Q4_K_M"), Some(1.5));
        assert_eq!(params_billions("gemma-3-270m"), None);
        assert_eq!(params_billions("my-model"), None);
        assert_eq!(params_billions("Q8_0-8bit"), None, "8bit is not a size");
    }

    #[test]
    fn default_preference_is_loaded_first_then_largest_then_listing_order() {
        let listing = json!({ "data": [
            { "id": "small-7B", "status": { "value": "unloaded" } },
            { "id": "big-27B", "status": { "value": "unloaded" } },
            { "id": "tiny-3B", "status": { "value": "loaded" } },
            { "id": "other-7B", "status": { "value": "unloaded" } },
            { "id": "unsized", "status": { "value": "unloaded" } },
        ]});
        assert_eq!(
            rank_default_models(&listing),
            vec!["tiny-3B", "big-27B", "small-7B", "other-7B", "unsized"]
        );
        // Classic single-model llama-server reports no status: it is loaded.
        let classic = json!({ "data": [{ "id": "served-8B" }] });
        assert_eq!(rank_default_models(&classic), vec!["served-8B"]);
        assert!(rank_default_models(&json!({ "object": "list" })).is_empty());
    }

    #[test]
    fn probe_order_is_the_configured_url_or_both_local_defaults() {
        // An operator-set api_url is authoritative; with none, llama-server's
        // 8080 is tried first and the Llama desktop app's 9931 second.
        assert_eq!(
            probe_order(Some("https://box.example/custom")),
            vec!["https://box.example/custom".to_string()]
        );
        assert_eq!(
            probe_order(None),
            vec![
                "http://127.0.0.1:8080/v1/chat/completions".to_string(),
                "http://127.0.0.1:9931/v1/chat/completions".to_string(),
            ]
        );
    }

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
    fn per_model_context_length_beats_props_and_default() {
        // Real /v1/models shape from Unsloth Studio: no /props endpoint
        // exists (it serves the web UI's HTML), but each model row carries
        // the configured window. Regression: this used to fall back to
        // 4096 and register a zero usable budget for a 128k server.
        let json = serde_json::json!({
            "data": [
                { "id": "unsloth/gemma-4-26B-A4B-it-qat-GGUF", "object": "model",
                  "context_length": 128_000, "max_context_length": 4096,
                  "native_context_length": 262_144, "loaded": true },
                { "id": "no-metadata", "object": "model" },
            ]
        });
        let models = parse_live_models(&json, &Props::default());
        assert_eq!(models[0].context_window, 128_000);
        assert_eq!(models[0].max_output_tokens, DEFAULT_MAX_TOKENS);
        // Rows without context_length still fall back (props, then 4096).
        assert_eq!(models[1].context_window, DEFAULT_CONTEXT_WINDOW);

        // Props n_ctx loses to an explicit per-model window, wins over
        // the default; a zero context_length is ignored, not trusted.
        let props = Props {
            context_window: Some(16_384),
            supports_vision: None,
        };
        let models = parse_live_models(&json, &props);
        assert_eq!(models[0].context_window, 128_000);
        assert_eq!(models[1].context_window, 16_384);
        let zero = serde_json::json!({ "data": [{ "id": "z", "context_length": 0 }] });
        assert_eq!(
            parse_live_models(&zero, &Props::default())[0].context_window,
            DEFAULT_CONTEXT_WINDOW
        );
    }

    #[test]
    fn small_n_ctx_leaves_a_nonzero_input_budget() {
        // Regression: with n_ctx <= DEFAULT_MAX_TOKENS the old
        // `context_window.min(DEFAULT_MAX_TOKENS)` made output == window,
        // so context-manager's usable budget (window - output - reserve)
        // clamped to 0 and every turn failed with context/overflow.
        let json = serde_json::json!({ "data": [{ "id": "m" }] });
        for n_ctx in [2048_u64, 4096, 8192] {
            let props = Props {
                context_window: Some(n_ctx),
                supports_vision: None,
            };
            let m = &parse_live_models(&json, &props)[0];
            assert!(
                m.max_output_tokens < m.context_window,
                "n_ctx={n_ctx}: output {} must leave input room in window {}",
                m.max_output_tokens,
                m.context_window
            );
        }
        // Props fetch failed -> 4096 fallback must also leave room.
        let m = &parse_live_models(&json, &Props::default())[0];
        assert!(m.max_output_tokens < m.context_window);
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
