//! Live model discovery: `GET /v1/models` (with the subscription OAuth bearer)
//! is the source of truth for the catalog slice — ids, display names,
//! context/output limits, and the capability tree all come from the API. Ids
//! are namespaced `claude-code/<id>` and carry no pricing (subscription
//! billing). When the endpoint rejects the OAuth token (401/403) the curated
//! fallback slice is reconciled instead of blanking a working provider.
use crate::config::{extract_access_token, DEFAULT_API_URL};
use crate::errors::upstream_unavailable;
use crate::request::{auth_header, ANTHROPIC_VERSION, OAUTH_BETA};
use crate::{auth, curated, router_client, state, PROVIDER_ID};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;
use std::time::Duration;

/// Periodic catalog refresh cadence. Anthropic's catalog moves slowly, so a
/// coarse interval is plenty; boot and router::ready refreshes cover the rest.
const MODELS_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Derive the models endpoint from the configured messages endpoint
/// (`…/v1/messages` → `…/v1/models`). The page size covers Anthropic's
/// full model list in one request.
pub fn models_url(api_url: &str) -> String {
    let base = match api_url.strip_suffix("/messages") {
        Some(base) => base.to_string(),
        None => "https://api.anthropic.com/v1".to_string(),
    };
    format!("{base}/models?limit=1000")
}

/// `capabilities.<path...>.supported` from a live row; absent stays None so
/// downstream gating remains permissive-but-honest.
fn cap_supported(row: &Value, path: &[&str]) -> Option<bool> {
    let mut node = row.get("capabilities")?;
    for key in path {
        node = node.get(key)?;
    }
    node.get("supported").and_then(Value::as_bool)
}

/// One live row → catalog Model, namespaced `claude-code/<id>`. Tolerant of
/// missing fields: an API that stops sending limits or capabilities degrades to
/// conservative defaults, never to a parse failure. No pricing (subscription).
///
/// Generation filter: this provider only implements adaptive thinking, so a
/// model the API marks as thinking-capable but NOT adaptive-capable (the
/// pre-4.6 generation) would 400 the moment a thinking_level arrives — those
/// rows are excluded. Exception: the haiku family (cheap tier, no adaptive
/// release yet) stays with thinking gated off.
fn model_from_live(row: &Value) -> Option<Model> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let thinking = cap_supported(row, &["thinking"]);
    let adaptive = cap_supported(row, &["thinking", "types", "adaptive"]);
    let legacy_thinking_only = thinking == Some(true) && adaptive == Some(false);
    if legacy_thinking_only && !curated::base_id(id).starts_with("claude-haiku-") {
        return None; // legacy-thinking-only generation
    }
    Some(Model {
        id: format!("claude-code/{id}"),
        provider: PROVIDER_ID.into(),
        display_name: row
            .get("display_name")
            .and_then(Value::as_str)
            .map(|n| format!("{n} (Claude Code)")),
        context_window: row
            .get("max_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(200_000),
        max_output_tokens: row
            .get("max_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(8192),
        input_limit: None,
        // Adaptive support is the operative flag for this provider; plain
        // thinking-supported only matters when it is an explicit `false`.
        supports_thinking: if thinking == Some(false) {
            Some(false)
        } else {
            adaptive.or(thinking)
        },
        supports_xhigh: cap_supported(row, &["effort", "xhigh"]),
        reasoning_efforts: None,
        supports_tools: Some(true), // uniform across the Messages API
        supports_vision: cap_supported(row, &["image_input"]),
        supports_cache: Some(true),
        // The provider has no response_format implementation, regardless of
        // what the model itself supports.
        supports_structured_output: Some(false),
        thinking_budgets: None,
        // Subscription billing: no per-token pricing.
        pricing: None,
    })
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(model_from_live).collect())
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
    let (auth_name, auth_value) = auth_header(credential_value);
    let req = http
        .get(url)
        .header(auth_name, auth_value)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", OAUTH_BETA);
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

/// The refresh flow; returns the reconciled slice size. The credential comes
/// from the vault / dev fallback (not the router config, unlike an API-key
/// provider), so discovery and streaming share one OAuth source.
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let api_url = match router_client::resolve(iii, token.as_deref()).await {
        Ok(r) => r.api_url.unwrap_or_else(|| DEFAULT_API_URL.to_string()),
        Err(_) => DEFAULT_API_URL.to_string(),
    };

    // No usable OAuth credential → genuinely unconfigured: prune the slice so
    // the picker reflects that instead of showing rows that can't be called.
    let Some(access_token) = auth::fetch_fresh_credential(iii)
        .await
        .as_ref()
        .and_then(extract_access_token)
    else {
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };

    let url = models_url(&api_url);
    match fetch_live_models(http, &url, &access_token).await {
        FetchOutcome::Ok(models) if !models.is_empty() => {
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        // Live endpoint accepted the token but returned nothing usable, or
        // rejected the OAuth bearer entirely: the token is still good for
        // streaming (that path is separately gated), so reconcile the curated
        // fallback rather than blanking a working provider.
        FetchOutcome::Ok(_) | FetchOutcome::AuthFailed => {
            let models = curated::curated_models();
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
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

/// Periodic catalog reconcile while the worker runs. A transient failure keeps
/// the last known slice; the next tick retries.
pub async fn refresh_models_periodically(iii: IIIClient, http: reqwest::Client) {
    let mut ticker = tokio::time::interval(MODELS_REFRESH_INTERVAL);
    ticker.tick().await; // consume the immediate first tick (boot already refreshed)
    loop {
        ticker.tick().await;
        match refresh_models(&iii, &http).await {
            Ok(count) => println!("[provider-claude-code] periodic catalog refresh: {count} models"),
            Err(e) => eprintln!(
                "[provider-claude-code] periodic catalog refresh failed ({e}); keeping last known catalog"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_url_derives_from_messages_endpoint() {
        assert_eq!(
            models_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/messages"),
            "http://127.0.0.1:9999/v1/models?limit=1000"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
    }

    #[test]
    fn legacy_thinking_only_rows_are_filtered_except_haiku() {
        let legacy_caps = serde_json::json!({
            "thinking": { "supported": true, "types": { "adaptive": { "supported": false } } }
        });
        let json = serde_json::json!({
            "data": [
                { "id": "claude-opus-4-1-20250805", "capabilities": legacy_caps },
                { "id": "claude-haiku-4-5-20251001", "capabilities": legacy_caps },
                {
                    "id": "claude-image-x",
                    "capabilities": { "thinking": { "supported": false } }
                },
            ]
        });
        let models = parse_live_models(&json);
        // non-adaptive thinker dropped; the haiku family (cheap tier, no
        // adaptive release yet) and a can't-think-at-all model both stay
        // with thinking gated off
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "claude-code/claude-haiku-4-5-20251001",
                "claude-code/claude-image-x"
            ]
        );
        assert_eq!(models[0].supports_thinking, Some(false));
        assert_eq!(models[1].supports_thinking, Some(false));
        assert!(models[0].pricing.is_none());
    }

    #[test]
    fn parses_capabilities_limits_and_namespaces_id() {
        let json = serde_json::json!({
            "data": [{
                "id": "claude-opus-4-8",
                "display_name": "Claude Opus 4.8",
                "max_input_tokens": 1_000_000,
                "max_tokens": 128_000,
                "capabilities": {
                    "image_input": { "supported": true },
                    "thinking": {
                        "supported": true,
                        "types": { "adaptive": { "supported": true } }
                    },
                    "effort": {
                        "supported": true,
                        "xhigh": { "supported": true }
                    },
                    "structured_outputs": { "supported": true }
                }
            },
            {
                "id": "claude-sonnet-4-5-20250929",
                "capabilities": {
                    "thinking": {
                        "supported": true,
                        "types": { "adaptive": { "supported": false } }
                    }
                }
            }]
        });
        let models = parse_live_models(&json);
        // the legacy (non-adaptive) sonnet 4.5 row is filtered out
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id, "claude-code/claude-opus-4-8");
        assert_eq!(m.provider, "claude-code");
        assert_eq!(
            m.display_name.as_deref(),
            Some("Claude Opus 4.8 (Claude Code)")
        );
        assert_eq!(m.context_window, 1_000_000);
        assert_eq!(m.max_output_tokens, 128_000);
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_xhigh, Some(true));
        assert_eq!(m.supports_vision, Some(true));
        // provider-local truth, not the model's: no response_format support
        assert_eq!(m.supports_structured_output, Some(false));
        // subscription: no pricing enrichment
        assert!(m.pricing.is_none());
    }

    #[test]
    fn missing_capabilities_degrade_to_conservative_defaults() {
        let json = serde_json::json!({
            "data": [
                { "id": "claude-mystery-9", "display_name": "Mystery" },
                { "id": "" },
                { "display_name": "no id" },
            ]
        });
        let models = parse_live_models(&json);
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id, "claude-code/claude-mystery-9");
        assert_eq!(m.context_window, 200_000);
        assert_eq!(m.max_output_tokens, 8192);
        assert_eq!(m.supports_thinking, None);
        assert_eq!(m.supports_xhigh, None);
        assert!(m.pricing.is_none());
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
