//! thinking_level → Groq's one reasoning knob.
//!
//! Groq has no `thinking` object: reasoning is requested with the top-level
//! `reasoning_effort` parameter alone, taking `none` | `default` | `low` |
//! `medium` | `high` (console.groq.com/docs/api-reference, 2026-08). With no
//! level the parameter is omitted so each model runs its own default, and
//! `none` is never synthesized — the router has no off level to express, and
//! sending one would blank the console's thinking pane on every chat that
//! simply never picked a level.
//!
//! Which models reason is not a provider-wide fact here, unlike at a
//! single-family provider: the GPT-OSS models reason and the Llama models do
//! not, so the catalog decides per model.
use llm_router::types::model::ThinkingLevel;

/// Whether this model reasons: the catalog's `supports_thinking` flag wins,
/// with an id-pattern fallback for a model the catalog has not caught up with.
///
/// Groq hosts other people's models, so the fallback keys on family rather
/// than on the provider name — there is no such thing as a "Groq model". A
/// model behind an `api_url` override that matches nothing gets no reasoning
/// parameters at all, which is the safe default: an unknown model rejecting an
/// unexpected parameter would fail the whole turn.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    let id = model.to_ascii_lowercase();
    id.contains("gpt-oss") || id.contains("qwen")
}

/// The router's five levels onto Groq's four requestable efforts.
///
/// `xhigh` has nowhere above `high` to go, so it saturates there rather than
/// inventing a tier the API would reject. `minimal` maps to `low` rather than
/// to `none`: the caller asked for some reasoning, and `none` would turn it
/// off entirely.
///
/// `None` when no level was requested — the parameter is then omitted and the
/// model's own default applies.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>) -> Option<&'static str> {
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
    fn the_catalog_flag_wins_over_the_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("openai/gpt-oss-120b", Some(false)));
    }

    #[test]
    fn the_fallback_keys_on_family_because_groq_hosts_other_peoples_models() {
        assert!(is_reasoning_model("openai/gpt-oss-20b", None));
        assert!(is_reasoning_model("qwen3-32b", None));
        // Llama does not reason, and neither does a model nothing recognizes.
        assert!(!is_reasoning_model("llama-3.3-70b-versatile", None));
        assert!(!is_reasoning_model("some-model-shipped-tomorrow", None));
    }

    #[test]
    fn five_levels_land_on_efforts_the_api_documents() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal)),
            Some("low")
        );
        assert_eq!(reasoning_effort_for(Some(ThinkingLevel::Low)), Some("low"));
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Medium)),
            Some("medium")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High)),
            Some("high")
        );
    }

    #[test]
    fn xhigh_saturates_rather_than_inventing_a_tier() {
        // Groq's ladder stops at high; sending anything above it would be
        // rejected, and silently dropping the request is worse than capping.
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh)),
            Some("high")
        );
    }

    #[test]
    fn absent_level_omits_the_param() {
        assert_eq!(reasoning_effort_for(None), None);
    }

    #[test]
    fn every_effort_is_a_value_the_api_accepts() {
        for level in [
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::Xhigh,
        ] {
            let effort = reasoning_effort_for(Some(level)).unwrap();
            assert!(
                ["low", "medium", "high"].contains(&effort),
                "{level:?} → {effort:?} is outside Groq's vocabulary"
            );
        }
    }
}
