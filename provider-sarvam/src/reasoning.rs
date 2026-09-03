//! thinking_level → Sarvam's one reasoning knob. The Sarvam chat models
//! reason by default and take `reasoning_effort: low | medium | high`
//! (docs.sarvam.ai chat completions); there is no on/off toggle, so an
//! absent level omits the param and the API default applies.
use llm_router::types::model::ThinkingLevel;

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// id-pattern fallback for models the catalog doesn't know. Every Sarvam
/// chat model reasons; other ids (custom OpenAI-compatible endpoints behind
/// an `api_url` override) get no Sarvam-specific params at all.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    model.to_ascii_lowercase().starts_with("sarvam-")
}

/// Effort for a reasoning model: the router's five levels fold onto
/// Sarvam's three. `None` when no level was requested or the model is not a
/// Sarvam chat model.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>, model: &str) -> Option<&'static str> {
    if !model.to_ascii_lowercase().starts_with("sarvam-") {
        return None;
    }
    Some(match level? {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High | ThinkingLevel::Xhigh => "high",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_flag_wins_over_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("sarvam-105b", Some(false)));
        assert!(is_reasoning_model("sarvam-105b", None));
        assert!(is_reasoning_model("sarvam-m", None));
        assert!(!is_reasoning_model("qwen2.5-coder-7b-instruct", None));
    }

    #[test]
    fn five_levels_fold_onto_three_efforts() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal), "sarvam-105b"),
            Some("low")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Low), "sarvam-105b"),
            Some("low")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "sarvam-30b"),
            Some("medium")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "sarvam-m"),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh), "sarvam-105b"),
            Some("high")
        );
    }

    #[test]
    fn absent_level_and_foreign_models_omit_the_param() {
        assert_eq!(reasoning_effort_for(None, "sarvam-105b"), None);
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "llama-3-8b"),
            None
        );
    }
}
