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
    pub mode: &'static str, // "adaptive", or "between_tools" for off
    /// Adaptive models default `display` to "omitted" (empty thinking text
    /// on the wire); "summarized" restores readable reasoning for the live
    /// thought panes. The raw chain of thought is never exposed either way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<&'static str>,
}

pub const ADAPTIVE: ThinkingConfig = ThinkingConfig {
    mode: "adaptive",
    display: Some("summarized"),
};

/// Up-front thinking off. The current generation has thinking on by default
/// and rejects `{type: "disabled"}`; Sonnet 5.5 documents `between_tools`
/// as its off switch (build-with-claude/thinking).
pub const BETWEEN_TOOLS: ThinkingConfig = ThinkingConfig {
    mode: "between_tools",
    display: None,
};

/// Thinking off on Sonnet 5 and Opus 5, which still accept `{type:
/// "disabled"}` (Opus 5 only at effort high or below, which the server
/// default satisfies). The 5.5 generation and later reject it.
pub const DISABLED: ThinkingConfig = ThinkingConfig {
    mode: "disabled",
    display: None,
};

/// The `thinking` config that turns reasoning off on `model`, when the docs
/// (thinking-troubleshooting#supported-models) name one.
fn off_config(id: &str) -> Option<ThinkingConfig> {
    if id.contains("sonnet-5-5") {
        Some(BETWEEN_TOOLS)
    } else if id.contains("opus-5-5") || id.contains("fable") || id.contains("mythos") {
        None
    } else if id.contains("sonnet-5") || id.contains("opus-5") {
        Some(DISABLED)
    } else {
        None
    }
}

/// Whether `thinking_level: off` can be honoured on `model` (base id, no
/// provider prefix): `Some(true)` where `off_config` names a switch (Sonnet
/// 5.5 `between_tools`, Sonnet 5 and Opus 5 `disabled`), `Some(false)` for
/// the always-on models (Opus 5.5, Fable, Mythos), `None` where the docs
/// read today do not say.
pub fn supports_off(model: &str) -> Option<bool> {
    let id = model.to_ascii_lowercase();
    if off_config(&id).is_some() {
        Some(true)
    } else if id.contains("opus-5-5") || id.contains("fable") || id.contains("mythos") {
        Some(false)
    } else {
        None
    }
}

/// thinking_level → `output_config.effort`. Minimal has no effort
/// equivalent; low is the closest depth.
fn effort_for(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off | ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
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
    if level == ThinkingLevel::Off {
        return match model.and_then(|m| off_config(&m.id.to_ascii_lowercase())) {
            Some(config) => ThinkingBuild {
                config: Some(config),
                effort: None,
                warnings,
            },
            // Thinking cannot be turned off here (or the docs do not say it
            // can): the lowest effort is the closest honest answer.
            _ => {
                warnings.push(
                    "thinking_level off degraded to low: this model cannot turn thinking off"
                        .to_string(),
                );
                ThinkingBuild {
                    config: Some(ADAPTIVE),
                    effort: Some("low"),
                    warnings,
                }
            }
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

    #[test]
    fn off_is_between_tools_on_sonnet_5_5_and_degrades_to_low_elsewhere() {
        let mut sonnet = model(Some(true), Some(true));
        sonnet.id = "claude-code/claude-sonnet-5-5".into();
        let built = build_thinking_config(Some(ThinkingLevel::Off), Some(&sonnet));
        assert_eq!(built.config, Some(BETWEEN_TOOLS));
        assert_eq!(built.effort, None);
        assert!(built.warnings.is_empty());
        assert_eq!(
            serde_json::to_value(BETWEEN_TOOLS).unwrap(),
            serde_json::json!({ "type": "between_tools" }),
            "no display field rides with between_tools"
        );

        let mut opus = model(Some(true), Some(true));
        opus.id = "claude-code/claude-opus-5-5".into();
        let built = build_thinking_config(Some(ThinkingLevel::Off), Some(&opus));
        assert_eq!(built.config, Some(ADAPTIVE));
        assert_eq!(built.effort, Some("low"));
        assert_eq!(built.warnings.len(), 1, "{:?}", built.warnings);

        assert_eq!(supports_off("claude-sonnet-5-5"), Some(true));
        assert_eq!(supports_off("claude-fable-5-1"), Some(false));
        assert_eq!(supports_off("claude-sonnet-4-6"), None);
    }

    #[test]
    fn off_is_disabled_on_sonnet_5_and_opus_5() {
        // thinking-troubleshooting#supported-models: Sonnet 5 and Opus 5 accept
        // `{type: "disabled"}` (Opus 5 at effort high or below, the default).
        for id in [
            "claude-code/claude-sonnet-5",
            "claude-code/claude-opus-5",
            "claude-code/claude-opus-5-20260301",
        ] {
            let mut m = model(Some(true), Some(true));
            m.id = id.into();
            let built = build_thinking_config(Some(ThinkingLevel::Off), Some(&m));
            assert_eq!(built.config, Some(DISABLED), "{id}");
            assert_eq!(built.effort, None, "{id}");
            assert!(built.warnings.is_empty(), "{id}: {:?}", built.warnings);
        }
        assert_eq!(
            serde_json::to_value(DISABLED).unwrap(),
            serde_json::json!({ "type": "disabled" })
        );
        assert_eq!(supports_off("claude-opus-5"), Some(true));
        assert_eq!(supports_off("claude-sonnet-5"), Some(true));
        assert_eq!(supports_off("claude-opus-5-5-20260901"), Some(false));
        assert_eq!(supports_off("claude-opus-4-8"), None);
    }

    fn model(thinking: Option<bool>, xhigh: Option<bool>) -> Model {
        Model {
            id: "claude-test".into(),
            provider: "claude-code".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: thinking,
            supports_xhigh: xhigh,
            supports_thinking_off: None,
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: Some(true),
            supports_structured_output: Some(false),
            thinking_budgets: None,
            pricing: None,
            speech: None,
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
