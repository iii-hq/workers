//! thinking_level → DeepSeek's two reasoning knobs (api-docs.deepseek.com
//! guides/thinking_mode, 2026-08):
//!   - `thinking: { "type": "enabled" }` — sent only when a level was
//!     requested. With no level the param is OMITTED so every model runs its
//!     own default: the V4 family reasons at high effort — the chain of
//!     thought streams into the console on an unconfigured chat — while a
//!     legacy non-thinking alias (deepseek-chat) keeps the semantics its
//!     name encodes. `"disabled"` is deliberately never sent: the router
//!     has no off level to express, and a synthetic off-by-default blanked
//!     the console's thinking pane on every chat that never picked a level.
//!   - `reasoning_effort` — a top-level parameter (not nested in `thinking`),
//!     taking `low` | `high` | `max`; also omitted with no level (API
//!     default: high).
use llm_router::types::model::ThinkingLevel;

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// id-pattern fallback for models the catalog doesn't know. Every DeepSeek
/// model from V3.1 on is hybrid-reasoning. Non-DeepSeek ids (custom
/// OpenAI-compatible endpoints behind an `api_url` override) get no
/// DeepSeek-specific params at all.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    model.to_ascii_lowercase().starts_with("deepseek")
}

/// The `thinking.type` body value: `enabled` when a level was requested on a
/// reasoning model; `None` (param omitted → the model's own default, which
/// is thinking-on for the V4 family) otherwise.
pub fn thinking_type(level: Option<ThinkingLevel>, reasoning: bool) -> Option<&'static str> {
    (reasoning && level.is_some()).then_some("enabled")
}

/// The router's five levels collapse onto DeepSeek's three-wide ladder.
/// `max` is reserved for `xhigh` — it is the top of the vocabulary on both
/// models — and `high` is DeepSeek's own default, so `medium` rounds up to it
/// rather than down to `low`. DeepSeek coerces per model server-side
/// (`low` → `low` on flash, `high` on pro), so no per-model gating is needed.
/// `None` when the model does not reason or no level was requested.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>) -> Option<&'static str> {
    Some(match level? {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium | ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "max",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_flag_wins_over_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("deepseek-v4-pro", Some(false)));
        assert!(is_reasoning_model("deepseek-v4-pro", None));
        assert!(is_reasoning_model("deepseek-v5-unreleased", None));
        assert!(!is_reasoning_model("qwen2.5-coder-7b-instruct", None));
    }

    #[test]
    fn thinking_param_rides_only_with_an_explicit_level() {
        assert_eq!(
            thinking_type(Some(ThinkingLevel::High), true),
            Some("enabled")
        );
        // No level → param omitted → the model's own default applies (V4:
        // enabled at high effort), so an unconfigured console chat still
        // streams its chain of thought. `"disabled"` is never produced.
        assert_eq!(thinking_type(None, true), None);
        // non-reasoning models never see the DeepSeek-specific param
        assert_eq!(thinking_type(Some(ThinkingLevel::High), false), None);
        assert_eq!(thinking_type(None, false), None);
    }

    #[test]
    fn five_levels_collapse_onto_low_high_max() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal)),
            Some("low")
        );
        assert_eq!(reasoning_effort_for(Some(ThinkingLevel::Low)), Some("low"));
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium)),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High)),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh)),
            Some("max")
        );
    }

    #[test]
    fn absent_level_omits_the_param() {
        assert_eq!(reasoning_effort_for(None), None);
    }

    #[test]
    fn every_effort_is_a_value_the_api_documents() {
        for level in [
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::Xhigh,
        ] {
            let effort = reasoning_effort_for(Some(level)).unwrap();
            assert!(
                ["low", "high", "max"].contains(&effort),
                "{level:?} → {effort:?} is outside DeepSeek's vocabulary"
            );
        }
    }
}
