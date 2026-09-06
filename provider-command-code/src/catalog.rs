use crate::config::DEFAULT_MAX_TOKENS;
use crate::PROVIDER_ID;
use llm_router::types::model::Model;
use serde_json::Value;

pub const MODEL_PREFIX: &str = "command-code/";

pub fn catalog_id(upstream: &str) -> String {
    format!("{MODEL_PREFIX}{upstream}")
}

pub fn upstream_id(catalog: &str) -> &str {
    catalog.strip_prefix(MODEL_PREFIX).unwrap_or(catalog)
}

pub fn model_from_row(row: &Value) -> Option<Model> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let context_window = row
        .get("context_length")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)?;
    Some(Model {
        id: catalog_id(id),
        provider: PROVIDER_ID.into(),
        display_name: row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(String::from),
        context_window,
        max_output_tokens: DEFAULT_MAX_TOKENS.min(context_window),
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: None,
        supports_vision: None,
        supports_cache: None,
        supports_structured_output: None,
        thinking_budgets: None,
        pricing: None,
        speech: None,
    })
}

pub fn parse_models(value: &Value) -> Vec<Model> {
    value
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(model_from_row).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn live_fields_map_without_fabricating_optional_metadata() {
        let model = model_from_row(&json!({
            "id": "claude-sonnet-4-6",
            "name": "Claude Sonnet 4.6",
            "context_length": 1_000_000
        }))
        .unwrap();
        assert_eq!(model.id, "command-code/claude-sonnet-4-6");
        assert_eq!(model.provider, "command-code");
        assert_eq!(model.display_name.as_deref(), Some("Claude Sonnet 4.6"));
        assert_eq!(model.context_window, 1_000_000);
        assert_eq!(model.max_output_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(model.supports_tools, None);
        assert_eq!(model.supports_thinking, None);
        assert_eq!(model.supports_vision, None);
        assert_eq!(model.supports_structured_output, None);
        assert!(model.pricing.is_none());
    }

    #[test]
    fn rows_missing_reported_limits_are_omitted_not_guessed() {
        assert!(model_from_row(&json!({ "id": "m" })).is_none());
        assert!(model_from_row(&json!({ "context_length": 1000 })).is_none());
        assert!(model_from_row(&json!({ "id": "m", "context_length": 0 })).is_none());
    }

    #[test]
    fn catalog_prefix_round_trips_vendor_slashes() {
        let id = catalog_id("deepseek/deepseek-v4-flash");
        assert_eq!(id, "command-code/deepseek/deepseek-v4-flash");
        assert_eq!(upstream_id(&id), "deepseek/deepseek-v4-flash");
    }
}
