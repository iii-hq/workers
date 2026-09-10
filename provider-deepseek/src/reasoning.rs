//! thinking_level → DeepSeek's two reasoning knobs (api-docs.deepseek.com
//! guides/thinking_mode, 2026-09):
//!   - `thinking: { "type": "enabled" | "disabled" }` — thinking is ON by
//!     default on every V4 model. `"enabled"` rides only when a level was
//!     requested; with no level the param is OMITTED so each model runs its
//!     own default (V4: high effort — the chain of thought streams into the
//!     console on an unconfigured chat) while a legacy non-thinking alias
//!     (deepseek-chat) keeps the semantics its name encodes. `"disabled"` is
//!     the one DeepSeek knob the router's five-level ladder cannot express:
//!     it is sent only when the caller asks for it through this provider's
//!     `provider_options.thinking` slice, never synthesised — an off-by-default
//!     blanked the console's thinking pane on every chat that never picked a
//!     level.
//!   - `reasoning_effort` — a top-level parameter (not nested in `thinking`)
//!     taking `low` | `high` | `max`; omitted with no level (API default:
//!     high) and omitted when thinking is disabled (nothing to grade).
//!     DeepSeek also accepts the router's own words and coerces server-side
//!     (`minimal`→low, `medium`→high, `xhigh`→**high**), so the ladder is
//!     mapped here, where `xhigh` gets `max`.
use llm_router::types::model::ThinkingLevel;
use serde_json::Value;

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

/// `provider_options.thinking`: the native on/off switch. `"disabled"` turns
/// the chain of thought off entirely; `"enabled"` is accepted for symmetry.
/// Anything else is a hard error — a misspelt off switch must not silently
/// run at full effort (the codex provider takes the same line on its native
/// `reasoning_effort`).
pub fn native_thinking(provider_options: Option<&Value>) -> Result<Option<&'static str>, String> {
    let Some(value) = provider_options.and_then(|options| options.get("thinking")) else {
        return Ok(None);
    };
    match value.as_str().map(str::trim) {
        Some("disabled") => Ok(Some("disabled")),
        Some("enabled") => Ok(Some("enabled")),
        _ => Err(format!(
            "provider_options.thinking must be \"enabled\" or \"disabled\", got {value}"
        )),
    }
}

/// The `thinking.type` body value. A native switch wins outright; otherwise
/// `enabled` when a level was requested on a reasoning model; `None` (param
/// omitted → the model's own default, thinking-on for the V4 family)
/// otherwise. Non-reasoning models never see the DeepSeek-specific param.
pub fn thinking_type(
    level: Option<ThinkingLevel>,
    reasoning: bool,
    native: Option<&'static str>,
) -> Option<&'static str> {
    if !reasoning {
        return None;
    }
    native.or_else(|| level.is_some().then_some("enabled"))
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

/// Both wire params, resolved together so the one rule that ties them —
/// disabled thinking carries no effort — lives in one place.
#[derive(Debug, PartialEq, Eq)]
pub struct ReasoningParams {
    pub thinking: Option<&'static str>,
    pub reasoning_effort: Option<&'static str>,
}

pub fn resolve(
    level: Option<ThinkingLevel>,
    reasoning: bool,
    native: Option<&'static str>,
) -> ReasoningParams {
    let thinking = thinking_type(level, reasoning, native);
    let reasoning_effort = if reasoning && thinking != Some("disabled") {
        reasoning_effort_for(level)
    } else {
        None
    };
    ReasoningParams {
        thinking,
        reasoning_effort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            thinking_type(Some(ThinkingLevel::High), true, None),
            Some("enabled")
        );
        // No level → param omitted → the model's own default applies (V4:
        // enabled at high effort), so an unconfigured console chat still
        // streams its chain of thought. `"disabled"` is never synthesised.
        assert_eq!(thinking_type(None, true, None), None);
        // non-reasoning models never see the DeepSeek-specific param
        assert_eq!(thinking_type(Some(ThinkingLevel::High), false, None), None);
        assert_eq!(thinking_type(None, false, None), None);
    }

    #[test]
    fn native_switch_is_exact_or_an_error() {
        assert_eq!(native_thinking(None), Ok(None));
        assert_eq!(native_thinking(Some(&json!({}))), Ok(None));
        assert_eq!(
            native_thinking(Some(&json!({ "thinking": "disabled" }))),
            Ok(Some("disabled"))
        );
        assert_eq!(
            native_thinking(Some(&json!({ "thinking": " enabled " }))),
            Ok(Some("enabled"))
        );
        // A typo on the off switch must fail the turn, not run at full effort.
        for bad in [
            json!({ "thinking": "off" }),
            json!({ "thinking": false }),
            json!({ "thinking": "" }),
        ] {
            let err = native_thinking(Some(&bad)).unwrap_err();
            assert!(err.contains("\"enabled\" or \"disabled\""), "{err}");
        }
    }

    #[test]
    fn native_off_switch_wins_and_drops_the_effort() {
        let off = ReasoningParams {
            thinking: Some("disabled"),
            reasoning_effort: None,
        };
        // off beats a requested level (the caller is warned upstream)
        assert_eq!(
            resolve(Some(ThinkingLevel::Xhigh), true, Some("disabled")),
            off
        );
        assert_eq!(resolve(None, true, Some("disabled")), off);
        // enabled without a level: thinking on, effort left to the API default
        assert_eq!(
            resolve(None, true, Some("enabled")),
            ReasoningParams {
                thinking: Some("enabled"),
                reasoning_effort: None,
            }
        );
        // the plain ladder is unchanged
        assert_eq!(
            resolve(Some(ThinkingLevel::High), true, None),
            ReasoningParams {
                thinking: Some("enabled"),
                reasoning_effort: Some("high"),
            }
        );
        assert_eq!(
            resolve(None, true, None),
            ReasoningParams {
                thinking: None,
                reasoning_effort: None,
            }
        );
        // a non-reasoning model gets neither param, switch or not
        assert_eq!(
            resolve(Some(ThinkingLevel::High), false, Some("disabled")),
            ReasoningParams {
                thinking: None,
                reasoning_effort: None,
            }
        );
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
