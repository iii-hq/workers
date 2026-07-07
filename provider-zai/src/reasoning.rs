//! thinking_level → GLM's two reasoning knobs (docs.z.ai
//! guides/capabilities/thinking, 2026-07):
//!   - `thinking: { "type": "enabled" | "disabled" }` — every current GLM
//!     chat model (4.5 and newer). The API default is enabled-with-auto-decide,
//!     so the toggle is sent explicitly: silence would buy unrequested
//!     thinking tokens.
//!   - `reasoning_effort` — GLM-5.2 and newer only; older families are not
//!     documented to take the param, so it is omitted for them.
use llm_router::types::model::ThinkingLevel;

/// Reasoning model detection: the catalog's `supports_thinking` flag wins;
/// id-pattern fallback for models the catalog doesn't know. Every current
/// GLM chat model reasons; non-GLM ids (custom OpenAI-compatible endpoints
/// behind an `api_url` override) get no GLM-specific params at all.
pub fn is_reasoning_model(model: &str, catalog_supports_thinking: Option<bool>) -> bool {
    if let Some(flag) = catalog_supports_thinking {
        return flag;
    }
    model.to_ascii_lowercase().starts_with("glm-")
}

/// The `thinking.type` body value: `enabled` when a level was requested,
/// `disabled` otherwise; `None` (param omitted) for non-reasoning models.
pub fn thinking_type(level: Option<ThinkingLevel>, reasoning: bool) -> Option<&'static str> {
    if !reasoning {
        return None;
    }
    Some(if level.is_some() {
        "enabled"
    } else {
        "disabled"
    })
}

/// Only GLM-5.2+ documents `reasoning_effort`.
fn accepts_effort(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("glm-5.2")
}

/// Effort for a reasoning model. Z.AI accepts the full vocabulary
/// (`minimal`/`low`/`medium`/`high`/`xhigh` — plus `none`/`max`, which the
/// router never requests) and coerces server-side, so the level maps 1:1.
/// `None` when the family takes no effort param or no level was requested.
pub fn reasoning_effort_for(level: Option<ThinkingLevel>, model: &str) -> Option<&'static str> {
    if !accepts_effort(model) {
        return None;
    }
    Some(match level? {
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_flag_wins_over_id_pattern() {
        assert!(is_reasoning_model("weird-model", Some(true)));
        assert!(!is_reasoning_model("glm-4.7", Some(false)));
        assert!(is_reasoning_model("glm-4.7", None));
        assert!(is_reasoning_model("glm-5.2", None));
        assert!(!is_reasoning_model("qwen2.5-coder-7b-instruct", None));
    }

    #[test]
    fn thinking_toggle_tracks_the_requested_level() {
        assert_eq!(
            thinking_type(Some(ThinkingLevel::High), true),
            Some("enabled")
        );
        assert_eq!(thinking_type(None, true), Some("disabled"));
        // non-reasoning models never see the GLM-specific param
        assert_eq!(thinking_type(Some(ThinkingLevel::High), false), None);
        assert_eq!(thinking_type(None, false), None);
    }

    #[test]
    fn effort_param_is_glm_52_only_and_maps_one_to_one() {
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Xhigh), "glm-5.2"),
            Some("xhigh")
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::Minimal), "glm-5.2"),
            Some("minimal")
        );
        // earlier families keep the thinking toggle but never the effort param
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "glm-4.7"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "glm-5.1"),
            None
        );
        assert_eq!(
            reasoning_effort_for(Some(ThinkingLevel::High), "glm-5-turbo"),
            None
        );
    }

    #[test]
    fn absent_level_omits_the_param() {
        assert_eq!(reasoning_effort_for(None, "glm-5.2"), None);
    }
}
