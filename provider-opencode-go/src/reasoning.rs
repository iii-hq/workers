//! thinking_level → the upstream `reasoning_effort` string, per model, from
//! the curated metadata table.
//!
//! Each model's allowed effort list traces to models.dev (`opencode-go`
//! provider, fetched 2026-08-03) — a wrong effort string fails the whole
//! request. Models that reason without published effort levels (toggle-only
//! or undocumented) take no `reasoning_effort` param at all; simplified from
//! provider-openai: no degradation ladder, an unsupported level just omits
//! the param and warns.
use crate::curated;
use llm_router::types::model::ThinkingLevel;

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// the curated table decides for known models; anything else is not a
/// reasoning model (conservative, like the catalog defaults).
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    curated::meta(model).is_some_and(|m| m.reasoning)
}

/// Efforts the model accepts on the wire; empty = don't send the param.
fn supported_efforts(model: &str) -> &'static [&'static str] {
    curated::meta(model)
        .map(|m| m.reasoning_efforts)
        .unwrap_or(&[])
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

/// Effort for a reasoning model: the requested level when the model accepts
/// it, otherwise None (param omitted, request proceeds at the API's default
/// effort).
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
    fn catalog_flag_wins_over_curated_lookup() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("deepseek-v4-flash", Some(false)));
        // Curated ids resolve as reasoning models without a catalog flag.
        assert!(is_reasoning_model("deepseek-v4-flash", None));
        assert!(is_reasoning_model("kimi-k2.7-code", None));
        assert!(is_reasoning_model("hy3", None));
        // Unknown ids are not reasoning models.
        assert!(!is_reasoning_model("qwen2.5-coder-7b-instruct", None));
        assert!(!is_reasoning_model("gpt-4o", None));
    }

    #[test]
    fn exact_level_passes_through_per_model() {
        // grok-4.5 accepts the full low/medium/high ladder.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "grok-4.5"),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "grok-4.5"),
            Some("medium")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Low), "grok-4.5"),
            Some("low")
        );
        // deepseek-v4-flash keeps sending an effort when thinking is
        // requested (high is its lowest accepted level).
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "deepseek-v4-flash"),
            Some("high")
        );
        // hy3 accepts low and high.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Low), "hy3"),
            Some("low")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "hy3"),
            Some("high")
        );
    }

    #[test]
    fn unsupported_level_omits_the_param() {
        // deepseek-v4-flash: only high/max — medium is not accepted.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "deepseek-v4-flash"),
            None
        );
        // glm-5.2: only high/max.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "glm-5.2"),
            None
        );
        // kimi-k3: only max — no ThinkingLevel maps to it.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "kimi-k3"),
            None
        );
        // hy3: no medium.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium), "hy3"),
            None
        );
        // Toggle-only and effort-less reasoning families take no param.
        for model in [
            "minimax-m3",
            "qwen3.7-max",
            "qwen3.7-plus",
            "qwen3.6-plus",
            "glm-5.1",
            "kimi-k2.6",
            "kimi-k2.7-code",
            "mimo-v2.5",
            "mimo-v2.5-pro",
            "minimax-m2.7",
        ] {
            assert_eq!(
                reasoning_effort_for(Some(ThinkingLevel::High), model),
                None,
                "{model} should omit reasoning_effort"
            );
        }
        // Unknown models take no param.
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
        assert_eq!(reasoning_effort_for(None, "grok-4.5"), None);
    }
}
