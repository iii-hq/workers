//! Live-metadata mapping for the Copilot slice: one `GET /models` row → one
//! catalog `Model`. The Copilot listing is capability-structured
//! (`capabilities.limits` for windows/ceilings, `capabilities.supports` for
//! feature flags), so live discovery owns the record; there is no pricing —
//! a subscription meters in premium requests, not per-token dollars.
//!
//! Catalog ids are prefixed `copilot/` (`copilot/gpt-5.2`): bare upstream ids
//! collide with the same vendors' ids from the sibling providers. The prefix
//! is stripped again on every upstream call.
use crate::PROVIDER_ID;
use llm_router::types::model::Model;
use serde_json::Value;

/// Catalog-id prefix; also what `upstream_id` strips before the wire.
pub const MODEL_PREFIX: &str = "copilot/";

/// Fallbacks for rows missing limits — conservative, so a wrong guess
/// under-fills a context instead of overflowing it upstream.
const FALLBACK_CONTEXT_WINDOW: u64 = 32_768;
const FALLBACK_MAX_OUTPUT: u64 = 8_192;

/// Copilot model id → catalog id (`gpt-5.2` → `copilot/gpt-5.2`).
pub fn catalog_id(upstream: &str) -> String {
    format!("{MODEL_PREFIX}{upstream}")
}

/// Catalog id → the id the Copilot API expects. Tolerates an already-bare id
/// so a caller passing an unprefixed model still routes.
pub fn upstream_id(catalog: &str) -> &str {
    catalog.strip_prefix(MODEL_PREFIX).unwrap_or(catalog)
}

fn supports(row: &Value, flag: &str) -> bool {
    row.pointer(&format!("/capabilities/supports/{flag}"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn limit(row: &Value, key: &str) -> Option<u64> {
    row.pointer(&format!("/capabilities/limits/{key}"))
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
}

/// One listing row → catalog Model. `None` when the row has no usable id.
/// Admission (chat + tools + picker-enabled) is discovery's concern, not
/// this mapping's.
pub fn model_from_row(row: &Value) -> Option<Model> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;

    let context_window = limit(row, "max_context_window_tokens").unwrap_or(FALLBACK_CONTEXT_WINDOW);
    let max_output_tokens = limit(row, "max_output_tokens")
        .unwrap_or(FALLBACK_MAX_OUTPUT)
        .min(context_window);

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
        input_limit: limit(row, "max_prompt_tokens"),
        // The listing carries no reasoning-effort surface; thinking models
        // stream reasoning implicitly and the router treats absence as
        // unknown rather than unsupported.
        supports_thinking: None,
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: Some(supports(row, "tool_calls")),
        supports_vision: Some(supports(row, "vision")),
        supports_cache: None,
        supports_structured_output: Some(supports(row, "structured_outputs")),
        thinking_budgets: None,
        // Subscription metering (premium requests), not per-token pricing.
        pricing: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn full_row() -> Value {
        json!({
            "id": "gpt-5.2",
            "name": "GPT-5.2",
            "capabilities": {
                "type": "chat",
                "family": "gpt-5.2",
                "limits": {
                    "max_context_window_tokens": 400_000,
                    "max_output_tokens": 64_000,
                    "max_prompt_tokens": 300_000
                },
                "supports": {
                    "tool_calls": true,
                    "streaming": true,
                    "vision": true,
                    "structured_outputs": true
                }
            },
            "model_picker_enabled": true
        })
    }

    #[test]
    fn id_round_trips_through_the_prefix() {
        assert_eq!(catalog_id("gpt-5.2"), "copilot/gpt-5.2");
        assert_eq!(upstream_id("copilot/gpt-5.2"), "gpt-5.2");
        // already-bare ids pass through
        assert_eq!(upstream_id("gpt-5.2"), "gpt-5.2");
    }

    #[test]
    fn full_row_maps_limits_and_capability_flags() {
        let m = model_from_row(&full_row()).unwrap();
        assert_eq!(m.id, "copilot/gpt-5.2");
        assert_eq!(m.provider, "github-copilot");
        assert_eq!(m.display_name.as_deref(), Some("GPT-5.2"));
        assert_eq!(m.context_window, 400_000);
        assert_eq!(m.max_output_tokens, 64_000);
        assert_eq!(m.input_limit, Some(300_000));
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_vision, Some(true));
        assert_eq!(m.supports_structured_output, Some(true));
        assert_eq!(m.supports_thinking, None);
        assert_eq!(m.pricing, None, "subscription metering, no per-token price");
    }

    #[test]
    fn sparse_row_falls_back_conservatively() {
        let m = model_from_row(&json!({ "id": "mystery" })).unwrap();
        assert_eq!(m.context_window, FALLBACK_CONTEXT_WINDOW);
        assert_eq!(m.max_output_tokens, FALLBACK_MAX_OUTPUT);
        assert_eq!(m.supports_tools, Some(false));
        assert_eq!(m.input_limit, None);
    }

    #[test]
    fn output_ceiling_never_exceeds_the_context_window() {
        let row = json!({
            "id": "tiny",
            "capabilities": { "limits": {
                "max_context_window_tokens": 4096,
                "max_output_tokens": 64_000
            } }
        });
        assert_eq!(model_from_row(&row).unwrap().max_output_tokens, 4096);
    }

    #[test]
    fn rows_without_an_id_are_rejected() {
        assert!(model_from_row(&json!({})).is_none());
        assert!(model_from_row(&json!({ "id": "" })).is_none());
    }
}
