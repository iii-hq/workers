//! thinking_level → reasoning_effort for OpenCode Go Chat Completions.
//! Simplified from provider-openai: no family-specific ladders, no
//! degradation logic.
use llm_router::types::model::ThinkingLevel;

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// id-pattern fallback for models the catalog doesn't know.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    let id = model.to_ascii_lowercase();
    id.starts_with("deepseek-") || id.starts_with("kimi-k2.7-")
}

/// Efforts the model family accepts; empty = don't send the param.
fn supported_efforts(model: &str) -> &'static [&'static str] {
    let _id = model.to_ascii_lowercase();
    if is_reasoning_model(model, None) {
        &["low", "medium", "high"]
    } else {
        &[]
    }
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
/// supports it, otherwise None.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>, model: &str) -> Option<&'static str> {
    let ladder = supported_efforts(model);
    if ladder.is_empty() {
        return None;
    }
    let want = level_str(level?);
    if ladder.contains(&want) {
        return Some(want);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_flag_wins_over_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("deepseek-v4-flash", Some(false)));
        assert!(is_reasoning_model("deepseek-v4-flash", None));
        assert!(is_reasoning_model("kimi-k2.7-code", None));
        assert!(!is_reasoning_model("qwen2.5-coder-7b-instruct", None));
    }

    #[test]
    fn exact_level_passes_through() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "deepseek-v4-flash"),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "kimi-k2.7-code"),
            Some("medium")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Low), "deepseek-v4-flash"),
            Some("low")
        );
    }

    #[test]
    fn unsupported_model_gets_none() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "qwen2.5-coder-7b-instruct"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "gpt-4o"),
            None
        );
    }

    #[test]
    fn absent_level_omits_the_param() {
        assert_eq!(reasoning_effort_for(None, "deepseek-v4-flash"), None);
    }
}
