//! Live-metadata mapping for the OpenRouter slice: one `GET /api/v1/models`
//! row → one catalog `Model`. Unlike the single-vendor providers, whose
//! listings return bare ids and need a hand-maintained metadata table,
//! OpenRouter's listing is self-describing — context length, output ceiling,
//! per-token pricing, modalities, and supported parameters all ride the row —
//! so live discovery owns the whole record and new upstream models need no
//! release here.
//!
//! Catalog ids are prefixed `openrouter/` (`openrouter/anthropic/claude-x`):
//! OpenRouter's own ids are `vendor/model`, which without a prefix would read
//! as belonging to the sibling single-vendor providers in the picker. The
//! prefix is stripped again on every upstream call.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, Pricing, ReasoningEffort};
use serde_json::Value;

/// Catalog-id prefix; also what `upstream_id` strips before the wire.
pub const MODEL_PREFIX: &str = "openrouter/";

/// Fallbacks for rows missing limits — conservative, so a wrong guess
/// under-fills a context instead of overflowing it upstream.
const FALLBACK_CONTEXT_WINDOW: u64 = 32_768;
const FALLBACK_MAX_OUTPUT: u64 = 16_384;

/// OpenRouter model id → catalog id (`anthropic/claude-x` →
/// `openrouter/anthropic/claude-x`).
pub fn catalog_id(upstream: &str) -> String {
    format!("{MODEL_PREFIX}{upstream}")
}

/// Catalog id → the id OpenRouter's API expects. Tolerates an already-bare id
/// so a caller passing an unprefixed model still routes.
pub fn upstream_id(catalog: &str) -> &str {
    catalog.strip_prefix(MODEL_PREFIX).unwrap_or(catalog)
}

/// OpenRouter prices are strings in USD per token; the catalog wants USD per
/// million tokens. Non-numeric or missing → None; a zero price (free variants)
/// is kept — it is real data, not absence.
fn price_per_mtok(pricing: &Value, key: &str) -> Option<f64> {
    let per_token: f64 = pricing.get(key)?.as_str()?.trim().parse().ok()?;
    Some(per_token * 1_000_000.0)
}

fn str_array(v: Option<&Value>) -> impl Iterator<Item = &str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn supports(row: &Value, parameter: &str) -> bool {
    str_array(row.get("supported_parameters")).any(|p| p == parameter)
}

/// One listing row → catalog Model. `None` when the row has no usable id.
/// Admission (tools + text output) is discovery's concern, not this mapping's.
pub fn model_from_row(row: &Value) -> Option<Model> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;

    let context_window = row
        .get("context_length")
        .and_then(Value::as_u64)
        .filter(|c| *c > 0)
        .unwrap_or(FALLBACK_CONTEXT_WINDOW);
    let max_output_tokens = row
        .pointer("/top_provider/max_completion_tokens")
        .and_then(Value::as_u64)
        .filter(|m| *m > 0)
        .unwrap_or(FALLBACK_MAX_OUTPUT)
        .min(context_window);

    let supports_vision =
        str_array(row.pointer("/architecture/input_modalities")).any(|m| m == "image");

    let efforts: Vec<ReasoningEffort> = str_array(row.pointer("/reasoning/supported_efforts"))
        .map(|e| ReasoningEffort {
            effort: e.to_string(),
            description: None,
        })
        .collect();
    let supports_thinking = supports(row, "reasoning");

    let pricing_row = row.get("pricing").cloned().unwrap_or(Value::Null);
    let pricing = Pricing {
        input: price_per_mtok(&pricing_row, "prompt"),
        output: price_per_mtok(&pricing_row, "completion"),
        cache_read: price_per_mtok(&pricing_row, "input_cache_read"),
        cache_write: price_per_mtok(&pricing_row, "input_cache_write"),
    };
    let has_pricing = pricing.input.is_some() || pricing.output.is_some();
    let supports_cache = pricing.cache_read.is_some();

    Some(Model {
        id: catalog_id(id),
        provider: PROVIDER_ID.into(),
        display_name: row
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        context_window,
        max_output_tokens,
        input_limit: None,
        supports_thinking: Some(supports_thinking),
        supports_xhigh: if supports_thinking {
            Some(efforts.iter().any(|e| e.effort == "xhigh"))
        } else {
            None
        },
        reasoning_efforts: if efforts.is_empty() {
            None
        } else {
            Some(efforts)
        },
        supports_tools: Some(supports(row, "tools")),
        supports_vision: Some(supports_vision),
        supports_cache: if supports_cache { Some(true) } else { None },
        supports_structured_output: Some(supports(row, "structured_outputs")),
        thinking_budgets: None,
        pricing: if has_pricing { Some(pricing) } else { None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn full_row() -> Value {
        json!({
            "id": "anthropic/claude-x",
            "name": "Anthropic: Claude X",
            "context_length": 1_000_000,
            "architecture": {
                "input_modalities": ["text", "image", "file"],
                "output_modalities": ["text"]
            },
            "pricing": {
                "prompt": "0.00001",
                "completion": "0.00005",
                "input_cache_read": "0.000001",
                "input_cache_write": "0.0000125"
            },
            "top_provider": { "max_completion_tokens": 128_000 },
            "supported_parameters": ["tools", "reasoning", "structured_outputs", "response_format"],
            "reasoning": { "supported_efforts": ["xhigh", "high", "medium", "low"] }
        })
    }

    #[test]
    fn id_round_trips_through_the_prefix() {
        assert_eq!(
            catalog_id("anthropic/claude-x"),
            "openrouter/anthropic/claude-x"
        );
        assert_eq!(
            upstream_id("openrouter/anthropic/claude-x"),
            "anthropic/claude-x"
        );
        // already-bare ids pass through
        assert_eq!(upstream_id("anthropic/claude-x"), "anthropic/claude-x");
    }

    #[test]
    fn full_row_maps_every_capability_field() {
        let m = model_from_row(&full_row()).unwrap();
        assert_eq!(m.id, "openrouter/anthropic/claude-x");
        assert_eq!(m.provider, "openrouter");
        assert_eq!(m.display_name.as_deref(), Some("Anthropic: Claude X"));
        assert_eq!(m.context_window, 1_000_000);
        assert_eq!(m.max_output_tokens, 128_000);
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_vision, Some(true));
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_xhigh, Some(true));
        assert_eq!(m.supports_structured_output, Some(true));
        assert_eq!(m.supports_cache, Some(true));
        let efforts: Vec<&str> = m
            .reasoning_efforts
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| e.effort.as_str())
            .collect();
        assert_eq!(efforts, ["xhigh", "high", "medium", "low"]);
    }

    #[test]
    fn pricing_converts_per_token_strings_to_per_mtok() {
        let p = model_from_row(&full_row()).unwrap().pricing.unwrap();
        assert_eq!(p.input, Some(10.0));
        assert_eq!(p.output, Some(50.0));
        assert_eq!(p.cache_read, Some(1.0));
        assert_eq!(p.cache_write, Some(12.5));
    }

    #[test]
    fn zero_price_free_variant_is_kept_as_data() {
        let row = json!({
            "id": "meta-llama/llama-x:free",
            "pricing": { "prompt": "0", "completion": "0" },
            "supported_parameters": ["tools"]
        });
        let p = model_from_row(&row).unwrap().pricing.unwrap();
        assert_eq!(p.input, Some(0.0));
        assert_eq!(p.output, Some(0.0));
    }

    #[test]
    fn sparse_row_falls_back_conservatively() {
        let m = model_from_row(&json!({ "id": "vendor/mystery" })).unwrap();
        assert_eq!(m.context_window, FALLBACK_CONTEXT_WINDOW);
        assert_eq!(m.max_output_tokens, FALLBACK_MAX_OUTPUT);
        assert_eq!(m.display_name, None);
        assert_eq!(m.supports_tools, Some(false));
        assert_eq!(m.supports_thinking, Some(false));
        assert_eq!(m.supports_xhigh, None);
        assert_eq!(m.reasoning_efforts, None);
        assert_eq!(m.supports_cache, None);
        assert_eq!(m.pricing, None);
    }

    #[test]
    fn output_ceiling_never_exceeds_the_context_window() {
        let row = json!({
            "id": "vendor/tiny",
            "context_length": 4096,
            "top_provider": { "max_completion_tokens": 128_000 }
        });
        assert_eq!(model_from_row(&row).unwrap().max_output_tokens, 4096);
    }

    #[test]
    fn rows_without_an_id_are_rejected() {
        assert!(model_from_row(&json!({})).is_none());
        assert!(model_from_row(&json!({ "id": "" })).is_none());
        assert!(model_from_row(&json!({ "id": 7 })).is_none());
    }

    #[test]
    fn malformed_prices_yield_none_not_zero() {
        let row = json!({
            "id": "vendor/m",
            "pricing": { "prompt": "not-a-number", "completion": null }
        });
        assert_eq!(model_from_row(&row).unwrap().pricing, None);
    }
}
