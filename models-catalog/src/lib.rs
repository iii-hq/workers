//! Model capabilities knowledge base.
//!
//! State-first model registry. The bus is the source of truth: each model
//! is stored under `models:<provider>:<id>` in scope `models`, and the
//! `models::list` / `models::get` / `models::supports` iii functions read
//! from there. A new `models::register` iii function lets any caller
//! (router, registry sync worker, user CLI) write models without touching
//! this crate.
//!
//! The compiled-in `data/models.json` is now used **only** as a one-time
//! seed: when `register_with_iii` runs and state is empty, it bulk-loads
//! the embedded baseline so existing setups keep working zero-config.
//! Subsequent registrations win — the embedded copy is never consulted
//! once state has at least one entry.
//!
//! Sync `pub fn list/get/supports` still query the embedded baseline for
//! callers that can't await a bus round-trip (e.g. `context-compaction`'s
//! `CompactionConfig::new`); they're documented as best-effort fallbacks.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

const EMBEDDED_MODELS: &str = include_str!("../data/models.json");

/// Tiered thinking effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    /// Highest tier; only supported by selected model families.
    Xhigh,
}

/// Per-tier token budgets for `ThinkingLevel`. Absent fields fall back to
/// provider defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThinkingBudgets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimal: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<u32>,
}

/// Streaming transport selection for a model adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Sse,
    Websocket,
    #[default]
    Auto,
}

/// Cache-control hint for prompt-cache-aware providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheRetention {
    None,
    #[default]
    Short,
    Long,
}

/// Scope under which model registrations live in iii state.
pub const MODELS_SCOPE: &str = "models";
/// Key prefix for individual model entries: `models:<provider>:<id>`.
pub const MODELS_KEY_PREFIX: &str = "models:";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub provider: String,
    pub api: String,
    pub display_name: String,
    pub context_window: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub supports_thinking: bool,
    #[serde(default)]
    pub supports_xhigh: bool,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_cache: bool,
    #[serde(default)]
    pub thinking_budgets: Option<ThinkingBudgets>,
    #[serde(default)]
    pub transports: Vec<Transport>,
    #[serde(default)]
    pub default_cache_retention: Option<CacheRetention>,
    #[serde(default)]
    pub pricing: Option<Pricing>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pricing {
    pub input_per_1m: f64,
    pub output_per_1m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_1m: Option<f64>,
}

/// Capability query passed to [`supports`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Thinking,
    ThinkingLevel(ThinkingLevel),
    Tools,
    Vision,
    Cache,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    models: Vec<Model>,
}

static CATALOG: Lazy<Vec<Model>> = Lazy::new(|| {
    let parsed: CatalogFile =
        serde_json::from_str(EMBEDDED_MODELS).expect("embedded models.json parses");
    parsed.models
});

/// Filter for [`list`].
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub provider: Option<String>,
    pub capability: Option<Capability>,
}

/// Return all models matching the filter.
pub fn list(filter: &ListFilter) -> Vec<Model> {
    CATALOG
        .iter()
        .filter(|m| filter.provider.as_ref().is_none_or(|p| p == &m.provider))
        .filter(|m| filter.capability.is_none_or(|c| supports_model(m, c)))
        .cloned()
        .collect()
}

/// Look up a single model by `(provider, model_id)`.
pub fn get(provider: &str, model_id: &str) -> Option<Model> {
    CATALOG
        .iter()
        .find(|m| m.provider == provider && m.id == model_id)
        .cloned()
}

/// True when the model supports the requested capability.
pub fn supports(provider: &str, model_id: &str, capability: Capability) -> bool {
    get(provider, model_id).is_some_and(|m| supports_model(&m, capability))
}

fn supports_model(m: &Model, capability: Capability) -> bool {
    match capability {
        Capability::Thinking => m.supports_thinking,
        Capability::ThinkingLevel(ThinkingLevel::Xhigh) => m.supports_xhigh,
        Capability::ThinkingLevel(_) => m.supports_thinking,
        Capability::Tools => m.supports_tools,
        Capability::Vision => m.supports_vision,
        Capability::Cache => m.supports_cache,
    }
}

/// Register `models::*` iii functions on the bus.
///
/// Functions registered:
/// - `models::list` — payload `{ provider?, capability? }`, returns
///   `{ models: [<Model>...] }`. State-first; falls back to the embedded
///   seed when no `models:` keys are registered.
/// - `models::get` — payload `{ provider, model_id }`, returns the model
///   or `null`.
/// - `models::supports` — payload `{ provider, model_id, capability }`,
///   returns `{ supported: bool }`.
/// - `models::register` — payload is a `Model`, writes to state under
///   `models:<provider>:<id>`. Returns `{ key, registered: true }`.
///
/// On registration the embedded baseline is seeded into state once if the
/// `models:` prefix is empty. Existing setups keep zero-config; new
/// deployments can clear state and register their own catalog from
/// scratch.
pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<ModelsFunctionRefs> {
    use iii_sdk::{IIIError, RegisterFunctionMessage};
    use serde_json::{json, Value};

    let _ = seed_state_if_empty(iii).await;

    let iii_for_list = iii.clone();
    let list_fn = iii.register_function((
        RegisterFunctionMessage::with_id("models::list".to_string())
            .with_description("List models, optionally filtered by provider or capability. Reads from iii state; falls back to the embedded seed when state is empty.".into()),
        move |payload: Value| {
            let iii = iii_for_list.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let capability = payload
                    .get("capability")
                    .and_then(Value::as_str)
                    .and_then(parse_capability);
                let filter = ListFilter {
                    provider,
                    capability,
                };
                let models = list_from_state_or_seed(&iii, &filter).await;
                serde_json::to_value(json!({ "models": models }))
                    .map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let iii_for_get = iii.clone();
    let get_fn = iii.register_function((
        RegisterFunctionMessage::with_id("models::get".to_string()).with_description(
            "Look up a single model by (provider, model_id). State-first.".into(),
        ),
        move |payload: Value| {
            let iii = iii_for_get.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: provider".into()))?
                    .to_string();
                let model_id = payload
                    .get("model_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: model_id".into()))?
                    .to_string();
                let model = get_from_state_or_seed(&iii, &provider, &model_id).await;
                serde_json::to_value(model).map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ));

    let iii_for_supports = iii.clone();
    let supports_fn = iii.register_function((
        RegisterFunctionMessage::with_id("models::supports".to_string())
            .with_description("Check whether a model supports a capability. State-first.".into()),
        move |payload: Value| {
            let iii = iii_for_supports.clone();
            async move {
                let provider = payload
                    .get("provider")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: provider".into()))?
                    .to_string();
                let model_id = payload
                    .get("model_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: model_id".into()))?
                    .to_string();
                let capability = payload
                    .get("capability")
                    .and_then(Value::as_str)
                    .and_then(parse_capability)
                    .ok_or_else(|| IIIError::Handler("missing or unknown capability".into()))?;
                let supported = get_from_state_or_seed(&iii, &provider, &model_id)
                    .await
                    .is_some_and(|m| supports_model(&m, capability));
                Ok(json!({ "supported": supported }))
            }
        },
    ));

    let iii_for_register = iii.clone();
    let register_fn = iii.register_function((
        RegisterFunctionMessage::with_id("models::register".to_string())
            .with_description("Write a model to iii state under models:<provider>:<id>.".into()),
        move |payload: Value| {
            let iii = iii_for_register.clone();
            async move {
                let model: Model = serde_json::from_value(payload.clone())
                    .map_err(|e| IIIError::Handler(format!("invalid Model payload: {e}")))?;
                let key = format!("{MODELS_KEY_PREFIX}{}:{}", model.provider, model.id);
                state_set(
                    &iii,
                    &key,
                    &serde_json::to_value(&model).unwrap_or(Value::Null),
                )
                .await
                .map_err(|e| IIIError::Handler(format!("state::set failed: {e}")))?;
                Ok(json!({ "key": key, "registered": true }))
            }
        },
    ));

    Ok(ModelsFunctionRefs {
        list: list_fn,
        get: get_fn,
        supports: supports_fn,
        register: register_fn,
    })
}

/// Seed the embedded baseline into iii state under `models:` if no
/// `models:` keys exist yet. Idempotent and best-effort: a transient bus
/// error or already-seeded state both leave the function silent.
async fn seed_state_if_empty(iii: &iii_sdk::III) -> anyhow::Result<()> {
    use serde_json::Value;
    let existing = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: "state::list".to_string(),
            payload: serde_json::json!({
                "scope": MODELS_SCOPE,
                "prefix": MODELS_KEY_PREFIX,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok();
    let already_seeded = existing
        .as_ref()
        .and_then(|v| {
            v.as_array()
                .cloned()
                .or_else(|| v.get("items").and_then(Value::as_array).cloned())
        })
        .is_some_and(|items| !items.is_empty());
    if already_seeded {
        return Ok(());
    }
    for m in CATALOG.iter() {
        let key = format!("{MODELS_KEY_PREFIX}{}:{}", m.provider, m.id);
        let _ = state_set(iii, &key, &serde_json::to_value(m).unwrap_or(Value::Null)).await;
    }
    Ok(())
}

async fn state_set(
    iii: &iii_sdk::III,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), iii_sdk::IIIError> {
    iii.trigger(iii_sdk::TriggerRequest {
        function_id: "state::set".to_string(),
        payload: serde_json::json!({
            "scope": MODELS_SCOPE,
            "key": key,
            "value": value,
        }),
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .map(|_| ())
}

async fn list_from_state_or_seed(iii: &iii_sdk::III, filter: &ListFilter) -> Vec<Model> {
    use serde_json::Value;
    let resp = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: "state::list".to_string(),
            payload: serde_json::json!({
                "scope": MODELS_SCOPE,
                "prefix": MODELS_KEY_PREFIX,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok();
    let items = resp.as_ref().and_then(|v| {
        v.as_array()
            .cloned()
            .or_else(|| v.get("items").and_then(Value::as_array).cloned())
    });
    let from_state: Vec<Model> = items
        .unwrap_or_default()
        .into_iter()
        .filter_map(|it| {
            let raw = it.get("value").cloned().unwrap_or(it);
            serde_json::from_value::<Model>(raw).ok()
        })
        .collect();
    let source = if from_state.is_empty() {
        CATALOG.clone()
    } else {
        from_state
    };
    source
        .into_iter()
        .filter(|m| filter.provider.as_ref().is_none_or(|p| p == &m.provider))
        .filter(|m| filter.capability.is_none_or(|c| supports_model(m, c)))
        .collect()
}

async fn get_from_state_or_seed(
    iii: &iii_sdk::III,
    provider: &str,
    model_id: &str,
) -> Option<Model> {
    let key = format!("{MODELS_KEY_PREFIX}{provider}:{model_id}");
    let resp = iii
        .trigger(iii_sdk::TriggerRequest {
            function_id: "state::get".to_string(),
            payload: serde_json::json!({
                "scope": MODELS_SCOPE,
                "key": key,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok();
    let raw = resp
        .as_ref()
        .and_then(|v| v.get("value").cloned().or_else(|| Some(v.clone())));
    let from_state = raw.and_then(|r| serde_json::from_value::<Model>(r).ok());
    from_state.or_else(|| get(provider, model_id))
}

fn parse_capability(s: &str) -> Option<Capability> {
    match s {
        "thinking" => Some(Capability::Thinking),
        "thinking:low" => Some(Capability::ThinkingLevel(ThinkingLevel::Low)),
        "thinking:medium" => Some(Capability::ThinkingLevel(ThinkingLevel::Medium)),
        "thinking:high" => Some(Capability::ThinkingLevel(ThinkingLevel::High)),
        "thinking:xhigh" => Some(Capability::ThinkingLevel(ThinkingLevel::Xhigh)),
        "tools" => Some(Capability::Tools),
        "vision" => Some(Capability::Vision),
        "cache" => Some(Capability::Cache),
        _ => None,
    }
}

/// Handles returned by [`register_with_iii`].
pub struct ModelsFunctionRefs {
    pub list: iii_sdk::FunctionRef,
    pub get: iii_sdk::FunctionRef,
    pub supports: iii_sdk::FunctionRef,
    pub register: iii_sdk::FunctionRef,
}

impl ModelsFunctionRefs {
    pub fn unregister_all(self) {
        for r in [self.list, self.get, self.supports, self.register] {
            r.unregister();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_loads() {
        assert!(!CATALOG.is_empty());
    }

    #[test]
    fn list_unfiltered_returns_all() {
        let all = list(&ListFilter::default());
        assert_eq!(all.len(), CATALOG.len());
    }

    #[test]
    fn list_by_provider() {
        let anthropic = list(&ListFilter {
            provider: Some("anthropic".into()),
            capability: None,
        });
        assert!(!anthropic.is_empty());
        assert!(anthropic.iter().all(|m| m.provider == "anthropic"));
    }

    #[test]
    fn list_by_capability_xhigh() {
        let xhigh = list(&ListFilter {
            provider: None,
            capability: Some(Capability::ThinkingLevel(ThinkingLevel::Xhigh)),
        });
        assert!(xhigh.iter().all(|m| m.supports_xhigh));
    }

    #[test]
    fn get_known_model() {
        let m = get("anthropic", "claude-opus-4-7").expect("known model");
        assert_eq!(m.context_window, 1_000_000);
        assert!(m.supports_xhigh);
    }

    #[test]
    fn get_unknown_returns_none() {
        assert!(get("anthropic", "does-not-exist").is_none());
    }

    #[test]
    fn supports_xhigh_is_subset_of_thinking() {
        for m in CATALOG.iter() {
            if m.supports_xhigh {
                assert!(
                    m.supports_thinking,
                    "model {} has xhigh but not thinking",
                    m.id
                );
            }
        }
    }

    #[test]
    fn supports_returns_true_for_known_capability() {
        assert!(supports(
            "anthropic",
            "claude-opus-4-7",
            Capability::ThinkingLevel(ThinkingLevel::Xhigh)
        ));
        assert!(supports("openai", "gpt-5", Capability::Tools));
    }

    #[test]
    fn supports_returns_false_for_unsupported() {
        assert!(!supports(
            "anthropic",
            "claude-haiku-4-5",
            Capability::Thinking
        ));
    }
}
