//! Live model discovery: `GET /v1/models` is the source of truth for the
//! catalog's id list — all models returned are valid chat models, no family
//! filtering, no legacy dedup, no curated enrichment.
//!
//! Model metadata (context window, reasoning, tool support) is sourced from
//! [models.dev](https://models.dev/api.json), a third-party aggregate. The
//! provider fetches it once per refresh and merges it into the model list.
//! If models.dev is unreachable the model list still works with conservative
//! defaults.
use crate::config::DEFAULT_API_URL;
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::{Model, ReasoningEffort};
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;
use std::collections::HashMap;

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const DEFAULT_CONTEXT: u64 = 128_000;

/// Per-model metadata sourced from models.dev.
#[derive(Clone)]
pub struct ModelsDevMeta {
    pub context_window: u64,
    pub supports_thinking: Option<bool>,
    pub reasoning_efforts: Option<Vec<ReasoningEffort>>,
    pub supports_tools: Option<bool>,
    pub supports_structured_output: Option<bool>,
}

impl Default for ModelsDevMeta {
    fn default() -> Self {
        Self {
            context_window: DEFAULT_CONTEXT,
            supports_thinking: None,
            reasoning_efforts: None,
            supports_tools: None,
            supports_structured_output: None,
        }
    }
}
/// Fetch the [models.dev](https://models.dev/api.json) aggregate and extract
/// the `opencode` provider's model metadata. Returns empty when unreachable
/// or unparseable so the model list degrades gracefully.
async fn fetch_models_dev_metadata(http: &reqwest::Client) -> HashMap<String, ModelsDevMeta> {
    let Ok(resp) = http.get(MODELS_DEV_URL).send().await else {
        return HashMap::new();
    };
    if !resp.status().is_success() {
        return HashMap::new();
    }
    let Ok(raw) = resp.json::<Value>().await else {
        return HashMap::new();
    };
    let Some(opencode) = raw.get("opencode") else {
        return HashMap::new();
    };
    let Some(models) = opencode.get("models") else {
        return HashMap::new();
    };
    let Some(models) = models.as_object() else {
        return HashMap::new();
    };

    let mut map = HashMap::with_capacity(models.len());
    for (id, meta) in models {
        let ctx = meta
            .get("limit")
            .and_then(|l| l.get("context"))
            .and_then(|c| c.as_u64())
            .unwrap_or(DEFAULT_CONTEXT);
        let reasoning = meta
            .get("reasoning")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);

        let reasoning_efforts = meta
            .get("reasoning_options")
            .and_then(|ro| ro.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|opt| {
                        let ty = opt.get("type")?.as_str()?;
                        if ty == "effort" {
                            opt.get("values")?.as_array().map(|vals| {
                                vals.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect::<Vec<_>>()
                            })
                        } else {
                            None
                        }
                    })
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let tool_call = meta
            .get("tool_call")
            .and_then(|t| t.as_bool())
            .unwrap_or(false);
        let structured_output = meta
            .get("structured_output")
            .and_then(|s| s.as_bool())
            .unwrap_or(false);

        map.insert(
            id.clone(),
            ModelsDevMeta {
                context_window: ctx,
                supports_thinking: if reasoning { Some(true) } else { None },
                reasoning_efforts: if reasoning_efforts.is_empty() {
                    None
                } else {
                    Some(
                        reasoning_efforts
                            .into_iter()
                            .map(|e| ReasoningEffort {
                                effort: e,
                                description: None,
                            })
                            .collect(),
                    )
                },
                supports_tools: if tool_call { Some(true) } else { None },
                supports_structured_output: if structured_output { Some(true) } else { None },
            },
        );
    }
    map
}

/// Derive the models endpoint from the generation endpoint.
pub fn models_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');
    trimmed
        .strip_suffix("/chat/completions")
        .map(|base| format!("{base}/models"))
        .unwrap_or_else(|| "https://opencode.ai/zen/go/v1/models".to_string())
}

/// Creates a Model from the raw API id, enriched with optional models.dev
/// metadata. When metadata is unavailable all models get conservative
/// defaults; `reasoning.rs` still resolves `supports_thinking` and
/// `reasoning_effort` by ID pattern at stream time.
fn enrich_opencode_go(id: &str, meta: Option<&ModelsDevMeta>) -> Model {
    let m = meta.cloned().unwrap_or_default();
    Model {
        id: id.to_string(),
        display_name: Some(id.to_string()),
        provider: crate::PROVIDER_ID.to_string(),
        context_window: m.context_window,
        max_output_tokens: 4096,
        input_limit: None,
        supports_thinking: m.supports_thinking,
        supports_xhigh: None,
        reasoning_efforts: m.reasoning_efforts,
        supports_tools: m.supports_tools.or(Some(true)),
        supports_vision: None,
        supports_cache: None,
        supports_structured_output: m.supports_structured_output,
        thinking_budgets: None,
        pricing: None,
    }
}

pub fn parse_live_models(json: &Value, metadata: &HashMap<String, ModelsDevMeta>) -> Vec<Model> {
    let ids: Vec<String> = json
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

    ids.iter()
        .map(|id| enrich_opencode_go(id, metadata.get(id)))
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
    metadata: &HashMap<String, ModelsDevMeta>,
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
        Ok(v) => FetchOutcome::Ok(parse_live_models(&v, metadata)),
        Err(e) => FetchOutcome::Transient(format!("models response not json: {e}")),
    }
}

/// The refresh flow; returns the reconciled slice size.
/// Fetches the live model list AND models.dev metadata, merging them.
pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let Some(credential) = resolved.credential else {
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let credential_value = crate::config::credential_parts(&credential);

    // Fetch models.dev metadata (best-effort, may be empty).
    let metadata = fetch_models_dev_metadata(http).await;

    let url = models_url(resolved.api_url.as_deref().unwrap_or(DEFAULT_API_URL));
    match fetch_live_models(http, &url, credential_value, &metadata).await {
        FetchOutcome::Ok(models) => {
            let count = models.len();
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            Ok(0)
        }
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
    fn models_url_derives_from_generation_endpoint() {
        assert_eq!(
            models_url("https://opencode.ai/zen/go/v1/chat/completions"),
            "https://opencode.ai/zen/go/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://opencode.ai/zen/go/v1/models"
        );
    }

    #[test]
    fn parses_all_ids() {
        let metadata: HashMap<String, ModelsDevMeta> = HashMap::new();
        let json = serde_json::json!({
            "data": [
                { "id": "deepseek-v4-flash", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "kimi-k2.7-code", "object": "model" },
                { "id": "qwen2.5-coder-7b-instruct", "object": "model" },
            ]
        });
        let models = parse_live_models(&json, &metadata);
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "deepseek-v4-flash");
        assert!(models[0].supports_thinking.is_none());
        assert_eq!(models[1].id, "kimi-k2.7-code");
        assert_eq!(models[2].id, "qwen2.5-coder-7b-instruct");
        assert!(models[2].supports_thinking.is_none());
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        let metadata: HashMap<String, ModelsDevMeta> = HashMap::new();
        assert!(parse_live_models(&serde_json::json!({}), &metadata).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" }), &metadata).is_empty());
    }

    #[test]
    fn enrichment_uses_metadata_when_available() {
        let id = "deepseek-v4-flash";
        let mut metadata: HashMap<String, ModelsDevMeta> = HashMap::new();
        metadata.insert(
            id.to_string(),
            ModelsDevMeta {
                context_window: 1_000_000,
                supports_thinking: Some(true),
                reasoning_efforts: Some(vec![
                    ReasoningEffort {
                        effort: "low".into(),
                        description: None,
                    },
                    ReasoningEffort {
                        effort: "medium".into(),
                        description: None,
                    },
                    ReasoningEffort {
                        effort: "high".into(),
                        description: None,
                    },
                ]),
                supports_tools: Some(true),
                supports_structured_output: Some(true),
            },
        );

        let m = enrich_opencode_go(id, metadata.get(id));
        assert_eq!(m.display_name.as_deref(), Some("deepseek-v4-flash"));
        assert_eq!(m.context_window, 1_000_000);
        assert!(m.supports_thinking.unwrap_or(false));
        assert!(m.reasoning_efforts.is_some());
        assert!(m.supports_tools.unwrap_or(false));
        assert!(m.supports_structured_output.unwrap_or(false));
    }

    #[test]
    fn enrichment_falls_back_to_defaults() {
        let metadata: HashMap<String, ModelsDevMeta> = HashMap::new();
        for id in ["unknown-model", "grok-4.5", "minimax-m3"] {
            let m = enrich_opencode_go(id, metadata.get(id));
            assert_eq!(m.display_name.as_deref(), Some(id));
            assert_eq!(m.context_window, DEFAULT_CONTEXT);
            assert!(m.supports_thinking.is_none());
            assert!(m.reasoning_efforts.is_none());
        }
    }
}
