//! thinking_level → Chat Completions `reasoning_effort`, per model family.
//! Each ladder branch traces to a documented 400 from the API — a wrong
//! effort string fails the whole request.
use llm_router::types::model::{Model, ThinkingLevel};
use serde_json::Value;

/// Full effort vocabulary in ascending order (superset across families).
const EFFORT_ORDER: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// id-pattern fallback for models the catalog doesn't know.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    let id = model.to_ascii_lowercase();
    id.starts_with("gpt-5") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4")
}

/// Efforts the model family accepts; empty = don't send the param.
fn supported_efforts(model: &str) -> &'static [&'static str] {
    let id = model.to_ascii_lowercase();
    if !(id.starts_with("gpt-5")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4"))
    {
        return &[];
    }
    // The o1 family (o1, o1-mini, o1-preview, o1-pro) rejects reasoning_effort
    // on Chat Completions with a 400 — even though catalogs flag it as a
    // reasoning model. Omit the param entirely.
    if id.starts_with("o1") {
        return &[];
    }
    // Chat-tuned variants only support the fixed default; omit the param.
    if id.contains("chat") {
        return &[];
    }
    // gpt-5-pro / gpt-5.x-pro: high only.
    if id.contains("pro") {
        return &["high"];
    }
    // gpt-5.1: none/low/medium/high; gpt-5.2+ adds xhigh.
    if let Some(minor) = gpt5_minor(&id) {
        return if minor >= 2 {
            &["none", "low", "medium", "high", "xhigh"]
        } else {
            &["none", "low", "medium", "high"]
        };
    }
    // gpt-5 base family (mini/nano/codex).
    if id.starts_with("gpt-5") {
        return &["minimal", "low", "medium", "high"];
    }
    // o-series (o3/o4).
    &["low", "medium", "high"]
}

/// `gpt-5.<minor>…` → minor version number.
fn gpt5_minor(id: &str) -> Option<u32> {
    let rest = id.strip_prefix("gpt-5.")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn level_str(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
    }
}

/// Effort for a reasoning model: the requested level when the family
/// supports it, else the nearest supported effort below (then above).
/// `None` when the family takes no effort param or no level was requested.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>, model: &str) -> Option<&'static str> {
    let ladder = supported_efforts(model);
    if ladder.is_empty() {
        return None;
    }
    let want = level_str(level?);
    if ladder.contains(&want) {
        return Some(want);
    }
    let want_idx = EFFORT_ORDER.iter().position(|e| *e == want)?;
    if let Some(&c) = EFFORT_ORDER[..want_idx]
        .iter()
        .rev()
        .find(|&&c| ladder.contains(&c))
    {
        return Some(c);
    }
    EFFORT_ORDER[want_idx + 1..]
        .iter()
        .find(|&&c| ladder.contains(&c))
        .copied()
}

/// Read and validate an exact provider-native effort from this provider's
/// `provider_options` slice. When catalog metadata is available it is the
/// authority; an absent catalog keeps the option permissive for cold starts.
pub fn native_reasoning_effort(
    provider_options: Option<&Value>,
    model_meta: Option<&Model>,
) -> Result<Option<String>, String> {
    let Some(value) = provider_options.and_then(|options| options.get("reasoning_effort")) else {
        return Ok(None);
    };
    let Some(effort) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err("reasoning_effort must be a non-empty string".into());
    };
    if let Some(supported) = model_meta.and_then(|model| model.reasoning_efforts.as_ref()) {
        if !supported.iter().any(|option| option.effort == effort) {
            let allowed = supported
                .iter()
                .map(|option| option.effort.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "reasoning effort {effort:?} is not supported by this model; choose one of: {allowed}"
            ));
        }
    }
    Ok(Some(effort.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_flag_wins_over_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("gpt-5.2", Some(false)));
        assert!(is_reasoning_model("gpt-5.2", None));
        assert!(is_reasoning_model("o4-mini", None));
        assert!(!is_reasoning_model("gpt-4o", None));
    }

    #[test]
    fn exact_level_passes_through_per_family() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "gpt-5.2"),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh), "gpt-5.2"),
            Some("xhigh")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "o3-mini"),
            Some("medium")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal), "gpt-5-mini"),
            Some("minimal")
        );
    }

    #[test]
    fn unsupported_level_degrades_to_nearest_below_then_above() {
        // gpt-5.1 has no xhigh → nearest below is high
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh), "gpt-5.1"),
            Some("high")
        );
        // gpt-5.1 has no minimal → below is none
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal), "gpt-5.1"),
            Some("none")
        );
        // o3 has no minimal and no none below → above is low
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal), "o3"),
            Some("low")
        );
        // pro: everything lands on high
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Low), "gpt-5-pro"),
            Some("high")
        );
    }

    #[test]
    fn families_that_reject_the_param_get_none() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "o1-preview"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "gpt-5-chat-latest"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "gpt-4o"),
            None
        );
    }

    #[test]
    fn absent_level_omits_the_param() {
        assert_eq!(reasoning_effort_for(None, "gpt-5.2"), None);
    }

    #[test]
    fn native_effort_uses_model_specific_catalog() {
        use llm_router::types::model::ReasoningEffort;

        let model = Model {
            id: "codex/gpt-5.6-sol".into(),
            provider: "openai-codex".into(),
            display_name: None,
            context_window: 128_000,
            max_output_tokens: 128_000,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: Some(true),
            reasoning_efforts: Some(vec![ReasoningEffort {
                effort: "ultra".into(),
                description: Some("Maximum reasoning with delegation".into()),
            }]),
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: Some(true),
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        };
        assert_eq!(
            native_reasoning_effort(
                Some(&serde_json::json!({ "reasoning_effort": "ultra" })),
                Some(&model),
            ),
            Ok(Some("ultra".into()))
        );
        assert!(native_reasoning_effort(
            Some(&serde_json::json!({ "reasoning_effort": "minimal" })),
            Some(&model),
        )
        .unwrap_err()
        .contains("choose one of: ultra"));
    }
}
