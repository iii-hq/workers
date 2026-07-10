//! thinking_level → Messages API adaptive `thinking` field plus
//! `output_config.effort`. Anthropic's adaptive generation (Opus 4.7+,
//! Fable/Mythos 5) hard-rejects the legacy `{type: "enabled", budget_tokens}`
//! shape, and every model this provider serves with thinking enabled
//! (Sonnet 4.6 onward) accepts adaptive — so adaptive is the only path.
use llm_router::types::model::{Model, ThinkingLevel};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub mode: &'static str, // always "adaptive"
    /// Adaptive models default `display` to "omitted" (empty thinking text
    /// on the wire); "summarized" restores readable reasoning for the live
    /// thought panes. The raw chain of thought is never exposed either way.
    pub display: &'static str,
}

pub const ADAPTIVE: ThinkingConfig = ThinkingConfig {
    mode: "adaptive",
    display: "summarized",
};

/// thinking_level → `output_config.effort`. Minimal has no effort
/// equivalent; low is the closest depth.
fn effort_for(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
    }
}

pub struct ThinkingBuild {
    pub config: Option<ThinkingConfig>,
    /// `output_config.effort`; present exactly when `config` is.
    pub effort: Option<&'static str>,
    pub warnings: Vec<String>,
}

pub fn build_thinking_config(level: Option<ThinkingLevel>, model: Option<&Model>) -> ThinkingBuild {
    let mut warnings = Vec::new();
    let Some(level) = level else {
        // Parity with xai reasoning models, which surface reasoning by default:
        // with no explicit level, still request adaptive thinking on models that
        // definitely support it (server default effort, so no output_config).
        // Gate on Some(true), not the permissive path — an implicit default must
        // never 400 on a non-thinking or unknown model.
        let config = (model.and_then(|m| m.supports_thinking) == Some(true)).then_some(ADAPTIVE);
        return ThinkingBuild {
            config,
            effort: None,
            warnings,
        };
    };
    // An explicit `supports_thinking: false` would 400; unknown stays permissive.
    if model.and_then(|m| m.supports_thinking) == Some(false) {
        warnings.push(format!(
            "thinking_level {level:?} dropped: model does not support thinking"
        ));
        return ThinkingBuild {
            config: None,
            effort: None,
            warnings,
        };
    }
    let effective =
        if level == ThinkingLevel::Xhigh && model.and_then(|m| m.supports_xhigh) == Some(false) {
            warnings.push(format!(
                "thinking_level {level:?} degraded to High: model does not support xhigh"
            ));
            ThinkingLevel::High
        } else {
            level
        };
    ThinkingBuild {
        config: Some(ADAPTIVE),
        effort: Some(effort_for(effective)),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(thinking: Option<bool>, xhigh: Option<bool>) -> Model {
        Model {
            id: "claude-test".into(),
            provider: "anthropic".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: thinking,
            supports_xhigh: xhigh,
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: Some(true),
            supports_structured_output: Some(false),
            thinking_budgets: None,
            pricing: None,
        }
    }

    #[test]
    fn absent_level_defaults_on_for_thinking_models() {
        // Parity with xai: no explicit level still surfaces reasoning on a model
        // that supports thinking, at the server's default effort (no output_config).
        let built = build_thinking_config(None, Some(&model(Some(true), Some(true))));
        assert_eq!(built.config, Some(ADAPTIVE));
        assert_eq!(built.effort, None);
        assert!(built.warnings.is_empty());
    }

    #[test]
    fn absent_level_stays_off_without_thinking_support() {
        // No implicit default when support is unknown or explicitly false, so the
        // default can never 400 a non-thinking model.
        for m in [
            None,
            Some(model(None, None)),
            Some(model(Some(false), None)),
        ] {
            let built = build_thinking_config(None, m.as_ref());
            assert_eq!(built.config, None);
            assert_eq!(built.effort, None);
            assert!(built.warnings.is_empty());
        }
    }

    #[test]
    fn levels_map_to_adaptive_with_effort() {
        let m = model(Some(true), Some(true));
        for (level, effort) in [
            (ThinkingLevel::Minimal, "low"),
            (ThinkingLevel::Low, "low"),
            (ThinkingLevel::Medium, "medium"),
            (ThinkingLevel::High, "high"),
            (ThinkingLevel::Xhigh, "xhigh"),
        ] {
            let built = build_thinking_config(Some(level), Some(&m));
            assert_eq!(built.config, Some(ADAPTIVE));
            assert_eq!(built.effort, Some(effort));
            assert!(built.warnings.is_empty());
        }
    }

    #[test]
    fn explicit_no_thinking_support_drops() {
        let built =
            build_thinking_config(Some(ThinkingLevel::High), Some(&model(Some(false), None)));
        assert_eq!(built.config, None);
        assert_eq!(built.effort, None);
        assert!(built.warnings.iter().any(|w| w.contains("dropped")));
    }

    #[test]
    fn xhigh_degrades_to_high_effort_when_unsupported() {
        let built = build_thinking_config(
            Some(ThinkingLevel::Xhigh),
            Some(&model(Some(true), Some(false))),
        );
        assert_eq!(built.config, Some(ADAPTIVE));
        assert_eq!(built.effort, Some("high"));
        assert!(built.warnings.iter().any(|w| w.contains("degraded")));
    }

    #[test]
    fn unknown_model_stays_permissive() {
        // No catalog record: adaptive needs no budget arithmetic, so
        // thinking flows through instead of dropping.
        let built = build_thinking_config(Some(ThinkingLevel::High), None);
        assert_eq!(built.config, Some(ADAPTIVE));
        assert_eq!(built.effort, Some("high"));
    }

    #[test]
    fn serializes_as_adaptive_with_summarized_display() {
        let v = serde_json::to_value(ADAPTIVE).unwrap();
        assert_eq!(v["type"], "adaptive");
        assert_eq!(v["display"], "summarized");
        assert!(v.get("budget_tokens").is_none());
    }
}
